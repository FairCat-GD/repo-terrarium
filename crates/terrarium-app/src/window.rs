//! A plain titled window that closes when asked. Nothing is drawn in it.

use std::error::Error;

use winit::application::ApplicationHandler;
use winit::error::OsError;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use crate::copy;

/// Runs the event loop until the window is closed.
pub fn run() -> Result<(), Box<dyn Error>> {
    let event_loop = EventLoop::new()?;
    // The loop sleeps until an event arrives; polling would break the idle CPU budget (N-01).
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App::default();
    event_loop.run_app(&mut app)?;
    match app.failed {
        Some(err) => Err(err.into()),
        None => Ok(()),
    }
}

#[derive(Default)]
struct App {
    window: Option<Window>,
    failed: Option<OsError>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes().with_title(copy::window_title());
        match event_loop.create_window(attributes) {
            Ok(window) => self.window = Some(window),
            Err(err) => {
                self.failed = Some(err);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if event == WindowEvent::CloseRequested {
            self.window = None;
            event_loop.exit();
        }
    }
}
