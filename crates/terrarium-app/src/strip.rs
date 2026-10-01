//! The strip: a borderless window along one edge of a monitor, below ordinary windows, showing
//! the composed frame at an integer scale.
//!
//! The loop sleeps in `ControlFlow::Wait` and wakes only for OS events, for what other threads
//! forward through the `EventLoopProxy` (hotkeys, menu choices, system notifications and probe
//! answers), and at three deadlines: a raise ending, the living strip's next change, which it
//! never asks for sooner than a fifteenth of a second after the last frame (I-10), and the
//! feed's next look at the clock, the idle time or `--demo`'s cycle. Nothing is drawn, and
//! nothing asked for, while the strip is covered.

use std::rc::Rc;
use std::time::{Duration, Instant};

use global_hotkey::GlobalHotKeyEvent;
use muda::MenuId;
use terrarium_compose::catalog::Art;
use terrarium_compose::compositor::composite;
use terrarium_compose::frame::{NATIVE_HEIGHT, Rgba, Rgba640x74};
use terrarium_compose::scale::{SizeStep, StripScale};
use terrarium_compose::tween::Millis;
use terrarium_compose::variants::ArtTable;
use terrarium_compose::{sample, world};
use terrarium_platform::SystemEvent;
use terrarium_platform::geometry::Edge;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::error::{EventLoopError, OsError};
use winit::event::{ElementState, MouseButton, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::{Window, WindowId};

use crate::animate::Living;
use crate::cli::ProbeKind;
use crate::copy;
#[cfg(feature = "dev")]
use crate::debug::Readout;
#[cfg(feature = "dev")]
use crate::dev::Dev;
#[cfg(feature = "dev")]
use crate::feed::Overrides;
use crate::feed::{Feed, Feeder, Os};
use crate::hotkey::{self, RaiseHotkey};
use crate::hover::HoverTracker;
use crate::menu::{self, StripMenu};
use crate::monitor;
use crate::pinning::{self, AttachError, DEFAULT_LEVEL, LevelMode, Monitor, Observers, Pinning};
use crate::present::{FrameGate, PresentError, Presenter, SoftbufferPresenter};
use crate::probe::report::{Activity, EndReason, ProbeEvent, ScaleReport};
use crate::probe::{Advance, Launch, Probe};
use crate::raise::RaiseTimer;

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
        frame: Rgba640x74,
        padding: Box<[Rgba; NATIVE_HEIGHT as usize]>,
    },
    Living {
        living: Box<Living>,
        feeder: Box<Feeder>,
        /// The moment the composer's clock counts from.
        origin: Instant,
    },
}

impl Scene {
    fn new(mode: &Mode) -> Scene {
        let feed = match mode {
            Mode::Live(feed) => feed.clone(),
            #[cfg(feature = "dev")]
            Mode::Probe(ProbeKind::Egui) => Feed::Thriving,
            Mode::Probe(ProbeKind::Pinning) => {
                let art = Art::embedded();
                // `cargo xtask verify` checks the embedded art is complete, so this table
                // builds.
                let table =
                    ArtTable::new(&art.catalog).expect("the embedded art depicts every variant");
                let frame = sample::frame(art, &table, copy::sample_label(), 0, 0);
                return Scene::Still {
                    frame: composite(art, &frame),
                    padding: Box::new(world::padding(art, 0, 0)),
                };
            }
        };
        let feeder = Box::new(Feeder::new(feed, Millis(0), &Os::read()));
        Scene::Living {
            living: Box::new(Living::new(feeder.scene(), &feeder.overlay(), Millis(0))),
            feeder,
            origin: Instant::now(),
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

    fn pixels(&self) -> (&Rgba640x74, &[Rgba; NATIVE_HEIGHT as usize]) {
        match self {
            Scene::Still { frame, padding } => (frame, padding),
            Scene::Living { living, .. } => (living.image(), living.padding()),
        }
    }
}

/// What other threads send the event loop.
#[derive(Debug)]
pub enum UserEvent {
    HotKey(GlobalHotKeyEvent),
    Menu(MenuId),
    System(SystemEvent),
    ProbeLine(String),
    ProbeEnd,
}

/// Why the strip could not run. Its wording is in the copy module.
#[derive(Debug)]
pub enum StripError {
    EventLoop(EventLoopError),
    Window(OsError),
    Attach(AttachError),
    Present(PresentError),
    NoDisplay,
}

impl From<EventLoopError> for StripError {
    fn from(err: EventLoopError) -> Self {
        Self::EventLoop(err)
    }
}

/// Runs the strip until it is quit. With a probe, it also records and reports.
pub fn run(mode: &Mode) -> Result<(), StripError> {
    let pinning_probe = matches!(mode, Mode::Probe(ProbeKind::Pinning));
    let mut builder = EventLoop::<UserEvent>::with_user_event();
    pinning::configure_event_loop(&mut builder, pinning_probe);
    let event_loop = builder.build()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    // Both crates keep only the first handler they are given, so these come before any menu
    // or hotkey exists. Each consumer gets its own proxy, made once.
    menu::forward_events(event_loop.create_proxy());
    hotkey::forward_events(event_loop.create_proxy());
    let probe = pinning_probe.then(|| Probe::start(event_loop.create_proxy()));
    let mut strip = Strip::new(event_loop.create_proxy(), probe, mode);
    event_loop.run_app(&mut strip)?;
    match strip.failed {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

struct Strip {
    /// Given to the system observers when the window opens. On macOS every proxy is a run-loop
    /// source, so none is kept once it is handed on.
    proxy: Option<EventLoopProxy<UserEvent>>,
    probe: Option<Probe>,
    view: Option<View>,
    scene: Scene,
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
    #[cfg(feature = "dev")]
    dev: Dev,
}

/// Everything that exists once the window does.
struct View {
    window: Rc<Window>,
    pinning: Pinning,
    presenter: SoftbufferPresenter,
    menu: Option<StripMenu>,
    hotkey: Option<RaiseHotkey>,
    _observers: Observers,
    scale: StripScale,
}

impl Strip {
    fn new(proxy: EventLoopProxy<UserEvent>, probe: Option<Probe>, mode: &Mode) -> Self {
        let level = probe.as_ref().map_or(DEFAULT_LEVEL, Probe::level);
        let scene = Scene::new(mode);
        Self {
            proxy: Some(proxy),
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
        let menu = StripMenu::new()
            .inspect_err(|err| eprintln!("{}", copy::menu_unavailable(err)))
            .ok();
        let hotkey = match RaiseHotkey::register() {
            Ok(hotkey) => Some(hotkey),
            Err(err) => {
                eprintln!("{}", copy::hotkey_unavailable(&err));
                self.record(ProbeEvent::HotkeyUnavailable {
                    error: err.to_string(),
                });
                None
            }
        };
        let proxy = self.proxy.take();
        let observers = pinning.observe(move |event| {
            if let Some(proxy) = &proxy {
                let _ = proxy.send_event(UserEvent::System(event));
            }
        });

        let monitors = pinning.monitors(&window);
        // Only the probe reports every monitor.
        let reported = self
            .probe
            .is_some()
            .then(|| monitors.iter().map(|m| m.info.clone()).collect());
        let monitor = choose(monitors).ok_or(StripError::NoDisplay)?;
        if let Some(probe) = &mut self.probe {
            probe.launched(Launch {
                os: pinning::os_version(),
                separate_spaces: pinning::separate_spaces(),
                monitors: reported.unwrap_or_default(),
                chosen: Some(monitor.info.id.clone()),
            });
        }
        let mut view = View {
            window,
            pinning,
            presenter,
            menu,
            hotkey,
            _observers: observers,
            // Until the placement says otherwise: on Windows the reserved band can be narrower.
            scale: StripScale::for_physical_width(monitor.info.physical_width, SIZE_STEP),
        };
        self.place(&mut view, &monitor);
        self.apply_level(&view);
        view.pinning.show(&view.window);
        view.window.request_redraw();
        let active = pinning::app_is_active();
        self.record(ProbeEvent::AppActive {
            at: "window shown",
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
                let changed = self.scene.advance(now);
                // The OS's own requests always present; the schedule's only on a change.
                let wanted = changed || !std::mem::take(&mut self.scheduled);
                if wanted && self.gate.should_present() {
                    let (frame, padding) = self.scene.pixels();
                    match view.presenter.present(frame, &view.scale, padding) {
                        Ok(()) => {
                            self.gate.presented();
                            #[cfg(feature = "dev")]
                            self.dev.fps.presented(now);
                        }
                        Err(err) => eprintln!("{}", copy::present_failed(&err)),
                    }
                }
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
                let living = matches!(self.scene, Scene::Living { .. });
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
                    && let Some(menu) = &view.menu
                {
                    view.pinning.show_menu(&view.window, menu.menu());
                }
            }
            _ => {}
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
        #[cfg(feature = "dev")]
        self.dev_woke(event_loop, now);
        #[cfg(not(feature = "dev"))]
        let _ = event_loop;
        if !self.gate.occluded() {
            self.scene.look(now, false);
        }
        if matches!(cause, StartCause::ResumeTimeReached { .. })
            && !self.gate.occluded()
            && self.scene.frame_due().is_some_and(|due| due <= now)
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

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::HotKey(event) => self.with_view(|strip, view| {
                if view.hotkey.as_ref().is_some_and(|h| h.is_pressed(event)) {
                    // A reserved edge stays topmost, so there is nothing to restore later.
                    if !pinning::EDGE_RESERVED {
                        strip.raise.press(Instant::now());
                    }
                    view.pinning.raise(&view.window);
                    strip.record(ProbeEvent::Raised {
                        value: view.pinning.level(),
                    });
                }
            }),
            UserEvent::Menu(id) => {
                let quit = self
                    .view
                    .as_ref()
                    .and_then(|view| view.menu.as_ref())
                    .is_some_and(|menu| menu.is_quit(&id));
                if quit {
                    self.end_probe(EndReason::Quit);
                    event_loop.exit();
                }
                #[cfg(feature = "dev")]
                if self
                    .view
                    .as_ref()
                    .and_then(|view| view.menu.as_ref())
                    .is_some_and(|menu| menu.is_debug(&id))
                    && let Some((feed, overrides)) = self.scene.shown()
                {
                    let feed = feed.clone();
                    self.dev.open_debug(event_loop, &feed, overrides);
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
                // monotonic clock stops while asleep: the feed takes the local time again.
                if event == SystemEvent::Woke {
                    self.scene.look(Instant::now(), true);
                }
            }
            UserEvent::ProbeLine(line) => {
                self.with_view(|strip, view| strip.answer(event_loop, view, &line));
            }
            UserEvent::ProbeEnd => {
                self.end_probe(EndReason::InputClosed);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        // Only the strip's own window quits the app when closed; other windows are egui's.
        if self
            .view
            .as_ref()
            .is_some_and(|view| view.window.id() == id)
        {
            self.with_view(|strip, view| strip.window_input(event_loop, view, event));
        } else {
            #[cfg(feature = "dev")]
            self.dev_window_event(id, &event);
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
                at: "first idle",
                active,
            });
        }
        // While covered, the living strip asks for nothing; it catches up when uncovered.
        let living = (!self.gate.occluded() && self.view.is_some())
            .then(|| self.scene.due())
            .flatten();
        // Open egui windows and the egui probe keep their own deadlines, covered or not.
        #[cfg(feature = "dev")]
        let dev = self.dev.deadline();
        #[cfg(not(feature = "dev"))]
        let dev = None;
        let deadline = [self.raise.deadline(), living, dev]
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
        // winit only posts its window's destruction, which the ended loop never handles, so
        // the reserved edge is given back here.
        if let Some(view) = &self.view {
            view.pinning.release();
        }
    }
}

#[cfg(feature = "dev")]
impl Strip {
    /// The loop woke: egui windows due a repaint redraw, and the egui probe takes its step.
    fn dev_woke(&mut self, event_loop: &ActiveEventLoop, now: Instant) {
        self.dev.woke(now);
        if !self.dev.probe_due(now) {
            return;
        }
        let Some((feed, overrides)) = self.scene.shown() else {
            return;
        };
        let feed = feed.clone();
        if self.dev.probe(event_loop, now, (&feed, overrides)) {
            event_loop.exit();
        }
    }

    fn dev_window_event(&mut self, id: WindowId, event: &WindowEvent) {
        if !self.dev.owns(id) {
            return;
        }
        let now = Instant::now();
        // Only a repaint shows the readout, and reading memory costs a call into the OS.
        let readout = if matches!(event, WindowEvent::RedrawRequested) {
            self.readout(now)
        } else {
            None
        };
        if let Some((feed, overrides)) = self.dev.window_event(id, event, readout.as_ref()) {
            self.scene.choose(feed, overrides, now);
        }
    }

    fn readout(&mut self, now: Instant) -> Option<Readout> {
        let frames = self.gate.count();
        let fps = self.dev.fps.per_second(now);
        let Scene::Living {
            living,
            feeder,
            origin,
        } = &self.scene
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
        })
    }
}
