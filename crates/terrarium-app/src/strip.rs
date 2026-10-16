//! The strip: a borderless window along one edge of a monitor, below ordinary windows, showing
//! the composed frame at an integer scale, and the windows beside it.
//!
//! The loop sleeps in `ControlFlow::Wait` and wakes only for OS events, for what other threads
//! forward through the `EventLoopProxy` (hotkeys, menu choices, system notifications, probe
//! answers, the engine's updates, a second launch's request, a folder picked and a lifecycle
//! change done), and at four deadlines: a raise ending, the living strip's next change, which
//! it never asks for sooner than a fifteenth of a second after the last frame (I-10), the feed's
//! next look at the clock, the idle time or `--demo`'s cycle, and an open window's next paint.
//! Nothing is drawn, and nothing asked for, while the strip is covered.
//!
//! On a run with a data directory, the strip shows only while a repository is connected. With
//! none, the connect window opens instead, and closing it ends the app; connecting opens the
//! strip, and disconnecting closes it and opens the connect window again.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::rc::Rc;
use std::thread;
use std::time::{Duration, Instant};

use global_hotkey::GlobalHotKeyEvent;
use muda::MenuId;
use std::sync::Arc;
use terrarium_compose::catalog::Art;
use terrarium_compose::compositor::composite;
use terrarium_compose::frame::{NativeImage, Viewport};
use terrarium_compose::panel::{self, Hover};
use terrarium_compose::salt::InstallSalt;
use terrarium_compose::scale::{SizeStep, StripScale};
use terrarium_compose::tween::Millis;
use terrarium_compose::variants::ArtTable;
use terrarium_compose::{preset, sample};

use terrarium_adapters::binding::RepoBinding;
use terrarium_adapters::ci::{Keychain, NativeKeychain};
use terrarium_engine::{ConnectError, Engine, Start, Tools, Update};
use terrarium_platform::SystemEvent;
use terrarium_platform::geometry::Edge;
use terrarium_store::identifiers::{BUNDLE_ID, KEYCHAIN_ACCOUNT_GITHUB, KEYCHAIN_SERVICE};
use terrarium_store::{DataDir, Store, Terrarium};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::error::{EventLoopError, OsError};
use winit::event::{ElementState, MouseButton, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::{Window, WindowId};

use crate::animate::Living;
use crate::cli::ProbeKind;
use crate::connect::ConnectForm;
use crate::copy;
#[cfg(feature = "dev")]
use crate::debug::Readout;
#[cfg(feature = "dev")]
use crate::dev::Dev;
use crate::egui_host::WindowKind;
#[cfg(feature = "dev")]
use crate::feed::Overrides;
use crate::feed::{Feed, Feeder, Os};
use crate::hotkey::{self, Hotkeys, Which};
use crate::hover::{self, HoverTracker, ViewportPoint};
use crate::instance::{Instance, RequestWatch};
use crate::lifecycle::{self, Job, Outcome};
use crate::live::{self, Changed, Launch, LiveView, PanelFacts};
use crate::login::{self, LoginItem};
use crate::menu::{self, Choice, StripMenu};
use crate::monitor::{self, MonitorId};
use crate::pinning::{self, AttachError, DEFAULT_LEVEL, LevelMode, Monitor, Observers, Pinning};
use crate::prefs::{Pref, Prefs, Saved, Writer};
use crate::present::{FrameGate, PresentError, Presenter, SoftbufferPresenter};
use crate::probe::report::{Activity, EndReason, ProbeEvent, ScaleReport};
use crate::probe::tools::ToolsReport;
use crate::probe::{self, Advance, Launch as ProbeLaunch, Probe};
use crate::raise::RaiseTimer;
use crate::settings::{MonitorChoice, RepoFacts, SettingsForm};
use crate::shell::{DataFile, Export, Intent, Picking, Shell};
use crate::soil::{Hits, SoilPanel};
use crate::timelapse::form::TimeLapseForm;
use crate::timelapse::job::{Msg, Session};
use crate::{crash, discover, now_secs, portability};

/// What the strip runs as.
pub enum Mode {
    /// The living strip, shown from this feed.
    Live(Feed),
    Probe(ProbeKind),
}

/// What the strip shows: the sample, composed once and still, for the pinning probe, whose
/// idle step counts on a strip that draws nothing; otherwise a feed, living.
enum Scene {
    Still {
        frame: NativeImage,
    },
    Living {
        living: Box<Living>,
        feeder: Box<Feeder>,
        /// The moment the composer's clock counts from.
        origin: Instant,
        /// What the pointer has the soil panel and the hover labels do.
        soil: SoilPanel,
        /// While paused, what the panel and the hover labels word, held with the scene.
        held: Option<Box<Held>>,
    },
}

/// The facts a paused strip words, as they were when it paused, so what it names is what it
/// shows.
struct Held {
    facts: Option<PanelFacts>,
    /// The wall clock then, in Unix seconds.
    wall: i64,
}

/// What the pointer did over the strip.
#[derive(Clone, Copy, Debug)]
enum Pointer {
    /// Moved to a pixel of the viewport, or off its picture.
    Moved(Option<ViewportPoint>),
    /// The left button went down where it is.
    Pressed,
    Left,
}

impl Scene {
    /// The scene for `mode`; a connected repository's starts from `live`, laid out by its salt;
    /// day, night and dim mode as `prefs` say.
    fn new(mode: &Mode, live: Option<(&LiveView, InstallSalt)>, prefs: &Prefs) -> Scene {
        let feed = match mode {
            Mode::Live(feed) => feed.clone(),
            #[cfg(feature = "dev")]
            Mode::Probe(ProbeKind::Egui) => Feed::Thriving,
            Mode::Probe(ProbeKind::Pinning) => return Scene::still(Viewport::SCENE),
        };
        let os = Os::read();
        let (feeder, salt) = match live {
            Some((view, salt)) => (
                Feeder::live(view.scene().clone(), view.label(), Millis(0), &os),
                salt,
            ),
            None => (Feeder::new(feed, Millis(0), &os), preset::SALT),
        };
        let mut feeder = Box::new(feeder);
        feeder.set_times(prefs.day_night(), prefs.dim_after(), Millis(0), &os);
        Scene::Living {
            living: Box::new(Living::new(
                feeder.scene(),
                &feeder.overlay(),
                salt,
                Millis(0),
            )),
            feeder,
            origin: Instant::now(),
            soil: SoilPanel::default(),
            held: None,
        }
    }

    /// Moves the composer toward what the connected repository now shows, an open page of the
    /// soil panel worded anew.
    fn show(&mut self, view: &LiveView, now: Instant) {
        if let Scene::Living {
            living,
            feeder,
            origin,
            soil,
            held,
        } = self
        {
            let at = Self::millis(*origin, now);
            let dim = feeder.dim();
            feeder.show(view.scene(), view.label(), at, &Os::read());
            feeder.set_hover(Self::worded(living, soil, held.as_deref(), view));
            Self::dimmed(feeder, soil, dim);
            living.update(feeder.scene(), &feeder.overlay(), at);
        }
    }

    /// Dim mode closes the panel, pinned or not, and any hover label, so nothing static stays
    /// on the screen (N-12): when the feed, dim before as `was`, has just dimmed.
    fn dimmed(feeder: &mut Feeder, soil: &mut SoilPanel, was: bool) {
        if !was && feeder.dim() {
            soil.dimmed();
            feeder.set_hover(Hover::default());
        }
    }

    /// The hover layer the soil panel has, worded from what the connected repository shows,
    /// or from what it showed when the strip paused.
    fn worded(living: &Living, soil: &SoilPanel, held: Option<&Held>, view: &LiveView) -> Hover {
        let (facts, wall) = match held {
            Some(held) => (held.facts.as_ref(), held.wall),
            None => (view.panel(), live::wall_secs()),
        };
        soil.hover(facts, wall, living.catalog(), living.columns())
    }

    /// Whether the scene is paused; `None` for one that cannot be.
    fn paused(&self) -> Option<bool> {
        match self {
            Scene::Living { feeder, .. } => Some(feeder.paused()),
            Scene::Still { .. } => None,
        }
    }

    /// Pauses the scene at `now`, holding what the connected repository `view` shows for the
    /// panel, or resumes it then, from what the feed and `view` now show.
    fn set_paused(&mut self, paused: bool, view: Option<&LiveView>, now: Instant) {
        if let Scene::Living {
            living,
            feeder,
            origin,
            soil,
            held,
        } = self
        {
            let at = Self::millis(*origin, now);
            *held = paused.then(|| {
                Box::new(Held {
                    facts: view.and_then(LiveView::panel).cloned(),
                    wall: live::wall_secs(),
                })
            });
            feeder.set_paused(paused, at, &Os::read());
            if let Some(view) = view.filter(|_| *feeder.feed() == Feed::Live) {
                feeder.set_hover(Self::worded(living, soil, held.as_deref(), view));
            }
            living.update(feeder.scene(), &feeder.overlay(), at);
        }
    }

    /// Day, night and dim mode as `prefs` now say, from `now`.
    fn set_times(&mut self, prefs: &Prefs, now: Instant) {
        if let Scene::Living {
            living,
            feeder,
            origin,
            soil,
            ..
        } = self
        {
            let at = Self::millis(*origin, now);
            let dim = feeder.dim();
            feeder.set_times(prefs.day_night(), prefs.dim_after(), at, &Os::read());
            Self::dimmed(feeder, soil, dim);
            living.update(feeder.scene(), &feeder.overlay(), at);
        }
    }

    /// Moves the soil panel and the hover labels as the pointer has them, while the connected
    /// repository is shown.
    fn point(&mut self, pointer: Pointer, view: &LiveView, now: Instant) {
        let Scene::Living {
            living,
            feeder,
            origin,
            soil,
            held,
        } = self
        else {
            return;
        };
        if *feeder.feed() != Feed::Live {
            return;
        }
        let at = Self::millis(*origin, now);
        let drawn = feeder.hover().panel.as_ref().and_then(|shown| {
            panel::layout(
                living.catalog(),
                &shown.page,
                shown.anchor,
                living.columns(),
            )
        });
        let hits = |landmark| Hits {
            panel: drawn.as_ref(),
            landmark,
        };
        match pointer {
            Pointer::Moved(point) => {
                let landmark = point.and_then(|p| living.landmark_at(p.x, p.y, at));
                soil.moved(point, &hits(landmark));
            }
            Pointer::Pressed => soil.pressed(&hits(None)),
            Pointer::Left => soil.left(),
        }
        let hover = Self::worded(living, soil, held.as_deref(), view);
        if hover != *feeder.hover() {
            living.hover(&hover, at);
            feeder.set_hover(hover);
        }
    }

    /// The composer's clock at `instant`.
    fn millis(origin: Instant, instant: Instant) -> Millis {
        Millis(u64::try_from(instant.duration_since(origin).as_millis()).unwrap_or(u64::MAX))
    }

    fn instant(origin: Instant, at: Millis) -> Option<Instant> {
        origin.checked_add(Duration::from_millis(at.0))
    }

    /// When the living strip next wants a frame.
    fn frame_due(&self) -> Option<Instant> {
        match self {
            Scene::Still { .. } => None,
            Scene::Living { living, origin, .. } => Self::instant(*origin, living.due()?),
        }
    }

    /// When the living strip next wants anything: a frame, or a look by its feed.
    fn due(&self) -> Option<Instant> {
        match self {
            Scene::Still { .. } => None,
            Scene::Living { feeder, origin, .. } => {
                let look = Self::instant(*origin, feeder.due());
                [self.frame_due(), look].into_iter().flatten().min()
            }
        }
    }

    /// Lets the feed look at the OS if it is due to, or at once with `force`, and moves the
    /// composer toward what it then shows.
    fn look(&mut self, now: Instant, force: bool) {
        if let Scene::Living {
            living,
            feeder,
            origin,
            soil,
            ..
        } = self
        {
            let at = Self::millis(*origin, now);
            let dim = feeder.dim();
            if (force || feeder.due() <= at) && feeder.tick(at, &Os::read(), force) {
                Self::dimmed(feeder, soil, dim);
                living.update(feeder.scene(), &feeder.overlay(), at);
            }
        }
    }

    /// Brings the scene to `now`; true if its pixels changed.
    fn advance(&mut self, now: Instant) -> bool {
        self.look(now, false);
        match self {
            Scene::Still { .. } => false,
            Scene::Living { living, origin, .. } => living.advance(Self::millis(*origin, now)),
        }
    }

    /// Lays the living strip out anew by `salt` from `now`, showing what the connected
    /// repository shows (re-seed).
    fn reseed(&mut self, view: &LiveView, salt: InstallSalt, now: Instant) {
        if let Scene::Living {
            living,
            feeder,
            origin,
            ..
        } = self
        {
            let at = Self::millis(*origin, now);
            feeder.show(view.scene(), view.label(), at, &Os::read());
            living.reseed(feeder.scene(), &feeder.overlay(), salt, at);
        }
    }

    /// Whether line `index` of the connected repository's label has had its turn on the plate
    /// by `now`; false while the strip shows another feed.
    fn label_step_done(&self, index: usize, now: Instant) -> bool {
        match self {
            Scene::Living {
                living,
                feeder,
                origin,
                ..
            } if *feeder.feed() == Feed::Live => {
                living.label_step_done(index, Self::millis(*origin, now))
            }
            _ => false,
        }
    }

    /// Shows `feed` with `overrides` from `now`, as the debug window chose.
    #[cfg(feature = "dev")]
    fn choose(&mut self, feed: Feed, overrides: Overrides, now: Instant) {
        if let Scene::Living {
            living,
            feeder,
            origin,
            soil,
            held,
        } = self
        {
            let at = Self::millis(*origin, now);
            // Another feed has no panel; back on the repository, it starts closed. A choice
            // ends a pause, so it shows.
            *soil = SoilPanel::default();
            *held = None;
            feeder.set_hover(Hover::default());
            feeder.choose(feed, overrides, at, &Os::read());
            living.update(feeder.scene(), &feeder.overlay(), at);
        }
    }

    /// What the living strip shows, for the debug window to start from.
    #[cfg(feature = "dev")]
    fn shown(&self) -> Option<(&Feed, Overrides)> {
        match self {
            Scene::Still { .. } => None,
            Scene::Living { feeder, .. } => Some((feeder.feed(), feeder.overrides())),
        }
    }

    /// The sample, composed once for `viewport`.
    fn still(viewport: Viewport) -> Scene {
        let art = Art::embedded();
        let frame = sample::frame_in(
            art,
            ArtTable::embedded(),
            copy::sample_label(),
            0,
            0,
            viewport,
        );
        Scene::Still {
            frame: composite(art, &frame),
        }
    }

    /// Shows `viewport` from `now` on, the strip's columns at its scale; true if that is
    /// another, so its pixels are new.
    fn fit(&mut self, viewport: Viewport, now: Instant) -> bool {
        match self {
            Scene::Still { frame } if frame.width() != viewport.width() => {
                *self = Scene::still(viewport);
                true
            }
            Scene::Still { .. } => false,
            Scene::Living { living, origin, .. } => {
                living.fit(viewport, Self::millis(*origin, now))
            }
        }
    }

    fn pixels(&self) -> &NativeImage {
        match self {
            Scene::Still { frame } => frame,
            Scene::Living { living, .. } => living.image(),
        }
    }
}

/// What other threads send the event loop.
pub enum UserEvent {
    HotKey(GlobalHotKeyEvent),
    Menu(MenuId),
    System(SystemEvent),
    ProbeLine(String),
    ProbeEnd,
    /// What the probe's tools step found.
    ProbeTools(Box<ToolsReport>),
    /// An update from the engine of that generation.
    Engine(u64, Update),
    /// A second launch asked for the settings.
    ShowSettings,
    /// The folder picker closed, with the folder chosen, if one was.
    Picked(Picking, Option<PathBuf>),
    /// A lifecycle change of that generation is done.
    Lifecycle(u64, Box<Outcome>),
    /// The settings' writer saved a change, or asked the OS about opening at login.
    Saved(Saved),
    /// The discovery report for the settings window, of that generation.
    Discovered(u64, String),
    /// What a job of the time-lapse window of that generation says.
    TimeLapse(u64, Msg),
    /// What the settings window's export of that generation says.
    Portability(u64, portability::Msg),
}

/// Why the strip could not run. Its wording is in the copy module.
#[derive(Debug)]
pub enum StripError {
    EventLoop(EventLoopError),
    Window(OsError),
    Attach(AttachError),
    Present(PresentError),
    NoDisplay,
    /// Nothing could be connected: `--repo` named something that is not a repository, or the
    /// store failed.
    Connect(Option<PathBuf>, ConnectError),
    /// No repository is connected and the connect window could not open.
    NoConnectWindow,
}

impl From<EventLoopError> for StripError {
    fn from(err: EventLoopError) -> Self {
        Self::EventLoop(err)
    }
}

/// Runs the strip until it is quit. With a probe, it also records and reports; with a data
/// directory, it connects what `launch` names (or the repository connected last), shows it,
/// and shuts its engine down on the way out.
pub fn run(mode: &Mode, launch: Option<Launch>) -> Result<(), StripError> {
    let pinning_probe = matches!(mode, Mode::Probe(ProbeKind::Pinning));
    let mut builder = EventLoop::<UserEvent>::with_user_event();
    pinning::configure_event_loop(&mut builder, pinning_probe);
    let event_loop = builder.build()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    // Both crates keep only the first handler they are given, so these come before any menu
    // or hotkey exists.
    menu::forward_events(event_loop.create_proxy());
    hotkey::forward_events(event_loop.create_proxy());
    let probe = pinning_probe.then(|| Probe::start(event_loop.create_proxy()));
    // The engine starts once the loop exists, so its updates go straight to the loop; the
    // strip waits only for connect's first answer, so the composer is built with the
    // terrarium's own salt from its first frame.
    let proxy = event_loop.create_proxy();
    let (repo, prefs) = match launch {
        Some(launch) => {
            let (repo, prefs) = Repo::start(launch, &proxy)?;
            (Some(repo), prefs)
        }
        // Without a data directory nothing is stored: the strip runs on the defaults.
        None => (None, Prefs::default()),
    };
    let mut strip = Strip::new(proxy, probe, mode, repo, prefs);
    event_loop.run_app(&mut strip)?;
    match strip.failed {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

/// A run with a data directory: the repository connected, or the wait for one.
struct Repo {
    data: DataDir,
    _instance: Instance,
    _requests: Option<RequestWatch>,
    /// The engine running; none while nothing is connected or a change holds it.
    engine: Option<Engine>,
    /// The generation of the engine whose updates are shown.
    generation: u64,
    connected: Option<Connected>,
    /// The connect window offers "Open at login".
    first_run: bool,
    /// The first run's "Open at login", applied once the connect it was asked with succeeds.
    pending_login: Option<bool>,
    /// Saves the settings changed, and the GitHub token, off the loop.
    writer: Writer,
    /// Where the GitHub token is kept: the OS keychain.
    keychain: Arc<dyn Keychain>,
}

/// What a lifecycle change leaves the strip to do.
#[derive(Debug, PartialEq, Eq)]
enum Effect {
    Nothing,
    /// Connected: open the strip, laid out by this salt.
    Open(InstallSalt),
    /// The engine started again: show what the live view says, laid out anew by the salt
    /// given (a re-seed); with the history changed, a time-lapse of it is out of date.
    Show {
        reseeded: Option<InstallSalt>,
        history_changed: bool,
    },
    /// Disconnected: close the strip, and open the connect window.
    Close,
}

/// The repository connected, and what the strip shows of it.
struct Connected {
    terrarium: Box<Terrarium>,
    live: LiveView,
    /// What the Knip run and the GitHub Actions poll last said of themselves.
    tools: Tools,
}

impl Repo {
    /// Connects what `launch` names, and watches for a second launch's requests; with the
    /// settings the strip starts on.
    fn start(
        launch: Launch,
        proxy: &EventLoopProxy<UserEvent>,
    ) -> Result<(Repo, Prefs), StripError> {
        let Launch {
            data,
            repo,
            instance,
        } = launch;
        // The settings, and the crash log they may ask for, before anything else can fail.
        let prefs = Store::open(&data)
            .map(|store| Prefs::load(&store))
            .unwrap_or_default();
        crash::install(data.logs_dir(), prefs.crash_log);
        if prefs.crash_log {
            let _ = crash::prune(&data.logs_dir(), crash::KEEP);
        }
        let saved = proxy.clone();
        let keychain: Arc<dyn Keychain> = Arc::new(NativeKeychain::new(
            KEYCHAIN_SERVICE,
            KEYCHAIN_ACCOUNT_GITHUB,
        ));
        let writer = Writer::start(
            data.clone(),
            Box::new(login::System),
            || terrarium_platform::login::status(BUNDLE_ID),
            Arc::clone(&keychain),
            Box::new(move |answer| {
                let _ = saved.send_event(UserEvent::Saved(answer));
            }),
        );
        let config = lifecycle::engine_config(data.clone(), repo.clone(), &keychain);
        let (engine, start) = Engine::spawn(config, lifecycle::sink(proxy.clone(), 0))
            .map_err(|err| StripError::Connect(repo, err))?;
        let requests = proxy.clone();
        let requests = match instance.watch(move || {
            let _ = requests.send_event(UserEvent::ShowSettings);
        }) {
            Ok((watch, waiting)) => {
                if waiting {
                    let _ = proxy.send_event(UserEvent::ShowSettings);
                }
                Some(watch)
            }
            Err(err) => {
                eprintln!("{}", copy::requests_unwatched(&err));
                None
            }
        };
        let (engine, connected) = match start {
            Start::Connected { terrarium, scene } => (
                Some(engine),
                Some(Connected {
                    live: LiveView::new(terrarium.repo.name.clone(), scene),
                    terrarium,
                    tools: Tools::default(),
                }),
            ),
            Start::NotConnected => (None, None),
        };
        let repo = Repo {
            first_run: lifecycle::first_run(&data),
            data,
            _instance: instance,
            _requests: requests,
            engine,
            generation: 0,
            connected,
            pending_login: None,
            writer,
            keychain,
        };
        Ok((repo, prefs))
    }

    /// Takes in a change of `generation` and its `outcome`: the engine it started, the
    /// connected repository, the windows' forms, and on a first connect the login choice,
    /// applied through `login`. Says what the strip is left to do. A change a later one
    /// outran has nothing left to show: its engine stops on a thread of its own, so the loop
    /// never waits for it.
    fn changed(
        &mut self,
        generation: u64,
        outcome: Outcome,
        shell: &mut Shell,
        login: &mut dyn LoginItem,
    ) -> Effect {
        if generation != self.generation {
            thread::spawn(move || drop(outcome));
            return Effect::Nothing;
        }
        match outcome {
            Outcome::Connected {
                engine,
                terrarium,
                scene,
                report,
            } => {
                self.engine = Some(engine);
                self.first_run = false;
                if let Some(wanted) = self.pending_login.take()
                    && let Some(line) = login::first_run(login, wanted)
                {
                    eprintln!("{line}");
                }
                let name = terrarium.repo.name.clone();
                let salt = InstallSalt::new(terrarium.install_salt);
                self.connected = Some(Connected {
                    live: LiveView::new(name.clone(), scene),
                    terrarium,
                    tools: Tools::default(),
                });
                if let Some(form) = &mut shell.connect {
                    form.connected(name, report);
                }
                shell.redraw(WindowKind::Connect);
                Effect::Open(salt)
            }
            Outcome::NotConnected(err) => {
                self.pending_login = None;
                if let Some(form) = &mut shell.connect {
                    form.failed(copy::connect_problem(&err));
                }
                shell.redraw(WindowKind::Connect);
                Effect::Nothing
            }
            Outcome::Restarted { changed, started } => {
                let history_changed = changed
                    .as_ref()
                    .is_ok_and(lifecycle::Changed::changes_history);
                let mut effect = Effect::Nothing;
                if let Some(connected) = &mut self.connected {
                    match started {
                        Ok((engine, terrarium)) => {
                            self.engine = Some(engine);
                            let reseeded = (terrarium.install_salt
                                != connected.terrarium.install_salt)
                                .then(|| InstallSalt::new(terrarium.install_salt));
                            connected.live.restarted(terrarium.repo.name.clone());
                            connected.terrarium = terrarium;
                            effect = Effect::Show {
                                reseeded,
                                history_changed,
                            };
                        }
                        Err(err) => {
                            eprintln!("{}", copy::connect_failed(None, &err));
                            // Halted already, the scene has nothing new to show; a history
                            // changed still dates a time-lapse of it.
                            if connected.live.apply(Update::Halted) != Changed::Nothing
                                || history_changed
                            {
                                effect = Effect::Show {
                                    reseeded: None,
                                    history_changed,
                                };
                            }
                        }
                    }
                }
                let facts = self.facts();
                if let Some(form) = &mut shell.settings {
                    if let Some(facts) = facts {
                        form.set_repo(facts);
                    }
                    match &changed {
                        Ok(lifecycle::Changed::Terrarium) => form.finished(None),
                        Ok(lifecycle::Changed::Imported(report)) => {
                            form.imported(copy::settings::imported(report));
                        }
                        Err(refused) => form.finished(Some(copy::change_refused(refused))),
                    }
                }
                shell.redraw(WindowKind::Settings);
                effect
            }
            Outcome::Disconnected => {
                self.connected = None;
                self.engine = None;
                shell.close(WindowKind::Settings);
                shell.settings = None;
                shell.connect = None;
                Effect::Close
            }
        }
    }

    /// The repository as the settings window shows it.
    fn facts(&self) -> Option<RepoFacts> {
        let connected = self.connected.as_ref()?;
        let repo = &connected.terrarium.repo;
        Some(RepoFacts {
            name: repo.name.clone(),
            path: repo.path.clone(),
            default_branch: repo.default_branch.clone(),
            binding: RepoBinding::new(&connected.terrarium.seed, &repo.path),
        })
    }
}

#[expect(
    clippy::struct_excessive_bools,
    reason = "independent flags the event loop takes up in turn"
)]
struct Strip {
    /// Cloned for each sender: the system observers, an engine's sink, a picker, a lifecycle
    /// change.
    proxy: EventLoopProxy<UserEvent>,
    probe: Option<Probe>,
    view: Option<View>,
    /// What the strip shows; none while no repository is connected.
    scene: Option<Scene>,
    /// The redraw on its way was asked for by the living strip's schedule, not by the OS: it
    /// is presented only if the pixels changed.
    scheduled: bool,
    level: LevelMode,
    raise: RaiseTimer,
    hover: HoverTracker,
    gate: FrameGate,
    placement_pending: bool,
    /// Whether to look at the full-screen apps once the events in hand are handled (Windows).
    fullscreen_pending: bool,
    /// The edge the probe's step puts the strip on, over the setting's.
    probe_edge: Option<Edge>,
    idle_seen: bool,
    failed: Option<StripError>,
    /// Made with the first strip window, and kept while the app runs.
    menu: Option<StripMenu>,
    hotkeys: Option<Hotkeys>,
    /// The settings the strip runs on.
    prefs: Prefs,
    /// The generation of the settings window's discovery report: only the latest is shown.
    discovery: u64,
    /// A run with a data directory.
    repo: Option<Repo>,
    /// The windows beside the strip.
    shell: Shell,
    /// The time-lapse window's work, while it is open.
    timelapse: Option<Session>,
    /// The generation of the latest time-lapse window: only its jobs are heard.
    lapses: u64,
    /// The generation of the settings window's latest export: only it is heard.
    exports: u64,
    reopen: Option<pinning::ReopenHook>,
    #[cfg(feature = "dev")]
    dev: Dev,
}

/// Everything that exists while the strip's window does. Fields drop in order, so what holds
/// the window lets go of it before the window itself goes.
struct View {
    _observers: Observers,
    pinning: Pinning,
    presenter: SoftbufferPresenter,
    scale: StripScale,
    window: Rc<Window>,
}

impl Strip {
    fn new(
        proxy: EventLoopProxy<UserEvent>,
        probe: Option<Probe>,
        mode: &Mode,
        repo: Option<Repo>,
        prefs: Prefs,
    ) -> Self {
        let level = probe.as_ref().map_or(DEFAULT_LEVEL, Probe::level);
        let scene = match repo.as_ref().map(|repo| repo.connected.as_ref()) {
            // A run with a data directory and nothing connected shows no strip.
            Some(None) => None,
            Some(Some(connected)) => Some(Scene::new(
                mode,
                Some((
                    &connected.live,
                    InstallSalt::new(connected.terrarium.install_salt),
                )),
                &prefs,
            )),
            None => Some(Scene::new(mode, None, &prefs)),
        };
        Self {
            proxy,
            probe,
            view: None,
            scene,
            scheduled: false,
            level,
            raise: RaiseTimer::default(),
            hover: HoverTracker::default(),
            gate: FrameGate::default(),
            placement_pending: false,
            fullscreen_pending: false,
            probe_edge: None,
            idle_seen: false,
            failed: None,
            menu: None,
            hotkeys: None,
            prefs,
            discovery: 0,
            repo,
            shell: Shell::default(),
            timelapse: None,
            lapses: 0,
            exports: 0,
            reopen: None,
            #[cfg(feature = "dev")]
            dev: Dev::new(matches!(mode, Mode::Probe(ProbeKind::Egui))),
        }
    }

    fn open(&mut self, event_loop: &ActiveEventLoop) -> Result<View, StripError> {
        let attributes = pinning::window_attributes()
            .with_title(copy::window_title())
            .with_decorations(false)
            .with_resizable(false)
            .with_visible(false)
            .with_active(false);
        let window = Rc::new(
            event_loop
                .create_window(attributes)
                .map_err(StripError::Window)?,
        );
        let pinning = Pinning::attach(&window).map_err(StripError::Attach)?;
        let presenter = SoftbufferPresenter::new(&window).map_err(StripError::Present)?;
        if self.menu.is_none() {
            self.menu = StripMenu::new()
                .inspect_err(|err| eprintln!("{}", copy::menu_unavailable(err)))
                .ok();
        }
        self.sync_menu();
        if self.hotkeys.is_none() {
            match Hotkeys::new() {
                Ok(mut hotkeys) => {
                    for which in Which::ALL {
                        let binding = self.prefs.binding(which);
                        if let Err(err) = hotkeys.bind(which, binding) {
                            self.hotkey_unavailable(which, binding, &err);
                        }
                    }
                    self.hotkeys = Some(hotkeys);
                }
                Err(err) => {
                    for which in Which::ALL {
                        self.hotkey_unavailable(which, self.prefs.binding(which), &err);
                    }
                }
            }
        }
        let proxy = self.proxy.clone();
        let observers = pinning.observe(move |event| {
            let _ = proxy.send_event(UserEvent::System(event));
        });

        let monitors = pinning.monitors(&window);
        // Only the probe reports every monitor.
        let reported = self
            .probe
            .is_some()
            .then(|| monitors.iter().map(|m| m.info.clone()).collect());
        let monitor = choose(monitors, self.prefs.monitor.as_ref()).ok_or(StripError::NoDisplay)?;
        if let Some(probe) = &mut self.probe {
            probe.launched(ProbeLaunch {
                os: pinning::os_version(),
                separate_spaces: pinning::separate_spaces(),
                monitors: reported.unwrap_or_default(),
                chosen: Some(monitor.info.id.clone()),
                taskbar: pinning::taskbar(),
            });
        }
        let mut view = View {
            _observers: observers,
            pinning,
            presenter,
            window,
            // Until the placement says otherwise: on Windows the reserved band can be narrower.
            scale: StripScale::for_physical_width(
                monitor.info.physical_width,
                self.prefs.size_step,
            ),
        };
        self.place(&mut view, &monitor);
        self.apply_level(&view);
        // A strip opened over a full-screen app steps behind it at once.
        self.fullscreen_pending = true;
        view.pinning.show(&view.window);
        view.window.request_redraw();
        let active = pinning::app_is_active();
        self.record(ProbeEvent::AppActive {
            at: copy::AT_WINDOW_SHOWN,
            active,
        });
        self.record_snapshot(&view);
        Ok(view)
    }

    /// Runs `f` with the view taken out of `self`, so it can record probe events while
    /// holding the window.
    fn with_view(&mut self, f: impl FnOnce(&mut Self, &mut View)) {
        if let Some(mut view) = self.view.take() {
            f(self, &mut view);
            self.view = Some(view);
        }
    }

    /// Moves the strip onto `monitor`, and takes the scale the placement gives.
    fn place(&mut self, view: &mut View, monitor: &Monitor) {
        let (step, edge) = (
            self.prefs.size_step,
            self.probe_edge.unwrap_or(self.prefs.edge),
        );
        let Some((scale, frame)) = view.pinning.place(&view.window, monitor, step, edge) else {
            return;
        };
        view.scale = scale;
        self.record(ProbeEvent::Placed {
            monitor: monitor.info.id.clone(),
            strip: ScaleReport::from(&view.scale),
            frame: Box::new(frame),
        });
    }

    /// Chooses the monitor again and moves the strip there, after a display change, a wake, a
    /// moved taskbar or a change of the settings.
    fn replace(&mut self, view: &mut View) {
        let monitors = view.pinning.monitors(&view.window);
        self.show_monitors(&monitors);
        if let Some(monitor) = choose(monitors, self.prefs.monitor.as_ref()) {
            self.place(view, &monitor);
            view.window.request_redraw();
        }
    }

    /// The displays connected, as the settings window lists them, with the strip's scales on
    /// the one it is on.
    fn show_monitors(&mut self, monitors: &[Monitor]) {
        let Some(form) = &mut self.shell.settings else {
            return;
        };
        let choices = monitors
            .iter()
            .map(|monitor| {
                let info = &monitor.info;
                MonitorChoice {
                    id: info.id.clone(),
                    label: copy::settings::monitor(
                        &info.name,
                        info.physical_width,
                        info.physical_height,
                        info.primary,
                    ),
                }
            })
            .collect();
        let scales = monitor::choose(monitors, self.prefs.monitor.as_ref()).map(|index| {
            let width = monitors[index].info.physical_width;
            [SizeStep::Full, SizeStep::OneSmaller]
                .map(|step| StripScale::for_physical_width(width, step).scale)
        });
        form.set_monitors(choices, scales);
        self.shell.redraw(WindowKind::Settings);
    }

    /// What the OS says about the strip now (Windows), recorded only while a probe report is
    /// still open.
    fn record_snapshot(&mut self, view: &View) {
        if self.probe.as_ref().is_some_and(Probe::is_open)
            && let Some(snapshot) = view.pinning.snapshot()
        {
            self.record(ProbeEvent::Os(snapshot));
        }
    }

    fn apply_level(&mut self, view: &View) {
        view.pinning.apply_level(&view.window, self.level);
        self.record(ProbeEvent::Level {
            mode: self.level,
            value: view.pinning.level(),
        });
    }

    fn record(&mut self, event: ProbeEvent) {
        if let Some(probe) = &mut self.probe {
            probe.record(event);
        }
    }

    /// Recorded only while probing: asking the OS costs a little on every event.
    fn activity(&self) -> Activity {
        if self.probe.is_none() {
            return Activity {
                app_active: None,
                frontmost: None,
            };
        }
        Activity {
            app_active: pinning::app_is_active(),
            frontmost: pinning::frontmost_app(),
        }
    }

    fn end_probe(&mut self, reason: EndReason) {
        if let Some(view) = self.view.take() {
            self.record_snapshot(&view);
            self.view = Some(view);
        }
        let frames = self.gate.count();
        if let Some(probe) = &mut self.probe {
            probe.end(reason, frames);
        }
    }

    fn answer(&mut self, event_loop: &ActiveEventLoop, view: &View, note: &str) {
        self.record_snapshot(view);
        let frames = self.gate.count();
        let Some(probe) = &mut self.probe else {
            return;
        };
        let setup = match probe.answer(note, frames) {
            Advance::Done => {
                event_loop.exit();
                return;
            }
            Advance::Next(setup) => setup,
        };
        if setup.tools {
            probe::tools::start(self.proxy.clone());
        }
        if setup.edge != self.probe_edge.unwrap_or(self.prefs.edge) {
            self.probe_edge = Some(setup.edge);
            self.placement_pending = true;
        }
        if setup.level != self.level {
            self.level = setup.level;
            // A raise in progress keeps the strip up; the restore applies the new level.
            if self.raise.deadline().is_none() {
                self.apply_level(view);
            }
        }
    }

    fn window_input(&mut self, event_loop: &ActiveEventLoop, view: &mut View, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.record_snapshot(view);
                self.end_probe(EndReason::WindowClosed);
                event_loop.exit();
            }
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                let Some(scene) = &mut self.scene else {
                    return;
                };
                let fitted = scene.fit(view.scale.viewport(), now);
                let changed = scene.advance(now) || fitted;
                // The OS's own requests always present; the schedule's only on a change.
                let wanted = changed || !std::mem::take(&mut self.scheduled);
                if wanted && self.gate.should_present() {
                    match view.presenter.present(scene.pixels(), &view.scale) {
                        Ok(()) => {
                            self.gate.presented();
                            #[cfg(feature = "dev")]
                            self.dev.fps.presented(now);
                        }
                        Err(err) => eprintln!("{}", copy::present_failed(&err)),
                    }
                }
                self.once_shown(now);
            }
            WindowEvent::Resized(size) => {
                view.window.request_redraw();
                self.record(ProbeEvent::Resized {
                    width: size.width,
                    height: size.height,
                });
            }
            WindowEvent::ScaleFactorChanged {
                scale_factor,
                mut inner_size_writer,
            } => {
                // The answer is the exact physical size; the move itself waits until the
                // handler has returned. A reserved edge keeps the size it was granted.
                let scale = if pinning::EDGE_RESERVED {
                    Some(view.scale)
                } else {
                    choose(
                        view.pinning.monitors(&view.window),
                        self.prefs.monitor.as_ref(),
                    )
                    .map(|monitor| {
                        StripScale::for_physical_width(
                            monitor.info.physical_width,
                            self.prefs.size_step,
                        )
                    })
                };
                if let Some(scale) = scale {
                    let _ = inner_size_writer
                        .request_inner_size(PhysicalSize::new(scale.width, scale.height));
                }
                self.placement_pending = true;
                self.record(ProbeEvent::ScaleFactor {
                    factor: scale_factor,
                });
            }
            WindowEvent::Occluded(occluded) => {
                // Uncovered, the living strip catches up at once.
                let living = matches!(self.scene, Some(Scene::Living { .. }));
                if self.gate.set_occluded(occluded) || (!occluded && living) {
                    view.window.request_redraw();
                }
                self.record(ProbeEvent::Occluded { occluded });
            }
            WindowEvent::Focused(focused) => self.record(ProbeEvent::Focused { focused }),
            pointer @ (WindowEvent::CursorEntered { .. }
            | WindowEvent::CursorLeft { .. }
            | WindowEvent::CursorMoved { .. }
            | WindowEvent::MouseInput { .. }) => self.pointer_input(view, &pointer),
            _ => {}
        }
    }

    /// The pointer over the strip: the probe records it, the right button opens the menu, and
    /// the soil panel and the hover labels follow it.
    fn pointer_input(&mut self, view: &View, event: &WindowEvent) {
        match *event {
            WindowEvent::CursorEntered { .. } => {
                if self.hover.enter() {
                    let activity = self.activity();
                    self.record(ProbeEvent::PointerEntered(activity));
                }
            }
            WindowEvent::CursorLeft { .. } => {
                if self.hover.leave() {
                    let activity = self.activity();
                    self.record(ProbeEvent::PointerLeft(activity));
                }
                self.pointer(Pointer::Left);
            }
            WindowEvent::CursorMoved { position, .. } => {
                let point = self.hover.moved(position.x, position.y, &view.scale);
                if let Some(probe) = &mut self.probe {
                    probe.moved(point, pinning::app_is_active());
                }
                let point = hover::to_viewport(position.x, position.y, &view.scale);
                self.pointer(Pointer::Moved(point));
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button,
                ..
            } => {
                let activity = self.activity();
                self.record(ProbeEvent::Click {
                    button: format!("{button:?}"),
                    point: self.hover.last(),
                    activity,
                });
                if button == MouseButton::Right
                    && let Some(menu) = &self.menu
                {
                    view.pinning.show_menu(&view.window, menu.menu());
                }
                if button == MouseButton::Left {
                    self.pointer(Pointer::Pressed);
                }
            }
            _ => {}
        }
    }

    /// Moves the soil panel and the hover labels as the pointer has them over a connected
    /// repository's strip.
    fn pointer(&mut self, pointer: Pointer) {
        let (Some(scene), Some(connected)) = (
            &mut self.scene,
            self.repo.as_ref().and_then(|repo| repo.connected.as_ref()),
        ) else {
            return;
        };
        scene.point(pointer, &connected.live, Instant::now());
    }

    /// The note the label shows once has had its turn on the plate: it goes.
    fn once_shown(&mut self, now: Instant) {
        let (Some(scene), Some(connected)) = (
            &mut self.scene,
            self.repo.as_mut().and_then(|repo| repo.connected.as_mut()),
        ) else {
            return;
        };
        if let Some(index) = connected.live.once_line()
            && scene.label_step_done(index, now)
            && connected.live.once_shown() != Changed::Nothing
        {
            scene.show(&connected.live, now);
        }
    }

    /// Closes the strip's window: the edge it reserved is given back, and its pinning goes
    /// before the window it holds (`View`'s field order).
    fn close_view(&mut self) {
        if let Some(view) = self.view.take() {
            view.pinning.release();
        }
        self.scheduled = false;
        self.raise = RaiseTimer::default();
        self.hover = HoverTracker::default();
        self.gate = FrameGate::default();
        self.placement_pending = false;
        self.fullscreen_pending = false;
    }

    /// Opens the strip's window for the scene now shown; a failure ends the app.
    fn open_view(&mut self, event_loop: &ActiveEventLoop) {
        if self.view.is_some() {
            return;
        }
        match self.open(event_loop) {
            Ok(view) => self.view = Some(view),
            Err(err) => {
                self.end_probe(EndReason::LaunchFailed);
                self.failed = Some(err);
                event_loop.exit();
            }
        }
    }

    /// Shows the settings, or, with nothing connected, the connect window. Only a run with a
    /// data directory has either.
    fn show_settings(&mut self, event_loop: &ActiveEventLoop) {
        let Some(repo) = &self.repo else {
            return;
        };
        if let Some(facts) = repo.facts() {
            let missing = repo
                .connected
                .as_ref()
                .is_some_and(|connected| connected.live.repo_missing());
            let logs = repo.data.logs_dir().display().to_string();
            let prefs = &self.prefs;
            let form = self
                .shell
                .settings
                .get_or_insert_with(|| SettingsForm::new(facts.clone(), prefs, logs));
            form.set_repo(facts);
            form.set_missing(missing);
            form.set_prefs(prefs);
            if let Some(connected) = &repo.connected {
                form.set_tools(connected.tools.clone());
            }
            repo.writer.ask_login_status();
            self.shell.open(event_loop, WindowKind::Settings);
            self.with_view(|strip, view| {
                let monitors = view.pinning.monitors(&view.window);
                strip.show_monitors(&monitors);
            });
            self.discover();
            return;
        }
        let first_run = repo.first_run;
        self.shell
            .connect
            .get_or_insert_with(|| ConnectForm::new(first_run));
        if !self.shell.open(event_loop, WindowKind::Connect) {
            self.failed = Some(StripError::NoConnectWindow);
            event_loop.exit();
        }
    }

    /// Shows the time-lapse of the connected repository, reading its history again, or brings
    /// the open window forward.
    fn show_timelapse(&mut self, event_loop: &ActiveEventLoop) {
        let Some((data, terrarium)) = self.repo.as_ref().and_then(|repo| {
            let connected = repo.connected.as_ref()?;
            Some((repo.data.clone(), connected.terrarium.as_ref().clone()))
        }) else {
            return;
        };
        if self.timelapse.is_some() && self.shell.is_open(WindowKind::TimeLapse) {
            self.shell.open(event_loop, WindowKind::TimeLapse);
            return;
        }
        self.lapses += 1;
        let strip_scale = self.view.as_ref().map_or(1, |view| view.scale.scale);
        self.shell.timelapse = Some(TimeLapseForm::new(
            terrarium.repo.name.clone(),
            InstallSalt::new(terrarium.install_salt),
            strip_scale,
        ));
        self.timelapse = Some(Session::start(
            self.lapses,
            &self.proxy,
            data,
            terrarium,
            now_secs(),
        ));
        if !self.shell.open(event_loop, WindowKind::TimeLapse) {
            self.close_timelapse();
        }
    }

    /// Closes the time-lapse window, stopping its work; its film goes with it.
    fn close_timelapse(&mut self) {
        self.timelapse = None;
        self.shell.timelapse = None;
        if self.shell.is_open(WindowKind::TimeLapse) {
            self.shell.close(WindowKind::TimeLapse);
        }
    }

    /// A dialog answered: the window that asked takes the path picked, if one was.
    fn picked(&mut self, picking: Picking, path: Option<PathBuf>) {
        match picking {
            Picking::Repository => {
                if let Some(form) = &mut self.shell.connect {
                    form.picked(path);
                }
                self.shell.redraw(WindowKind::Connect);
            }
            Picking::Location => {
                if let Some(form) = &mut self.shell.settings {
                    form.located(path);
                }
                self.shell.redraw(WindowKind::Settings);
            }
            Picking::Save(export) => {
                if let Some(form) = &mut self.shell.timelapse {
                    form.picked(export, path);
                    let intents = form.take_intents();
                    self.carry_out(intents);
                }
                self.shell.redraw(WindowKind::TimeLapse);
            }
            Picking::SaveData(file) => {
                if let Some(form) = &mut self.shell.settings {
                    form.export_to(file, path);
                    let intents = form.take_intents();
                    self.carry_out(intents);
                }
                self.shell.redraw(WindowKind::Settings);
            }
            Picking::Import => {
                if let Some(form) = &mut self.shell.settings {
                    form.import_chosen(path);
                }
                self.shell.redraw(WindowKind::Settings);
            }
        }
    }

    /// Exports the connected terrarium's `file` to `path`, off the loop.
    fn export(&mut self, file: DataFile, path: PathBuf) {
        let Some((data, terrarium)) = self.repo.as_ref().and_then(|repo| {
            let connected = repo.connected.as_ref()?;
            Some((repo.data.clone(), connected.terrarium.as_ref().clone()))
        }) else {
            return;
        };
        self.exports += 1;
        portability::spawn(
            &self.proxy,
            self.exports,
            data,
            terrarium,
            file,
            path,
            now_secs(),
        );
    }

    /// What the settings window's export said, if it is the latest.
    fn export_said(&mut self, generation: u64, msg: &portability::Msg) {
        if generation != self.exports {
            return;
        }
        if let Some(form) = &mut self.shell.settings {
            form.export_said(msg);
        }
        self.shell.redraw(WindowKind::Settings);
    }

    /// What a time-lapse job said, if it is the open window's.
    fn time_lapse_said(&mut self, generation: u64, msg: Msg) {
        let (Some(session), Some(form)) = (&mut self.timelapse, &mut self.shell.timelapse) else {
            return;
        };
        if session.generation != generation {
            return;
        }
        match msg {
            Msg::Ffmpeg(ffmpeg) => form.ffmpeg(ffmpeg),
            Msg::Reading(percent) => form.reading(percent),
            Msg::Read(Ok(lapse)) => {
                session.read(lapse);
                if let Some(Intent::Film { from, to, night }) = form.read(&session.days()) {
                    session.film(from, to, night);
                }
            }
            Msg::Read(Err(why)) => form.failed(why),
            Msg::Filmed { asked, film } => {
                if session.is_latest(asked) {
                    match film {
                        Ok(film) => form.filmed(film, Instant::now()),
                        Err(why) => form.film_failed(why),
                    }
                }
            }
            Msg::Writing(percent) => form.writing(percent),
            Msg::Written(written) => form.written(written),
        }
        self.shell.redraw(WindowKind::TimeLapse);
    }

    /// Writes the time-lapse window's `export` to `path`.
    fn write(&mut self, export: Export, path: PathBuf) {
        let (Some(session), Some(form)) = (&self.timelapse, &self.shell.timelapse) else {
            return;
        };
        match export {
            Export::Gif => {
                if let Some((film, scale)) = form.gif() {
                    session.write_gif(film, scale, path);
                }
            }
            Export::Postcard => {
                if let Some((shot, scale)) = form.postcard() {
                    session.write_postcard(shot, scale, path);
                }
            }
            Export::Mp4 => {
                if let Some((film, scale, ffmpeg)) = form.mp4() {
                    session.write_mp4(film, scale, ffmpeg, path);
                }
            }
        }
    }

    /// Carries out what the windows asked for.
    fn carry_out(&mut self, intents: Vec<Intent>) {
        for intent in intents {
            match intent {
                Intent::Pick(picking) => self.pick(picking),
                Intent::Connect {
                    path,
                    open_at_login,
                } => {
                    if let Some(repo) = &mut self.repo {
                        repo.pending_login = open_at_login;
                    }
                    self.change(Job::Connect {
                        path,
                        first_run: open_at_login,
                    });
                }
                Intent::Relink(path) => self.change_connected(|id| Job::Relink { id, path }),
                Intent::Reseed => self.change_connected(|id| Job::Reseed { id }),
                Intent::Disconnect {
                    delete_observations,
                } => self.change_connected(|id| Job::Disconnect {
                    id,
                    delete_observations,
                }),
                Intent::Done => {
                    self.shell.close(WindowKind::Connect);
                    self.shell.connect = None;
                }
                Intent::Set(pref) => self.set(pref),
                Intent::Token(change) => {
                    if let Some(repo) = &self.repo {
                        repo.writer.change_token(change);
                    }
                }
                Intent::Film { from, to, night } => {
                    if let Some(session) = &mut self.timelapse {
                        session.film(from, to, night);
                    }
                }
                Intent::Write { export, path } => self.write(export, path),
                Intent::Export { file, path } => self.export(file, path),
                Intent::Import(path) => self.change_connected(|id| Job::Import { id, path }),
            }
        }
    }

    /// Applies a setting changed in the settings window at once, then has it saved. A hotkey
    /// the OS refuses changes nothing, and neither does one with no hotkeys to register it with.
    fn set(&mut self, pref: Pref) {
        if let Pref::Hotkey(which, binding) = pref
            && let Err(err) = self.bind(which, binding)
        {
            if let Some(form) = &mut self.shell.settings {
                form.hotkey_refused(which, copy::settings::not_registered(&err));
            }
            self.shell.redraw(WindowKind::Settings);
            return;
        }
        self.prefs.apply(&pref);
        match pref {
            Pref::Monitor(_) | Pref::Edge(_) | Pref::SizeStep(_) => self.placement_pending = true,
            Pref::DayStarts(_) | Pref::NightStarts(_) | Pref::DimAfter(_) => {
                if let Some(scene) = &mut self.scene {
                    scene.set_times(&self.prefs, Instant::now());
                }
            }
            Pref::CrashLog(on) => {
                crash::set_enabled(on);
                if let (true, Some(repo)) = (on, &self.repo) {
                    let _ = crash::prune(&repo.data.logs_dir(), crash::KEEP);
                }
            }
            _ => {}
        }
        if let Some(form) = &mut self.shell.settings {
            form.set_prefs(&self.prefs);
        }
        self.shell.redraw(WindowKind::Settings);
        if let Some(repo) = &self.repo {
            repo.writer.save(pref);
        }
    }

    /// Registers `binding` for `which`, making the hotkeys first if they could not be made at
    /// the start; the other hotkey's keys are registered too if they are not yet.
    fn bind(&mut self, which: Which, binding: hotkey::Binding) -> Result<(), global_hotkey::Error> {
        let other = self.prefs.binding(which.other());
        if let Some(hotkeys) = &mut self.hotkeys {
            hotkeys.bind(which, binding)?;
            // The other's keys may be free now: the ones this hotkey just let go of.
            if !hotkeys.is_bound(which.other()) {
                let _ = hotkeys.bind(which.other(), other);
            }
            return Ok(());
        }
        let mut hotkeys = Hotkeys::new()?;
        hotkeys.bind(which, binding)?;
        if let Err(err) = hotkeys.bind(which.other(), other) {
            self.hotkey_unavailable(which.other(), other, &err);
        }
        self.hotkeys = Some(hotkeys);
        Ok(())
    }

    /// Says a hotkey could not be registered; the probe hears of the raise's.
    fn hotkey_unavailable(
        &mut self,
        which: Which,
        binding: hotkey::Binding,
        err: &global_hotkey::Error,
    ) {
        eprintln!("{}", copy::hotkey_unavailable(which, binding, err));
        if which == Which::Raise {
            self.record(ProbeEvent::HotkeyUnavailable {
                error: err.to_string(),
            });
        }
    }

    /// The writer's answer: a setting saved, or why not, and what the OS says of opening at
    /// login. A saved adapter setting is taken in by the engine, and the report made again.
    fn saved(&mut self, saved: Saved) {
        match saved {
            Saved::Pref {
                pref,
                problem,
                login,
            } => {
                if pref.is_adapter() && problem.is_none() {
                    if let Some(engine) = self.repo.as_ref().and_then(|repo| repo.engine.as_ref()) {
                        engine.refresh();
                    }
                    self.discover();
                }
                if matches!(pref, Pref::OpenAtLogin(_))
                    && let Some(repo) = &self.repo
                {
                    repo.writer.ask_login_status();
                }
                if let Some(form) = &mut self.shell.settings {
                    form.saved(
                        &pref,
                        problem.map(|problem| copy::settings::not_saved(&problem)),
                        login.map(|err| copy::settings::login_problem(&err)),
                    );
                }
            }
            Saved::Token { problem, .. } => {
                if problem.is_none()
                    && let Some(engine) = self.repo.as_ref().and_then(|repo| repo.engine.as_ref())
                {
                    engine.token_changed();
                }
                if let Some(form) = &mut self.shell.settings {
                    form.token_saved(problem);
                }
                self.shell.redraw(WindowKind::Settings);
            }
            Saved::LoginStatus(status) => {
                if let Some(form) = &mut self.shell.settings {
                    form.set_login_status(status);
                }
            }
        }
        self.shell.redraw(WindowKind::Settings);
    }

    /// Makes the settings window's discovery report again, off the loop.
    fn discover(&mut self) {
        let (Some(form), Some(repo)) = (&mut self.shell.settings, &self.repo) else {
            return;
        };
        let Some(connected) = &repo.connected else {
            return;
        };
        form.set_report(None);
        self.discovery += 1;
        let generation = self.discovery;
        let data = repo.data.clone();
        let terrarium = connected.terrarium.clone();
        let proxy = self.proxy.clone();
        let _ = thread::Builder::new()
            .name("terrarium-discovery".into())
            .spawn(move || {
                // Discovery reads files the repository holds: whatever it meets, the window
                // gets an answer.
                let report = std::panic::catch_unwind(|| discover::for_settings(&data, &terrarium))
                    .unwrap_or_else(|_| copy::report_unavailable().to_owned());
                let _ = proxy.send_event(UserEvent::Discovered(generation, report));
            });
    }

    /// Pauses the strip, or resumes it.
    fn toggle_pause(&mut self) {
        let view = self
            .repo
            .as_ref()
            .and_then(|repo| repo.connected.as_ref())
            .map(|connected| &connected.live);
        if let Some(scene) = &mut self.scene
            && let Some(paused) = scene.paused()
        {
            scene.set_paused(!paused, view, Instant::now());
        }
        self.sync_menu();
    }

    /// The menu's pause item, as the scene now is.
    fn sync_menu(&self) {
        if let Some(menu) = &self.menu {
            let connected = self
                .repo
                .as_ref()
                .is_some_and(|repo| repo.connected.is_some());
            menu.sync(self.scene.as_ref().and_then(Scene::paused), connected);
        }
    }

    /// The folder picker, parented to the window that asked: its answer comes back as
    /// [`UserEvent::Picked`]. It is awaited off the loop, which keeps running meanwhile.
    fn pick(&self, picking: Picking) {
        let kind = match picking {
            Picking::Repository => WindowKind::Connect,
            Picking::Location | Picking::SaveData(_) | Picking::Import => WindowKind::Settings,
            Picking::Save(_) => WindowKind::TimeLapse,
        };
        let mut dialog = rfd::AsyncFileDialog::new();
        if let Some(window) = self.shell.window(kind) {
            dialog = dialog.set_parent(window);
        }
        let chosen: Pin<Box<dyn Future<Output = Option<rfd::FileHandle>> + Send>> = match picking {
            Picking::Repository | Picking::Location => Box::pin(dialog.pick_folder()),
            Picking::Save(export) => {
                let Some(form) = &self.shell.timelapse else {
                    return;
                };
                let (filter, extension) = match export {
                    Export::Gif => (copy::timelapse::GIF, "gif"),
                    Export::Postcard => (copy::timelapse::PNG, "png"),
                    Export::Mp4 => (copy::timelapse::MP4, "mp4"),
                };
                Box::pin(
                    dialog
                        .set_file_name(form.file_name(export))
                        .add_filter(filter, &[extension])
                        .save_file(),
                )
            }
            Picking::SaveData(file) => {
                let Some(form) = &self.shell.settings else {
                    return;
                };
                let (filter, extension) = match file {
                    DataFile::Observations => (copy::settings::OBSERVATIONS_FILTER, "jsonl"),
                    DataFile::Climate => (copy::settings::JSON_FILTER, "json"),
                };
                Box::pin(
                    dialog
                        .set_file_name(form.file_name(file))
                        .add_filter(filter, &[extension])
                        .save_file(),
                )
            }
            // No filter: macOS merges filters, and would hide an export renamed; a file that
            // is not one is refused by the import.
            Picking::Import => Box::pin(dialog.pick_file()),
        };
        let proxy = self.proxy.clone();
        let _ = thread::Builder::new()
            .name("terrarium-picker".into())
            .spawn(move || {
                let path = pollster::block_on(chosen).map(|handle| handle.path().to_path_buf());
                let _ = proxy.send_event(UserEvent::Picked(picking, path));
            });
    }

    /// Runs `job` for the connected terrarium.
    fn change_connected(&mut self, job: impl FnOnce(terrarium_store::TerrariumId) -> Job) {
        let Some(id) = self
            .repo
            .as_ref()
            .and_then(|repo| repo.connected.as_ref())
            .map(|connected| connected.terrarium.id)
        else {
            return;
        };
        self.change(job(id));
    }

    /// Runs `job` off the loop with a new engine generation; updates from the engine it stops
    /// are dropped from now on.
    fn change(&mut self, job: Job) {
        let Some(repo) = &mut self.repo else {
            return;
        };
        repo.generation += 1;
        lifecycle::spawn(
            job,
            repo.engine.take(),
            repo.data.clone(),
            Arc::clone(&repo.keychain),
            self.proxy.clone(),
            repo.generation,
        );
    }

    /// A change of `generation` is done: the repository and the forms take it in, and the
    /// strip does what that leaves it to do.
    fn changed(&mut self, event_loop: &ActiveEventLoop, generation: u64, outcome: Outcome) {
        let now = Instant::now();
        let Some(repo) = &mut self.repo else {
            return;
        };
        let first_login = repo.pending_login;
        match repo.changed(generation, outcome, &mut self.shell, &mut login::System) {
            Effect::Nothing => {}
            Effect::Open(salt) => {
                // The first run's choice is the setting now.
                if let Some(on) = first_login {
                    self.prefs.open_at_login = on;
                }
                if let Some(connected) = &repo.connected {
                    let live = Some((&connected.live, salt));
                    self.scene = Some(Scene::new(&Mode::Live(Feed::Live), live, &self.prefs));
                }
                self.open_view(event_loop);
                self.sync_menu();
            }
            Effect::Show {
                reseeded,
                history_changed,
            } => {
                // A relinked, re-seeded or newly imported terrarium's time-lapse is another's.
                if history_changed {
                    self.close_timelapse();
                }
                let Some(repo) = &self.repo else {
                    return;
                };
                if let (Some(scene), Some(connected)) = (&mut self.scene, &repo.connected) {
                    match reseeded {
                        Some(salt) => scene.reseed(&connected.live, salt, now),
                        None => scene.show(&connected.live, now),
                    }
                }
            }
            Effect::Close => {
                self.close_timelapse();
                self.close_view();
                self.scene = None;
                self.sync_menu();
                self.show_settings(event_loop);
            }
        }
    }

    /// An update from the engine: shown if it is the current engine's.
    fn engine_update(&mut self, generation: u64, update: Update) {
        let Some(repo) = &mut self.repo else {
            return;
        };
        let Some(connected) = repo
            .connected
            .as_mut()
            .filter(|_| generation == repo.generation)
        else {
            return;
        };
        if let Update::Tools(tools) = update {
            if let Some(form) = &mut self.shell.settings {
                form.set_tools(tools.clone());
                self.shell.redraw(WindowKind::Settings);
            }
            connected.tools = tools;
            return;
        }
        if connected.live.apply(update) != Changed::Nothing
            && let Some(scene) = &mut self.scene
        {
            scene.show(&connected.live, Instant::now());
        }
        if let Some(form) = &mut self.shell.settings {
            let missing = connected.live.repo_missing();
            if form.is_missing() != missing {
                form.set_missing(missing);
                self.shell.redraw(WindowKind::Settings);
            }
        }
    }
}

/// What a wake asks of the strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Waking {
    /// The feed takes the local time again.
    look: bool,
    /// The engine looks at the repository at once.
    engine: bool,
}

/// The composer carried the clock on through a sleep, but on macOS the monotonic clock stops
/// while asleep, so after the machine wakes the feed takes the local time again and the engine
/// catches truth up. Screens coming back on (a display that slept while the machine did not, or
/// a Windows machine in Modern Standby, which sends no wake of its own) have the feed look
/// again too, but leave the engine to its next minute's look: waking it restarts the GitHub
/// poll's backoff, which every idle display would then reset.
fn waking(event: SystemEvent) -> Waking {
    Waking {
        look: matches!(event, SystemEvent::Woke | SystemEvent::ScreensWoke),
        engine: event == SystemEvent::Woke,
    }
}

/// The monitor the strip belongs on: `preferred` when it is connected.
fn choose(mut monitors: Vec<Monitor>, preferred: Option<&MonitorId>) -> Option<Monitor> {
    let index = monitor::choose(&monitors, preferred)?;
    Some(monitors.swap_remove(index))
}

impl ApplicationHandler<UserEvent> for Strip {
    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        let now = Instant::now();
        self.shell.woke(now);
        #[cfg(feature = "dev")]
        self.dev_woke(event_loop, now);
        #[cfg(not(feature = "dev"))]
        let _ = event_loop;
        let covered = self.gate.occluded();
        if let Some(scene) = self.scene.as_mut().filter(|_| !covered) {
            scene.look(now, false);
        }
        if matches!(cause, StartCause::ResumeTimeReached { .. })
            && !covered
            && self
                .scene
                .as_ref()
                .and_then(Scene::frame_due)
                .is_some_and(|due| due <= now)
            && let Some(view) = &self.view
        {
            self.scheduled = true;
            view.window.request_redraw();
        }
        if !self.raise.restore_due(now) {
            return;
        }
        self.with_view(|strip, view| {
            view.pinning.apply_level(&view.window, strip.level);
            strip.record(ProbeEvent::Restored {
                mode: strip.level,
                value: view.pinning.level(),
            });
        });
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // Once the app has finished launching, so `AppKit`'s own handler does not replace it.
        if self.repo.is_some() && self.reopen.is_none() {
            let proxy = self.proxy.clone();
            self.reopen = pinning::on_reopen(move || {
                let _ = proxy.send_event(UserEvent::ShowSettings);
            });
        }
        // A second launch's request may have opened the connect window already.
        if self.view.is_some() || self.shell.is_open(WindowKind::Connect) {
            return;
        }
        if self.scene.is_some() {
            self.open_view(event_loop);
        } else {
            self.show_settings(event_loop);
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::HotKey(event) => match self.hotkeys.as_ref().and_then(|h| h.which(event)) {
                Some(Which::Raise) => {
                    self.with_view(|strip, view| {
                        // A reserved edge stays topmost, so there is nothing to restore later.
                        if !pinning::EDGE_RESERVED {
                            strip.raise.press(Instant::now());
                        }
                        view.pinning.raise(&view.window);
                        strip.record(ProbeEvent::Raised {
                            value: view.pinning.level(),
                        });
                    });
                }
                Some(Which::TimeLapse) => self.show_timelapse(event_loop),
                None => {}
            },
            UserEvent::Menu(id) => match self.menu.as_ref().and_then(|menu| menu.choice(&id)) {
                Some(Choice::Quit) => {
                    self.end_probe(EndReason::Quit);
                    event_loop.exit();
                }
                Some(Choice::Settings) => self.show_settings(event_loop),
                Some(Choice::TimeLapse) => self.show_timelapse(event_loop),
                Some(Choice::Pause) => self.toggle_pause(),
                #[cfg(feature = "dev")]
                Some(Choice::Debug) => {
                    if let Some((feed, overrides)) = self.scene.as_ref().and_then(Scene::shown) {
                        let feed = feed.clone();
                        self.dev
                            .open_debug(&mut self.shell, event_loop, &feed, overrides);
                    }
                }
                None => {}
            },
            UserEvent::System(event) => {
                // Only a probe keeps it, and the shell hook sends one for every activation.
                if self.probe.is_some() {
                    self.record(ProbeEvent::System {
                        what: format!("{event:?}"),
                    });
                }
                if matches!(
                    event,
                    SystemEvent::ScreensChanged
                        | SystemEvent::Woke
                        | SystemEvent::ScreensWoke
                        | SystemEvent::AppBarPosChanged
                        | SystemEvent::AppBarStateChanged
                        | SystemEvent::TaskbarCreated
                ) {
                    self.placement_pending = true;
                }
                if matches!(event, SystemEvent::FullscreenHint(_)) {
                    self.fullscreen_pending = true;
                }
                if event == SystemEvent::TaskbarCreated {
                    self.with_view(|strip, view| {
                        if let Some(report) = view.pinning.taskbar_created() {
                            strip.record(ProbeEvent::Taskbar(report));
                        }
                    });
                }
                let waking = waking(event);
                if waking.look
                    && let Some(scene) = &mut self.scene
                {
                    scene.look(Instant::now(), true);
                }
                if waking.engine
                    && let Some(engine) = self.repo.as_ref().and_then(|r| r.engine.as_ref())
                {
                    engine.wake();
                }
            }
            UserEvent::ProbeLine(line) => {
                self.with_view(|strip, view| strip.answer(event_loop, view, &line));
            }
            UserEvent::ProbeEnd => {
                self.end_probe(EndReason::InputClosed);
                event_loop.exit();
            }
            UserEvent::ProbeTools(report) => self.record(ProbeEvent::Tools(report)),
            UserEvent::Engine(generation, update) => self.engine_update(generation, update),
            UserEvent::ShowSettings => self.show_settings(event_loop),
            UserEvent::Picked(picking, path) => self.picked(picking, path),
            UserEvent::Lifecycle(generation, outcome) => {
                self.changed(event_loop, generation, *outcome);
            }
            UserEvent::Saved(saved) => self.saved(saved),
            UserEvent::Discovered(generation, report) => {
                // A report a later one outran says what was so before the latest change.
                if generation != self.discovery {
                    return;
                }
                if let Some(form) = &mut self.shell.settings {
                    form.set_report(Some(report));
                }
                self.shell.redraw(WindowKind::Settings);
            }
            UserEvent::TimeLapse(generation, msg) => self.time_lapse_said(generation, msg),
            UserEvent::Portability(generation, msg) => self.export_said(generation, &msg),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        // Only the strip's own window quits the app when closed; the others are the shell's.
        if self
            .view
            .as_ref()
            .is_some_and(|view| view.window.id() == id)
        {
            self.with_view(|strip, view| strip.window_input(event_loop, view, event));
            return;
        }
        if !self.shell.owns(id) {
            return;
        }
        let intents = self.shell_window_event(id, &event);
        self.carry_out(intents);
        // A closed time-lapse window's work stops, and its film goes with it.
        if self.timelapse.is_some() && !self.shell.is_open(WindowKind::TimeLapse) {
            self.close_timelapse();
        }
        // With nothing connected, the connect window is all there is: closed, the app ends.
        if self
            .repo
            .as_ref()
            .is_some_and(|repo| repo.connected.is_none())
            && !self.shell.is_open(WindowKind::Connect)
        {
            event_loop.exit();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // A placement can move the strip to another display, so the full-screen apps are
        // looked at after it.
        let placed = std::mem::take(&mut self.placement_pending);
        if placed {
            self.with_view(Self::replace);
        }
        if std::mem::take(&mut self.fullscreen_pending) || placed {
            self.with_view(|strip, view| {
                if let Some(report) = view.pinning.check_fullscreen(&view.window) {
                    strip.record(ProbeEvent::Fullscreen(report));
                }
            });
        }
        if !self.idle_seen && self.view.is_some() {
            self.idle_seen = true;
            let active = pinning::app_is_active();
            self.record(ProbeEvent::AppActive {
                at: copy::AT_FIRST_IDLE,
                active,
            });
        }
        // While covered, the living strip asks for nothing; it catches up when uncovered.
        let living = (!self.gate.occluded() && self.view.is_some())
            .then(|| self.scene.as_ref().and_then(Scene::due))
            .flatten();
        // Open windows keep their own deadlines, covered or not, and so does the egui probe.
        let windows = self.shell.deadline();
        #[cfg(feature = "dev")]
        let dev = self.dev.deadline();
        #[cfg(not(feature = "dev"))]
        let dev = None;
        let deadline = [self.raise.deadline(), living, windows, dev]
            .into_iter()
            .flatten()
            .min();
        event_loop.set_control_flow(match deadline {
            Some(deadline) => ControlFlow::WaitUntil(deadline),
            None => ControlFlow::Wait,
        });
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.end_probe(EndReason::WindowClosed);
        if let Some(engine) = self.repo.as_mut().and_then(|repo| repo.engine.take()) {
            engine.shutdown();
        }
        // winit only posts its window's destruction, which the ended loop never handles, so
        // the reserved edge is given back here.
        if let Some(view) = &self.view {
            view.pinning.release();
        }
    }
}

impl Strip {
    /// Handles `event` for one of the shell's windows, and returns what they asked for.
    #[cfg(not(feature = "dev"))]
    fn shell_window_event(&mut self, id: WindowId, event: &WindowEvent) -> Vec<Intent> {
        self.shell.window_event(id, event, &mut |_| {})
    }

    /// Handles `event` for one of the shell's windows, the debug window among them, and
    /// returns what they asked for.
    #[cfg(feature = "dev")]
    fn shell_window_event(&mut self, id: WindowId, event: &WindowEvent) -> Vec<Intent> {
        let now = Instant::now();
        // Only the debug window's repaint shows the readout, and reading memory costs a call
        // into the OS.
        let debug = self
            .shell
            .window(WindowKind::Debug)
            .is_some_and(|window| window.id() == id);
        let readout = if debug && matches!(event, WindowEvent::RedrawRequested) {
            self.readout(now)
        } else {
            None
        };
        let dev = &mut self.dev;
        let intents = self
            .shell
            .window_event(id, event, &mut |ui| dev.draw(ui, readout.as_ref()));
        if let Some((feed, overrides)) = self.dev.take_change()
            && let Some(scene) = &mut self.scene
        {
            scene.choose(feed, overrides, now);
            self.sync_menu();
        }
        intents
    }
}

#[cfg(feature = "dev")]
impl Strip {
    /// The loop woke: the egui probe takes its step.
    fn dev_woke(&mut self, event_loop: &ActiveEventLoop, now: Instant) {
        self.dev.woke();
        if !self.dev.probe_due(now) {
            return;
        }
        let Some((feed, overrides)) = self.scene.as_ref().and_then(Scene::shown) else {
            return;
        };
        let feed = feed.clone();
        if self
            .dev
            .probe(&mut self.shell, event_loop, now, (&feed, overrides))
        {
            event_loop.exit();
        }
    }

    fn readout(&mut self, now: Instant) -> Option<Readout> {
        let frames = self.gate.count();
        let fps = self.dev.fps.per_second(now);
        let Some(Scene::Living {
            living,
            feeder,
            origin,
            ..
        }) = &self.scene
        else {
            return None;
        };
        let overlay = feeder.overlay();
        Some(Readout {
            frames,
            fps,
            compositions: living.compositions(),
            memory: terrarium_platform::memory::usage(),
            state: feeder.scene().presentation.state,
            clock: overlay.clock,
            dimmed: overlay.dim,
            stage: feeder.stage(Scene::millis(*origin, now)),
            sim_day: feeder.sim_day(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use terrarium_adapters::ci::MemoryKeychain;
    use terrarium_platform::login::LoginError;
    use terrarium_store::{RepoLocation, TerrariumId};
    use terrarium_truth::RepoSeed;

    use super::*;
    use crate::instance::Claim;
    use crate::shell::Intent;

    #[test]
    fn the_machine_waking_wakes_the_engine_and_the_screens_waking_only_takes_the_time() {
        assert_eq!(
            waking(SystemEvent::Woke),
            Waking {
                look: true,
                engine: true
            }
        );
        assert_eq!(
            waking(SystemEvent::ScreensWoke),
            Waking {
                look: true,
                engine: false
            }
        );
        assert_eq!(
            waking(SystemEvent::ScreensChanged),
            Waking {
                look: false,
                engine: false
            }
        );
    }

    /// A login item that records what it is asked.
    #[derive(Default)]
    struct Asked(Vec<bool>);

    impl LoginItem for Asked {
        fn set(&mut self, on: bool) -> Result<(), LoginError> {
            self.0.push(on);
            Ok(())
        }
    }

    /// A run with a data directory in `dir` and nothing connected.
    fn repo(dir: &Path) -> Repo {
        let data = DataDir::at(dir.join("data"));
        let Ok(Claim::First(instance)) = Instance::claim(&data) else {
            panic!("the claim");
        };
        // Never the person's keychain.
        let keychain: Arc<dyn Keychain> = Arc::new(MemoryKeychain::default());
        let writer = Writer::start(
            data.clone(),
            Box::new(Asked::default()),
            || terrarium_platform::login::LoginStatus::Unavailable,
            Arc::clone(&keychain),
            Box::new(|_| {}),
        );
        Repo {
            data,
            _instance: instance,
            _requests: None,
            engine: None,
            generation: 1,
            connected: None,
            first_run: true,
            pending_login: None,
            writer,
            keychain,
        }
    }

    /// An engine with nothing connected, which does nothing.
    fn engine(dir: &Path) -> Engine {
        let config = terrarium_engine::EngineConfig::new(DataDir::at(dir.join("engine")), None);
        Engine::spawn(config, Box::new(|_| {})).unwrap().0
    }

    fn terrarium(salt: u8) -> Box<Terrarium> {
        Box::new(Terrarium {
            id: TerrariumId::new(1),
            seed: RepoSeed::from_bytes([1; 32]),
            repo: RepoLocation {
                path: "/projects/moss".into(),
                name: "moss".into(),
                default_branch: "main".into(),
            },
            install_salt: [salt; 16],
            connected_at: 0,
            active: true,
        })
    }

    fn connected(dir: &Path, repo: &mut Repo, shell: &mut Shell) {
        let outcome = Outcome::Connected {
            engine: engine(dir),
            terrarium: terrarium(3),
            scene: None,
            report: "Found\n".into(),
        };
        repo.changed(1, outcome, shell, &mut Asked::default());
    }

    #[test]
    fn a_connect_opens_the_strip_by_its_salt_shows_the_report_and_applies_the_login_choice() {
        let dir = tempfile::tempdir().unwrap();
        let mut repo = repo(dir.path());
        let mut shell = Shell::default();
        shell.connect = Some(ConnectForm::new(true));
        repo.pending_login = Some(true);
        let mut asked = Asked::default();
        let outcome = Outcome::Connected {
            engine: engine(dir.path()),
            terrarium: terrarium(3),
            scene: None,
            report: "Found\n".into(),
        };
        let effect = repo.changed(1, outcome, &mut shell, &mut asked);
        assert_eq!(effect, Effect::Open(InstallSalt::new([3; 16])));
        assert_eq!(asked.0, [true]);
        assert!(!repo.first_run && repo.pending_login.is_none());
        assert!(repo.engine.is_some());
        assert_eq!(repo.facts().map(|facts| facts.name), Some("moss".into()));
        // The window shows the report: "Done" means something now.
        let form = shell.connect.as_mut().unwrap();
        form.done();
        assert_eq!(form.take_intent(), Some(Intent::Done));
    }

    #[test]
    fn a_failed_connect_is_said_in_the_window_and_connects_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut repo = repo(dir.path());
        let mut shell = Shell::default();
        let mut form = ConnectForm::new(true);
        form.picked(Some("/plain".into()));
        form.connect();
        form.take_intent();
        shell.connect = Some(form);
        repo.pending_login = Some(true);
        let mut asked = Asked::default();
        let outcome = Outcome::NotConnected(ConnectError::OtherRepository);
        assert_eq!(
            repo.changed(1, outcome, &mut shell, &mut asked),
            Effect::Nothing
        );
        assert!(asked.0.is_empty(), "nothing registered");
        assert!(repo.connected.is_none() && repo.pending_login.is_none());
        // No longer busy: another try can follow.
        let form = shell.connect.as_mut().unwrap();
        form.connect();
        assert!(form.take_intent().is_some());
    }

    #[test]
    fn a_restart_shows_the_live_view_and_lays_out_anew_only_on_a_new_salt() {
        let dir = tempfile::tempdir().unwrap();
        let mut repo = repo(dir.path());
        let mut shell = Shell::default();
        connected(dir.path(), &mut repo, &mut shell);
        let facts = repo.facts().unwrap();
        let mut form = SettingsForm::new(facts, &Prefs::default(), String::new());
        form.repo().ask_reseed();
        form.repo().reseed();
        form.take_intents();
        shell.settings = Some(form);
        let restart = |salt: u8| Outcome::Restarted {
            changed: Ok(lifecycle::Changed::Terrarium),
            started: Ok((engine(dir.path()), terrarium(salt))),
        };
        let mut asked = Asked::default();
        assert_eq!(
            repo.changed(1, restart(3), &mut shell, &mut asked),
            Effect::Show {
                reseeded: None,
                history_changed: true
            },
            "relinked: the same salt"
        );
        assert_eq!(
            repo.changed(1, restart(9), &mut shell, &mut asked),
            Effect::Show {
                reseeded: Some(InstallSalt::new([9; 16])),
                history_changed: true
            },
            "re-seeded"
        );
        assert_eq!(
            repo.connected.as_ref().map(|c| c.terrarium.install_salt),
            Some([9; 16])
        );
        // The form is free again.
        let form = shell.settings.as_mut().unwrap();
        form.repo().ask_reseed();
        form.repo().reseed();
        assert_eq!(form.take_intents(), [Intent::Reseed]);
        // An engine that cannot start again leaves the strip halted.
        let failed = Outcome::Restarted {
            changed: Ok(lifecycle::Changed::Terrarium),
            started: Err(ConnectError::Halted),
        };
        assert_eq!(
            repo.changed(1, failed, &mut shell, &mut asked),
            Effect::Show {
                reseeded: None,
                history_changed: true
            }
        );
        let live = &repo.connected.as_ref().unwrap().live;
        assert_eq!(live.label().last().map(String::as_str), Some("halted"));
    }

    #[test]
    fn an_import_says_what_it_did_and_dates_the_time_lapse_only_when_it_added_any() {
        use terrarium_store::{ImportError, ImportReport};
        use terrarium_truth::SIM_VERSION;

        let dir = tempfile::tempdir().unwrap();
        let mut repo = repo(dir.path());
        let mut shell = Shell::default();
        connected(dir.path(), &mut repo, &mut shell);
        let facts = repo.facts().unwrap();
        shell.settings = Some(SettingsForm::new(facts, &Prefs::default(), String::new()));
        let imported = |inserted, duplicates| {
            Ok(lifecycle::Changed::Imported(ImportReport {
                sim_version: SIM_VERSION,
                inserted,
                duplicates,
                conflicted: 0,
                conflicts: vec![],
            }))
        };
        let mut asked = Asked::default();
        for (changed, history_changed) in [
            (imported(0, 4), false),
            (imported(3, 1), true),
            (
                Err(lifecycle::Refused::Import(ImportError::NotAnExport)),
                false,
            ),
            (Err(lifecycle::Refused::ImportHalted), false),
        ] {
            let outcome = Outcome::Restarted {
                changed,
                started: Ok((engine(dir.path()), terrarium(3))),
            };
            assert_eq!(
                repo.changed(1, outcome, &mut shell, &mut asked),
                Effect::Show {
                    reseeded: None,
                    history_changed
                }
            );
            assert!(repo.engine.is_some(), "the engine runs again");
        }
        // An engine that cannot start again after rows were added still dates the
        // time-lapse, the second time too, when the strip is halted already.
        for _ in 0..2 {
            let outcome = Outcome::Restarted {
                changed: imported(3, 0),
                started: Err(ConnectError::Halted),
            };
            assert_eq!(
                repo.changed(1, outcome, &mut shell, &mut asked),
                Effect::Show {
                    reseeded: None,
                    history_changed: true
                }
            );
        }
        // The form is free again after each.
        let form = shell.settings.as_mut().unwrap();
        form.repo().ask_reseed();
        form.repo().reseed();
        assert_eq!(form.take_intents(), [Intent::Reseed]);
    }

    #[test]
    fn a_disconnect_closes_the_strip_and_forgets_the_repository() {
        let dir = tempfile::tempdir().unwrap();
        let mut repo = repo(dir.path());
        let mut shell = Shell::default();
        connected(dir.path(), &mut repo, &mut shell);
        shell.settings = repo
            .facts()
            .map(|facts| SettingsForm::new(facts, &Prefs::default(), String::new()));
        assert_eq!(
            repo.changed(1, Outcome::Disconnected, &mut shell, &mut Asked::default()),
            Effect::Close
        );
        assert!(repo.connected.is_none() && repo.engine.is_none());
        assert!(shell.settings.is_none() && shell.connect.is_none());
    }

    #[test]
    fn a_change_outrun_by_a_later_one_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut repo = repo(dir.path());
        let mut shell = Shell::default();
        repo.generation = 2;
        let outcome = Outcome::Connected {
            engine: engine(dir.path()),
            terrarium: terrarium(3),
            scene: None,
            report: String::new(),
        };
        assert_eq!(
            repo.changed(1, outcome, &mut shell, &mut Asked::default()),
            Effect::Nothing
        );
        assert!(repo.connected.is_none() && repo.engine.is_none());
    }
}

#[cfg(test)]
mod pointer_tests {
    use std::time::Duration as Wait;

    use terrarium_engine::schedule::Cause;
    use terrarium_engine::{Branch, Details, News, RefreshStats, Replay, Snapshot};
    use terrarium_truth::{Basis, RepoSeed, SimState, TickGrid};

    use super::*;

    /// A connected repository's view after one snapshot.
    fn view() -> LiveView {
        let state = SimState::new(Basis::new(
            RepoSeed::from_bytes([1; 32]),
            TickGrid::from_origin(0).unwrap(),
            "main",
        ));
        let mut view = LiveView::new("moss".into(), None);
        view.apply(Update::Snapshot(Box::new(Snapshot {
            scene: terrarium_truth::project(&state),
            report: state.report(),
            details: Details::default(),
            branch: Branch::Named("main".into()),
            notes: News::default(),
            stats: RefreshStats {
                cause: Cause::Connect,
                replay: Replay::Full,
                working: true,
                adapters_to_snapshot: Wait::ZERO,
            },
        })));
        view
    }

    fn shown(scene: &Scene) -> (&Feeder, &Living) {
        let Scene::Living { feeder, living, .. } = scene else {
            panic!("a living scene");
        };
        (feeder, living)
    }

    #[cfg(feature = "dev")]
    #[test]
    fn dim_mode_closes_a_pinned_panel() {
        let view = view();
        let mut scene = Scene::new(
            &Mode::Live(Feed::Live),
            Some((&view, preset::SALT)),
            &Prefs::default(),
        );
        let now = Instant::now();
        scene.point(
            Pointer::Moved(Some(ViewportPoint { x: 300, y: 70 })),
            &view,
            now,
        );
        scene.point(Pointer::Pressed, &view, now);
        let Scene::Living { feeder, soil, .. } = &mut scene else {
            panic!("a living scene");
        };
        assert!(feeder.hover().panel.is_some());
        // Dimmed or not as the test says, whatever this machine's own idle time.
        let dim = |on| Overrides {
            dim: Some(on),
            ..Overrides::default()
        };
        feeder.choose(Feed::Live, dim(false), Millis(1000), &Os::read());
        let was = feeder.dim();
        Scene::dimmed(feeder, soil, was);
        assert!(feeder.hover().panel.is_some(), "not dimmed, it stays");
        feeder.choose(Feed::Live, dim(true), Millis(2000), &Os::read());
        Scene::dimmed(feeder, soil, was);
        assert_eq!(feeder.hover(), &Hover::default());
        // Closed, it shows nothing however the pointer last was.
        let catalog = &Art::embedded().catalog;
        assert_eq!(soil.hover(view.panel(), 0, catalog, 640), Hover::default());
    }

    #[test]
    fn the_pointer_raises_the_panel_it_hit_tests_and_the_strip_composes_it() {
        let view = view();
        let mut scene = Scene::new(
            &Mode::Live(Feed::Live),
            Some((&view, preset::SALT)),
            &Prefs::default(),
        );
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let point = |x, y| Pointer::Moved(Some(ViewportPoint { x, y }));
        scene.point(point(300, 70), &view, at(10));
        let panel = shown(&scene).0.hover().panel.clone().expect("raised");
        assert_eq!(panel.page.title, copy::panel::CLIMATE);
        assert_eq!(shown(&scene).0.overlay().hover.panel, Some(panel.clone()));
        // The strip has a frame to make while it rises.
        assert!(shown(&scene).1.due().is_some());
        // Its rows are hit where they are drawn: the light row opens a page, and a click
        // there shows it.
        let (feeder, living) = shown(&scene);
        let drawn = panel::layout(
            living.catalog(),
            &panel.page,
            panel.anchor,
            living.columns(),
        )
        .expect("a layout");
        assert_eq!(feeder.hover().panel.as_ref().unwrap().anchor, 300);
        let light = drawn.rows[0];
        scene.point(point(light.x + 1, light.y + 2), &view, at(400));
        let pointed = shown(&scene).0.hover().panel.clone().unwrap();
        assert_eq!(pointed.pointer, Some(panel::Target::Row(0)));
        scene.point(Pointer::Pressed, &view, at(500));
        let opened = shown(&scene).0.hover().panel.clone().unwrap();
        assert_eq!(opened.page.title, copy::panel::UNCOVERED_LINES);
        // Pinned, it stays when the pointer leaves; a click off it closes it.
        scene.point(Pointer::Left, &view, at(600));
        assert!(shown(&scene).0.hover().panel.is_some());
        scene.point(point(300, 10), &view, at(700));
        scene.point(Pointer::Pressed, &view, at(800));
        assert_eq!(shown(&scene).0.hover(), &Hover::default());
    }
}
