//! The egui windows: egui-winit for input and egui-wgpu for drawing, inside the strip's own
//! event loop, never eframe.
//!
//! Every window shares one egui context and one painter, so they share one wgpu device and one
//! texture namespace. The host exists only while a window is open: the first window creates it,
//! and with it the device; the strip drops it when the last one closes, and the device goes
//! with it. A wgpu device alone costs far more memory than the strip, so none is kept for a
//! window that might open later.
//!
//! A window is painted only when egui asks: after input, and at the delay it names after each
//! pass. The loop waits for the earliest such deadline and for nothing once none remains
//! (`repaint`), so an open window that shows nothing new keeps the loop asleep.

use std::num::NonZeroU32;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use egui::{TexturesDelta, ViewportId, ViewportIdSet};
use egui_wgpu::winit::Painter;
use egui_wgpu::{RendererOptions, SurfaceConfig, WgpuConfiguration, WgpuSetup, WgpuSetupCreateNew};
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowId};

use crate::repaint::{self, RepaintSchedule};
use crate::{copy, pinning};

/// The kinds of egui window, at most one of each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindowKind {
    Debug,
}

impl WindowKind {
    /// A fixed viewport per kind. Never egui's root, which every context keeps for itself.
    fn viewport(self) -> ViewportId {
        ViewportId::from_hash_of(("repo-terrarium", self))
    }

    fn size(self) -> LogicalSize<f64> {
        match self {
            WindowKind::Debug => LogicalSize::new(460.0, 760.0),
        }
    }
}

/// Why an egui window could not open. Its wording is in the copy module.
#[derive(Debug)]
pub enum HostError {
    Window(winit::error::OsError),
    Gpu(egui_wgpu::WgpuError),
}

struct EguiWindow {
    kind: WindowKind,
    viewport: ViewportId,
    window: Arc<Window>,
    state: egui_winit::State,
    /// Nothing is painted while a window is covered or has no size: a hidden surface can block
    /// on its next drawable, and an empty one is an error.
    occluded: bool,
    empty: bool,
}

/// Repaints egui asked for between passes, from its callback, waiting to be scheduled.
type Inbox = Arc<Mutex<Vec<(ViewportId, Instant)>>>;

pub struct EguiHost {
    ctx: egui::Context,
    painter: Painter,
    windows: Vec<EguiWindow>,
    /// Texture changes not yet uploaded: one context feeds every window, and a pass whose
    /// window could not paint leaves its changes for the next one that can.
    pending: TexturesDelta,
    schedule: RepaintSchedule<ViewportId>,
    inbox: Inbox,
}

impl EguiHost {
    /// A host with no window yet, and so no device: only the wgpu instance exists.
    pub fn new() -> EguiHost {
        let ctx = egui::Context::default();
        let inbox: Inbox = Arc::default();
        let sink = Arc::clone(&inbox);
        ctx.set_request_repaint_callback(move |info| {
            if let Some(at) = repaint::after(Instant::now(), info.delay) {
                sink.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push((info.viewport_id, at));
            }
        });
        let painter = pollster::block_on(Painter::new(
            ctx.clone(),
            configuration(),
            false,
            RendererOptions::default(),
        ));
        EguiHost {
            ctx,
            painter,
            windows: Vec::new(),
            pending: TexturesDelta::default(),
            schedule: RepaintSchedule::default(),
            inbox,
        }
    }

    /// Opens a `kind` window, or brings the open one forward, and gives it the keyboard.
    pub fn open(
        &mut self,
        event_loop: &ActiveEventLoop,
        kind: WindowKind,
    ) -> Result<(), HostError> {
        if let Some(open) = self.windows.iter().find(|w| w.kind == kind) {
            pinning::activate(&open.window);
            return Ok(());
        }
        let attributes = Window::default_attributes()
            .with_title(copy::debug::window_title(kind))
            .with_inner_size(kind.size())
            .with_min_inner_size(LogicalSize::new(320.0, 240.0))
            .with_active(true);
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .map_err(HostError::Window)?,
        );
        let viewport = kind.viewport();
        pollster::block_on(self.painter.set_window(viewport, Some(Arc::clone(&window))))
            .map_err(HostError::Gpu)?;
        // egui measures in points, as f32; a display's scale is a small ratio such as 2.0.
        #[allow(clippy::cast_possible_truncation)]
        let pixels_per_point = window.scale_factor() as f32;
        let state = egui_winit::State::new(
            self.ctx.clone(),
            viewport,
            &*window,
            Some(pixels_per_point),
            window.theme(),
            self.painter.max_texture_side(),
        );
        window.request_redraw();
        pinning::activate(&window);
        self.windows.push(EguiWindow {
            kind,
            viewport,
            window,
            state,
            occluded: false,
            empty: false,
        });
        Ok(())
    }

    pub fn owns(&self, id: WindowId) -> bool {
        self.windows.iter().any(|w| w.window.id() == id)
    }

    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    /// Handles `event` for the window `id`, painting it with `draw` when asked to.
    pub fn window_event(
        &mut self,
        id: WindowId,
        event: &WindowEvent,
        draw: &mut dyn FnMut(WindowKind, &mut egui::Ui),
    ) {
        let Some(index) = self.windows.iter().position(|w| w.window.id() == id) else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => {
                self.close(self.windows[index].kind);
                return;
            }
            WindowEvent::RedrawRequested => {
                self.paint(index, draw);
                return;
            }
            WindowEvent::Resized(size) => {
                let window = &mut self.windows[index];
                match (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) {
                    (Some(width), Some(height)) => {
                        window.empty = false;
                        self.painter
                            .on_window_resized(window.viewport, width, height);
                    }
                    _ => window.empty = true,
                }
            }
            WindowEvent::Occluded(occluded) => {
                let window = &mut self.windows[index];
                window.occluded = *occluded;
                if !occluded {
                    window.window.request_redraw();
                }
            }
            _ => {}
        }
        let window = &mut self.windows[index];
        if window.state.on_window_event(&window.window, event).repaint {
            window.window.request_redraw();
        }
    }

    /// Closes the `kind` window. The host stays, without it; the strip drops the host once
    /// it is empty.
    pub fn close(&mut self, kind: WindowKind) {
        let Some(index) = self.windows.iter().position(|w| w.kind == kind) else {
            return;
        };
        let closed = self.windows.swap_remove(index);
        self.schedule.forget(closed.viewport);
        // Each surface holds its window; only this lets the window go.
        let live: ViewportIdSet = self.windows.iter().map(|w| w.viewport).collect();
        self.painter.gc_viewports(&live);
    }

    /// Asks every window due a repaint by `now` to redraw.
    pub fn redraw_due(&mut self, now: Instant) {
        self.take_inbox();
        for viewport in self.schedule.take_due(now) {
            if let Some(window) = self.windows.iter().find(|w| w.viewport == viewport) {
                window.window.request_redraw();
            }
        }
    }

    /// The earliest repaint any open window wants.
    pub fn next_repaint(&mut self) -> Option<Instant> {
        self.take_inbox();
        self.schedule.next()
    }

    fn take_inbox(&mut self) {
        let requests =
            std::mem::take(&mut *self.inbox.lock().unwrap_or_else(PoisonError::into_inner));
        for (viewport, at) in requests {
            if self.windows.iter().any(|w| w.viewport == viewport) {
                self.schedule.request(viewport, at);
            }
        }
    }

    fn paint(&mut self, index: usize, draw: &mut dyn FnMut(WindowKind, &mut egui::Ui)) {
        let window = &mut self.windows[index];
        self.schedule.forget(window.viewport);
        if window.occluded || window.empty {
            return;
        }
        let input = window.state.take_egui_input(&window.window);
        let kind = window.kind;
        let output = self.ctx.run_ui(input, |ui| {
            egui::CentralPanel::default_margins().show(ui, |ui| draw(kind, ui));
        });
        window
            .state
            .handle_platform_output(&window.window, output.platform_output);
        let primitives = self.ctx.tessellate(output.shapes, output.pixels_per_point);
        self.pending.append(output.textures_delta);
        // egui-wgpu's surfaces are not sRGB, so the clear colour is given gamma-encoded.
        let clear = self
            .ctx
            .global_style()
            .visuals
            .panel_fill
            .to_normalized_gamma_f32();
        self.painter.paint_and_update_textures(
            window.viewport,
            output.pixels_per_point,
            clear,
            &primitives,
            &mut self.pending,
            Vec::new(),
            &window.window,
        );
        if let Some(viewport) = output.viewport_output.get(&window.viewport)
            && let Some(at) = repaint::after(Instant::now(), viewport.repaint_delay)
        {
            self.schedule.request(window.viewport, at);
        }
    }
}

impl Drop for EguiHost {
    /// Lets the GPU finish what it was given, then lets the device go with the painter. Texture
    /// changes no window took are dropped with it; egui insists they be cleared first.
    fn drop(&mut self) {
        if let Some(render) = self.painter.render_state() {
            let _ = render.device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_secs(1)),
            });
        }
        self.pending.clear();
    }
}

/// The device the windows share: the one backend each OS needs, the integrated GPU where
/// there are two (these windows are forms, not games), and allocations that favour memory over
/// speed. One frame in flight keeps the fewest drawables alive.
fn configuration() -> WgpuConfiguration {
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = if cfg!(target_os = "macos") {
        wgpu::Backends::METAL
    } else {
        wgpu::Backends::DX12
    };
    setup.power_preference = wgpu::PowerPreference::LowPower;
    let device = Arc::clone(&setup.device_descriptor);
    setup.device_descriptor = Arc::new(move |adapter| wgpu::DeviceDescriptor {
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        ..device(adapter)
    });
    WgpuConfiguration {
        surface: SurfaceConfig::LOW_LATENCY,
        wgpu_setup: WgpuSetup::CreateNew(setup),
        ..WgpuConfiguration::default()
    }
}
