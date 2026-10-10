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

use std::path::PathBuf;
use std::rc::Rc;
use std::thread;
use std::time::{Duration, Instant};

use global_hotkey::GlobalHotKeyEvent;
use muda::MenuId;
use terrarium_compose::catalog::Art;
use terrarium_compose::compositor::composite;
use terrarium_compose::frame::{NativeImage, Viewport};
use terrarium_compose::salt::InstallSalt;
use terrarium_compose::scale::{SizeStep, StripScale};
use terrarium_compose::tween::Millis;
use terrarium_compose::variants::ArtTable;
use terrarium_compose::{preset, sample};
use terrarium_engine::{ConnectError, Engine, EngineConfig, Start, Update};
use terrarium_platform::SystemEvent;
use terrarium_platform::geometry::Edge;
use terrarium_store::{DataDir, Terrarium};
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
use crate::hotkey::{self, RaiseHotkey};
use crate::hover::HoverTracker;
use crate::instance::{Instance, RequestWatch};
use crate::lifecycle::{self, Job, Outcome};
use crate::live::{Changed, Launch, LiveView};
use crate::login::{self, LoginItem};
use crate::menu::{self, StripMenu};
use crate::monitor;
use crate::pinning::{self, AttachError, DEFAULT_LEVEL, LevelMode, Monitor, Observers, Pinning};
use crate::present::{FrameGate, PresentError, Presenter, SoftbufferPresenter};
use crate::probe::report::{Activity, EndReason, ProbeEvent, ScaleReport};
use crate::probe::{Advance, Launch as ProbeLaunch, Probe};
use crate::raise::RaiseTimer;
use crate::settings::{RepoFacts, SettingsForm};
use crate::shell::{Intent, Picking, Shell};

/// Until settings exist, the strip uses the default edge and size.
const EDGE: Edge = Edge::Bottom;
const SIZE_STEP: SizeStep = SizeStep::Full;

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
    },
}

impl Scene {
    /// The scene for `mode`; a connected repository's starts from `live`, laid out by its salt.
    fn new(mode: &Mode, live: Option<(&LiveView, InstallSalt)>) -> Scene {
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
        let feeder = Box::new(feeder);
        Scene::Living {
            living: Box::new(Living::new(
                feeder.scene(),
                &feeder.overlay(),
                salt,
                Millis(0),
            )),
            feeder,
            origin: Instant::now(),
        }
    }

    /// Moves the composer toward what the connected repository now shows.
    fn show(&mut self, view: &LiveView, now: Instant) {
        if let Scene::Living {
            living,
            feeder,
            origin,
        } = self
        {
            let at = Self::millis(*origin, now);
            feeder.show(view.scene(), view.label(), at, &Os::read());
            living.update(feeder.scene(), &feeder.overlay(), at);
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
        } = self
        {
            let at = Self::millis(*origin, now);
            if (force || feeder.due() <= at) && feeder.tick(at, &Os::read(), force) {
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
        } = self
        {
            let at = Self::millis(*origin, now);
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
    /// An update from the engine of that generation.
    Engine(u64, Update),
    /// A second launch asked for the settings.
    ShowSettings,
    /// The folder picker closed, with the folder chosen, if one was.
    Picked(Picking, Option<PathBuf>),
    /// A lifecycle change of that generation is done.
    Lifecycle(u64, Box<Outcome>),
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
    let repo = launch
        .map(|launch| Repo::start(launch, &proxy))
        .transpose()?;
    let mut strip = Strip::new(proxy, probe, mode, repo);
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
}

/// What a lifecycle change leaves the strip to do.
#[derive(Debug, PartialEq, Eq)]
enum Effect {
    Nothing,
    /// Connected: open the strip, laid out by this salt.
    Open(InstallSalt),
    /// The engine started again: show what the live view says, laid out anew by the salt
    /// given (a re-seed).
    Show(Option<InstallSalt>),
    /// Disconnected: close the strip, and open the connect window.
    Close,
}

/// The repository connected, and what the strip shows of it.
struct Connected {
    terrarium: Box<Terrarium>,
    live: LiveView,
}

impl Repo {
    /// Connects what `launch` names, and watches for a second launch's requests.
    fn start(launch: Launch, proxy: &EventLoopProxy<UserEvent>) -> Result<Repo, StripError> {
        let Launch {
            data,
            repo,
            instance,
        } = launch;
        let config = EngineConfig::new(data.clone(), repo.clone());
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
                }),
            ),
            Start::NotConnected => (None, None),
        };
        Ok(Repo {
            first_run: lifecycle::first_run(&data),
            data,
            _instance: instance,
            _requests: requests,
            engine,
            generation: 0,
            connected,
            pending_login: None,
        })
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
                            effect = Effect::Show(reseeded);
                        }
                        Err(err) => {
                            eprintln!("{}", copy::connect_failed(None, &err));
                            if connected.live.apply(Update::Halted) != Changed::Nothing {
                                effect = Effect::Show(None);
                            }
                        }
                    }
                }
                let facts = self.facts();
                if let Some(form) = &mut shell.settings {
                    if let Some(facts) = facts {
                        form.set_repo(facts);
                    }
                    form.finished(changed.err().map(|err| copy::connect_problem(&err)));
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
        })
    }
}

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
    idle_seen: bool,
    failed: Option<StripError>,
    /// Made with the first strip window, and kept while the app runs.
    menu: Option<StripMenu>,
    hotkey: Option<RaiseHotkey>,
    /// A run with a data directory.
    repo: Option<Repo>,
    /// The windows beside the strip.
    shell: Shell,
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
            )),
            None => Some(Scene::new(mode, None)),
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
            idle_seen: false,
            failed: None,
            menu: None,
            hotkey: None,
            repo,
            shell: Shell::default(),
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
        if self.hotkey.is_none() {
            match RaiseHotkey::register() {
                Ok(hotkey) => self.hotkey = Some(hotkey),
                Err(err) => {
                    eprintln!("{}", copy::hotkey_unavailable(&err));
                    self.record(ProbeEvent::HotkeyUnavailable {
                        error: err.to_string(),
                    });
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
        let monitor = choose(monitors).ok_or(StripError::NoDisplay)?;
        if let Some(probe) = &mut self.probe {
            probe.launched(ProbeLaunch {
                os: pinning::os_version(),
                separate_spaces: pinning::separate_spaces(),
                monitors: reported.unwrap_or_default(),
                chosen: Some(monitor.info.id.clone()),
            });
        }
        let mut view = View {
            _observers: observers,
            pinning,
            presenter,
            window,
            // Until the placement says otherwise: on Windows the reserved band can be narrower.
            scale: StripScale::for_physical_width(monitor.info.physical_width, SIZE_STEP),
        };
        self.place(&mut view, &monitor);
        self.apply_level(&view);
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
        let Some((scale, frame)) = view.pinning.place(&view.window, monitor, SIZE_STEP, EDGE)
        else {
            return;
        };
        view.scale = scale;
        self.record(ProbeEvent::Placed {
            monitor: monitor.info.id.clone(),
            strip: ScaleReport::from(&view.scale),
            frame,
        });
    }

    /// Chooses the monitor again and moves the strip there, after a display change, a wake or
    /// a moved taskbar.
    fn replace(&mut self, view: &mut View) {
        if let Some(monitor) = choose(view.pinning.monitors(&view.window)) {
            self.place(view, &monitor);
            view.window.request_redraw();
        }
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
        match probe.answer(note, frames) {
            Advance::Done => event_loop.exit(),
            Advance::Next(level) if level != self.level => {
                self.level = level;
                // A raise in progress keeps the strip up; the restore applies the new level.
                if self.raise.deadline().is_none() {
                    self.apply_level(view);
                }
            }
            Advance::Next(_) => {}
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
                    choose(view.pinning.monitors(&view.window)).map(|monitor| {
                        StripScale::for_physical_width(monitor.info.physical_width, SIZE_STEP)
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
            }
            WindowEvent::CursorMoved { position, .. } => {
                let point = self.hover.moved(position.x, position.y, &view.scale);
                if let Some(probe) = &mut self.probe {
                    probe.moved(point, pinning::app_is_active());
                }
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
            }
            _ => {}
        }
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
            let form = self
                .shell
                .settings
                .get_or_insert_with(|| SettingsForm::new(facts.clone()));
            form.set_repo(facts);
            form.set_missing(missing);
            self.shell.open(event_loop, WindowKind::Settings);
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
            }
        }
    }

    /// The folder picker, parented to the window that asked: its answer comes back as
    /// [`UserEvent::Picked`]. It is awaited off the loop, which keeps running meanwhile.
    fn pick(&self, picking: Picking) {
        let kind = match picking {
            Picking::Repository => WindowKind::Connect,
            Picking::Location => WindowKind::Settings,
        };
        let mut dialog = rfd::AsyncFileDialog::new();
        if let Some(window) = self.shell.window(kind) {
            dialog = dialog.set_parent(window);
        }
        let chosen = dialog.pick_folder();
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
        match repo.changed(generation, outcome, &mut self.shell, &mut login::System) {
            Effect::Nothing => {}
            Effect::Open(salt) => {
                if let Some(connected) = &repo.connected {
                    let live = Some((&connected.live, salt));
                    self.scene = Some(Scene::new(&Mode::Live(Feed::Live), live));
                }
                self.open_view(event_loop);
            }
            Effect::Show(reseeded) => {
                if let (Some(scene), Some(connected)) = (&mut self.scene, &repo.connected) {
                    match reseeded {
                        Some(salt) => scene.reseed(&connected.live, salt, now),
                        None => scene.show(&connected.live, now),
                    }
                }
            }
            Effect::Close => {
                self.close_view();
                self.scene = None;
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

/// The monitor the strip belongs on.
fn choose(mut monitors: Vec<Monitor>) -> Option<Monitor> {
    let index = monitor::choose(&monitors, None)?;
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
            UserEvent::HotKey(event) => {
                if !self.hotkey.as_ref().is_some_and(|h| h.is_pressed(event)) {
                    return;
                }
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
            UserEvent::Menu(id) => {
                let Some(menu) = &self.menu else {
                    return;
                };
                if menu.is_quit(&id) {
                    self.end_probe(EndReason::Quit);
                    event_loop.exit();
                } else if menu.is_settings(&id) {
                    self.show_settings(event_loop);
                }
                #[cfg(feature = "dev")]
                if self.menu.as_ref().is_some_and(|menu| menu.is_debug(&id))
                    && let Some((feed, overrides)) = self.scene.as_ref().and_then(Scene::shown)
                {
                    let feed = feed.clone();
                    self.dev
                        .open_debug(&mut self.shell, event_loop, &feed, overrides);
                }
            }
            UserEvent::System(event) => {
                self.record(ProbeEvent::System {
                    what: format!("{event:?}"),
                });
                if matches!(
                    event,
                    SystemEvent::ScreensChanged
                        | SystemEvent::Woke
                        | SystemEvent::ScreensWoke
                        | SystemEvent::AppBarPosChanged
                        | SystemEvent::AppBarStateChanged
                ) {
                    self.placement_pending = true;
                }
                // The composer carried the clock on through the sleep, but on macOS the
                // monotonic clock stops while asleep: the feed takes the local time again, and
                // the engine catches truth up.
                if event == SystemEvent::Woke {
                    if let Some(scene) = &mut self.scene {
                        scene.look(Instant::now(), true);
                    }
                    if let Some(engine) = self.repo.as_ref().and_then(|r| r.engine.as_ref()) {
                        engine.wake();
                    }
                }
            }
            UserEvent::ProbeLine(line) => {
                self.with_view(|strip, view| strip.answer(event_loop, view, &line));
            }
            UserEvent::ProbeEnd => {
                self.end_probe(EndReason::InputClosed);
                event_loop.exit();
            }
            UserEvent::Engine(generation, update) => self.engine_update(generation, update),
            UserEvent::ShowSettings => self.show_settings(event_loop),
            UserEvent::Picked(picking, path) => match picking {
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
            },
            UserEvent::Lifecycle(generation, outcome) => {
                self.changed(event_loop, generation, *outcome);
            }
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
        if std::mem::take(&mut self.placement_pending) {
            self.with_view(Self::replace);
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

    use terrarium_platform::login::LoginError;
    use terrarium_store::{RepoLocation, TerrariumId};
    use terrarium_truth::RepoSeed;

    use super::*;
    use crate::instance::Claim;
    use crate::shell::Intent;

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
        Repo {
            data,
            _instance: instance,
            _requests: None,
            engine: None,
            generation: 1,
            connected: None,
            first_run: true,
            pending_login: None,
        }
    }

    /// An engine with nothing connected, which does nothing.
    fn engine(dir: &Path) -> Engine {
        let config = EngineConfig::new(DataDir::at(dir.join("engine")), None);
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
        let mut form = SettingsForm::new(facts);
        form.ask_reseed();
        form.reseed();
        form.take_intent();
        shell.settings = Some(form);
        let restart = |salt: u8| Outcome::Restarted {
            changed: Ok(()),
            started: Ok((engine(dir.path()), terrarium(salt))),
        };
        let mut asked = Asked::default();
        assert_eq!(
            repo.changed(1, restart(3), &mut shell, &mut asked),
            Effect::Show(None),
            "relinked: the same salt"
        );
        assert_eq!(
            repo.changed(1, restart(9), &mut shell, &mut asked),
            Effect::Show(Some(InstallSalt::new([9; 16]))),
            "re-seeded"
        );
        assert_eq!(
            repo.connected.as_ref().map(|c| c.terrarium.install_salt),
            Some([9; 16])
        );
        // The form is free again.
        let form = shell.settings.as_mut().unwrap();
        form.ask_reseed();
        form.reseed();
        assert_eq!(form.take_intent(), Some(Intent::Reseed));
        // An engine that cannot start again leaves the strip halted.
        let failed = Outcome::Restarted {
            changed: Ok(()),
            started: Err(ConnectError::Halted),
        };
        assert_eq!(
            repo.changed(1, failed, &mut shell, &mut asked),
            Effect::Show(None)
        );
        let live = &repo.connected.as_ref().unwrap().live;
        assert_eq!(live.label().last().map(String::as_str), Some("halted"));
    }

    #[test]
    fn a_disconnect_closes_the_strip_and_forgets_the_repository() {
        let dir = tempfile::tempdir().unwrap();
        let mut repo = repo(dir.path());
        let mut shell = Shell::default();
        connected(dir.path(), &mut repo, &mut shell);
        shell.settings = repo.facts().map(SettingsForm::new);
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
