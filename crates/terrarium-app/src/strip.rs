//! The strip: a borderless window along one edge of a monitor, below ordinary windows, showing
//! the composed frame at an integer scale.
//!
//! The loop sleeps in `ControlFlow::Wait` and wakes only for OS events and for what other
//! threads forward through the `EventLoopProxy`: hotkeys, menu choices, system notifications
//! and probe answers. It waits until a deadline only while a raise is pending.

use std::rc::Rc;
use std::time::Instant;

use global_hotkey::GlobalHotKeyEvent;
use muda::MenuId;
use terrarium_compose::frame::{NATIVE_HEIGHT, Rgba, Rgba640x74};
use terrarium_compose::placeholder;
use terrarium_compose::scale::{SizeStep, StripScale};
use terrarium_platform::SystemEvent;
use terrarium_platform::geometry::Edge;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::error::{EventLoopError, OsError};
use winit::event::{ElementState, MouseButton, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::{Window, WindowId};

use crate::cli::ProbeKind;
use crate::copy;
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
pub fn run(probe: Option<ProbeKind>) -> Result<(), StripError> {
    let mut builder = EventLoop::<UserEvent>::with_user_event();
    pinning::configure_event_loop(&mut builder, probe.is_some());
    let event_loop = builder.build()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    // Both crates keep only the first handler they are given, so these come before any menu
    // or hotkey exists. Each consumer gets its own proxy, made once.
    menu::forward_events(event_loop.create_proxy());
    hotkey::forward_events(event_loop.create_proxy());
    let probe = probe.map(|ProbeKind::Pinning| Probe::start(event_loop.create_proxy()));
    let mut strip = Strip::new(event_loop.create_proxy(), probe);
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
    frame: Rgba640x74,
    padding: [Rgba; NATIVE_HEIGHT as usize],
    level: LevelMode,
    raise: RaiseTimer,
    hover: HoverTracker,
    gate: FrameGate,
    placement_pending: bool,
    idle_seen: bool,
    failed: Option<StripError>,
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
    fn new(proxy: EventLoopProxy<UserEvent>, probe: Option<Probe>) -> Self {
        let level = probe.as_ref().map_or(DEFAULT_LEVEL, Probe::level);
        Self {
            proxy: Some(proxy),
            probe,
            view: None,
            frame: placeholder::frame(),
            padding: placeholder::padding(),
            level,
            raise: RaiseTimer::default(),
            hover: HoverTracker::default(),
            gate: FrameGate::default(),
            placement_pending: false,
            idle_seen: false,
            failed: None,
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
                if self.gate.should_present() {
                    match view
                        .presenter
                        .present(&self.frame, &view.scale, &self.padding)
                    {
                        Ok(()) => self.gate.presented(),
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
                if self.gate.set_occluded(occluded) {
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
    fn new_events(&mut self, _event_loop: &ActiveEventLoop, _cause: StartCause) {
        if !self.raise.restore_due(Instant::now()) {
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

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        self.with_view(|strip, view| strip.window_input(event_loop, view, event));
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
        event_loop.set_control_flow(match self.raise.deadline() {
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
