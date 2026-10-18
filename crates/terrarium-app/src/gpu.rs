//! The one GPU device the app holds: the strip's, when it is drawn by wgpu, which the egui
//! windows then share, so the process never holds two. A device alone costs far more memory
//! than the strip, so it exists only while something draws with it.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use egui_wgpu::{WgpuSetup, WgpuSetupCreateNew, WgpuSetupExisting};

/// A device and what made it. Clones share it.
#[derive(Clone)]
pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    /// Set once the device is lost or reports an error nobody caught: from then on nothing
    /// is drawn with it.
    failed: Arc<AtomicBool>,
}

/// Why no device could be had. Its wording is in the copy module.
#[derive(Debug)]
pub enum GpuError {
    Adapter(wgpu::RequestAdapterError),
    Device(wgpu::RequestDeviceError),
}

impl Gpu {
    /// A device with the settings every window of the app uses.
    pub fn new() -> Result<Gpu, GpuError> {
        let setup = create_new();
        let instance = wgpu::Instance::new(setup.instance_descriptor);
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: setup.power_preference,
            force_fallback_adapter: false,
            compatible_surface: None,
            ..Default::default()
        }))
        .map_err(GpuError::Adapter)?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&(setup.device_descriptor)(&adapter)))
                .map_err(GpuError::Device)?;
        let failed = Arc::new(AtomicBool::new(false));
        // The default handler panics; here an error ends drawing with the device instead, and
        // the strip falls back to its CPU presenter.
        let flag = Arc::clone(&failed);
        device.on_uncaptured_error(Arc::new(move |_| flag.store(true, Ordering::Relaxed)));
        let flag = Arc::clone(&failed);
        device.set_device_lost_callback(move |_, _| flag.store(true, Ordering::Relaxed));
        Ok(Gpu {
            instance,
            adapter,
            device,
            queue,
            failed,
        })
    }

    /// Whether the device was lost, or reported an error.
    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }

    /// The egui windows' setup on this device.
    pub fn existing(&self) -> WgpuSetup {
        WgpuSetup::Existing(WgpuSetupExisting {
            instance: self.instance.clone(),
            adapter: self.adapter.clone(),
            device: self.device.clone(),
            queue: self.queue.clone(),
        })
    }
}

/// The settings of every device the app makes: the one backend each OS needs, the integrated
/// GPU where there are two (a strip and forms, not games), and allocations that favour memory
/// over speed. egui's own device descriptor otherwise, whose texture limit holds the widest
/// display.
pub fn create_new() -> WgpuSetupCreateNew {
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
    setup
}
