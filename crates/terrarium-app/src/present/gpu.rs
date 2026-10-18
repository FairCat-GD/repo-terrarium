//! The GPU presenter: the native image goes up as a texture, at most 840 by 74, and a fragment
//! shader scales it by the strip's integer, nearest pixel, with letterbox beside it. Its pixels
//! are those `upscale_into` writes, which [`Upscaler::read_back`] lets a test, and the soak,
//! check on the device itself.
//!
//! The surface is always the window's own size, so the OS never stretches it (I-11): while the
//! window settles on the strip's size, what lies past the strip is letterbox.

use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use terrarium_compose::frame::{NATIVE_HEIGHT, NativeImage, Rgba};
use terrarium_compose::scale::{LETTERBOX, StripScale, upscale_into};
use winit::window::Window;

use super::{Backend, PresentError, Presenter, Shown};
use crate::gpu::Gpu;

/// The surface formats that hold a colour's bytes as they are, preferred first: no sRGB
/// conversion, so the screen gets the compositor's bytes.
const FORMATS: [wgpu::TextureFormat; 2] = [
    wgpu::TextureFormat::Bgra8Unorm,
    wgpu::TextureFormat::Rgba8Unorm,
];

pub struct WgpuPresenter {
    gpu: Gpu,
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    upscaler: Upscaler,
}

impl WgpuPresenter {
    pub fn new(gpu: &Gpu, window: &Arc<Window>) -> Result<Self, PresentError> {
        let surface = gpu
            .instance
            .create_surface(Arc::clone(window))
            .map_err(|err| PresentError::Surface(err.to_string()))?;
        let formats = surface.get_capabilities(&gpu.adapter).formats;
        let format = FORMATS
            .into_iter()
            .find(|format| formats.contains(format))
            .ok_or(PresentError::NoFormat)?;
        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: size.width.max(1),
            height: size.height.max(1),
            // Shown at the display's refresh, one frame in flight: the fewest drawables alive.
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 1,
            alpha_mode: wgpu::CompositeAlphaMode::Opaque,
            view_formats: Vec::new(),
        };
        surface.configure(&gpu.device, &config);
        Ok(Self {
            upscaler: Upscaler::new(&gpu.device, format),
            gpu: gpu.clone(),
            window: Arc::clone(window),
            surface,
            config,
        })
    }

    /// Sizes the surface to the window, if it is not already.
    fn fit(&mut self, width: u32, height: u32) {
        if (self.config.width, self.config.height) != (width, height) {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&self.gpu.device, &self.config);
        }
    }
}

impl Presenter for WgpuPresenter {
    fn present(
        &mut self,
        frame: &NativeImage,
        scale: &StripScale,
        _whole: bool,
    ) -> Result<Shown, PresentError> {
        if self.gpu.failed() {
            return Err(PresentError::Lost);
        }
        let size = self.window.inner_size();
        if size.width == 0 || size.height == 0 {
            return Ok(Shown::Skipped);
        }
        self.fit(size.width, size.height);
        let mut texture = self.surface.get_current_texture();
        if matches!(texture, wgpu::CurrentSurfaceTexture::Outdated) {
            self.surface.configure(&self.gpu.device, &self.config);
            texture = self.surface.get_current_texture();
        }
        let texture = match texture {
            wgpu::CurrentSurfaceTexture::Success(texture)
            | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Outdated => return Ok(Shown::Skipped),
            wgpu::CurrentSurfaceTexture::Lost => return Err(PresentError::Lost),
            wgpu::CurrentSurfaceTexture::Validation => return Err(PresentError::Validation),
        };
        let view = texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.upscaler.draw(&self.gpu, frame, scale, &view);
        self.gpu.queue.present(texture);
        Ok(Shown::Presented)
    }

    fn backend(&self) -> Backend {
        Backend::Wgpu
    }

    fn pixels_equal(&mut self, frame: &NativeImage, scale: &StripScale) -> Option<bool> {
        let gpu = self.upscaler.read_back(&self.gpu, frame, scale)?;
        Some(gpu == upscaled(frame, scale))
    }
}

/// What `upscale_into` makes of `frame`, alpha left out: no surface shows it.
pub fn upscaled(frame: &NativeImage, scale: &StripScale) -> Vec<[u8; 3]> {
    let mut pixels = vec![[0; 3]; scale.pixel_count()];
    let opaque = |[r, g, b, _]: Rgba| [r, g, b];
    match upscale_into(frame, scale, opaque, &mut pixels) {
        Ok(()) => pixels,
        Err(_) => Vec::new(),
    }
}

/// The shader, its pipeline, and the native image's texture.
pub struct Upscaler {
    format: wgpu::TextureFormat,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    placement: wgpu::Buffer,
    /// The texture the native image goes up to, with its width: a viewport of another width
    /// gets a texture of its own.
    native: Option<(u32, wgpu::Texture, wgpu::BindGroup)>,
}

/// The shader's `Placement`: eight words, a multiple of a uniform's sixteen bytes.
const PLACEMENT_BYTES: u64 = 32;

impl Upscaler {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Upscaler {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("strip upscale"),
            source: wgpu::ShaderSource::Wgsl(include_str!("upscale.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("strip upscale"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("strip upscale"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("strip upscale"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vertex"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            // No blending: the target takes the native image's bytes as they are.
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fragment"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let placement = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("strip placement"),
            size: PLACEMENT_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Upscaler {
            format,
            pipeline,
            layout,
            placement,
            native: None,
        }
    }

    /// Uploads `frame` and draws it at `scale` into `target`.
    pub fn draw(
        &mut self,
        gpu: &Gpu,
        frame: &NativeImage,
        scale: &StripScale,
        target: &wgpu::TextureView,
    ) {
        let width = frame.width();
        if self.native.as_ref().is_none_or(|(held, ..)| *held != width) {
            self.native = Some(self.texture(&gpu.device, width));
        }
        let Some((_, texture, bind_group)) = &self.native else {
            return;
        };
        let extent = wgpu::Extent3d {
            width,
            height: NATIVE_HEIGHT,
            depth_or_array_layers: 1,
        };
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            frame.pixels().as_flattened(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(NATIVE_HEIGHT),
            },
            extent,
        );
        gpu.queue
            .write_buffer(&self.placement, 0, &placement(frame, scale));
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("strip"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("strip"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        gpu.queue.submit([encoder.finish()]);
    }

    fn texture(&self, device: &wgpu::Device, width: u32) -> (u32, wgpu::Texture, wgpu::BindGroup) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("strip native"),
            size: wgpu::Extent3d {
                width,
                height: NATIVE_HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // Not sRGB: the shader reads the bytes as they are.
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("strip upscale"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.placement.as_entire_binding(),
                },
            ],
        });
        (width, texture, bind_group)
    }

    /// Draws `frame` at `scale` off screen, in the surface's format, and reads the pixels back,
    /// row by row, alpha left out; none if the device could not say.
    pub fn read_back(
        &mut self,
        gpu: &Gpu,
        frame: &NativeImage,
        scale: &StripScale,
    ) -> Option<Vec<[u8; 3]>> {
        let extent = wgpu::Extent3d {
            width: scale.width,
            height: scale.height,
            depth_or_array_layers: 1,
        };
        let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("strip read back"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        self.draw(gpu, frame, scale, &view);
        let row = (scale.width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("strip read back"),
            size: u64::from(row) * u64::from(scale.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("strip read back"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(scale.height),
                },
            },
            extent,
        );
        gpu.queue.submit([encoder.finish()]);
        let (sent, mapped) = mpsc::channel();
        buffer.map_async(wgpu::MapMode::Read, .., move |result| {
            let _ = sent.send(result);
        });
        gpu.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_secs(5)),
            })
            .ok()?;
        mapped.recv_timeout(Duration::from_secs(5)).ok()?.ok()?;
        let bytes = buffer.get_mapped_range(..).ok()?;
        let bgra = self.format == wgpu::TextureFormat::Bgra8Unorm;
        let width = scale.width as usize * 4;
        let pixels = bytes
            .chunks_exact(row as usize)
            .flat_map(|line| line[..width].as_chunks::<4>().0)
            .map(|&[a, b, c, _]| if bgra { [c, b, a] } else { [a, b, c] })
            .collect();
        Some(pixels)
    }
}

/// The shader's `Placement` for `frame` at `scale`, as its bytes.
fn placement(frame: &NativeImage, scale: &StripScale) -> [u8; 32] {
    let image_end = scale
        .image_x
        .saturating_add(frame.width().saturating_mul(scale.scale))
        .min(scale.width);
    let words = [
        scale.scale,
        scale.image_x,
        image_end,
        scale.height,
        u32::from_le_bytes(LETTERBOX),
        0,
        0,
        0,
    ];
    let mut bytes = [0; 32];
    for (chunk, word) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(words) {
        *chunk = word.to_le_bytes();
    }
    bytes
}

#[cfg(test)]
mod tests {
    use terrarium_compose::scale::SizeStep;

    use super::*;

    /// A frame no two neighbouring pixels share, so a pixel taken from the wrong place shows.
    fn pattern(columns: u32) -> NativeImage {
        let scale = StripScale::exact_viewport(1, terrarium_compose::frame::Viewport::new(columns));
        let mut frame = NativeImage::new(scale.viewport(), [0, 0, 0, u8::MAX]);
        for y in 0..NATIVE_HEIGHT {
            for x in 0..frame.width() {
                let [lo, hi] = u16::try_from(x).expect("narrow").to_le_bytes();
                let y8 = u8::try_from(y).expect("short");
                frame.set_pixel(x, y, [lo, hi ^ y8, y8.wrapping_mul(3) ^ lo, u8::MAX]);
            }
        }
        frame
    }

    #[test]
    fn the_gpu_draws_the_pixels_the_cpu_upscale_writes() {
        // No adapter fails the test: on the Mac and on Windows there is always one.
        let gpu = Gpu::new().expect("a GPU device");
        for format in FORMATS {
            let mut upscaler = Upscaler::new(&gpu.device, format);
            // Cut inside a column (1301), letterboxed with an odd remainder (1683), the scene
            // at 3× and 4× (1920, 2560), and 4K at 6× (3840); then one smaller.
            for width in [1301, 1683, 1920, 2560, 3840] {
                for step in [SizeStep::Full, SizeStep::OneSmaller] {
                    let scale = StripScale::for_physical_width(width, step);
                    let frame = pattern(scale.columns);
                    let gpu_pixels = upscaler
                        .read_back(&gpu, &frame, &scale)
                        .expect("the device reads back");
                    let cpu_pixels = upscaled(&frame, &scale);
                    assert_eq!(gpu_pixels.len(), cpu_pixels.len());
                    let first_difference = gpu_pixels
                        .iter()
                        .zip(&cpu_pixels)
                        .position(|(gpu, cpu)| gpu != cpu)
                        .map(|at| (at % scale.width as usize, at / scale.width as usize));
                    assert_eq!(first_difference, None, "{format:?} at {width} {step:?}");
                }
            }
        }
        assert!(!gpu.failed());
    }

    #[test]
    fn the_placement_cuts_the_image_at_the_strip_s_edge() {
        let scale = StripScale::for_physical_width(1301, SizeStep::Full);
        let frame = pattern(scale.columns);
        let words: Vec<u32> = placement(&frame, &scale)
            .as_chunks::<4>()
            .0
            .iter()
            .map(|&chunk| u32::from_le_bytes(chunk))
            .collect();
        assert_eq!(words[..4], [2, 0, 1301, 148]);
        assert_eq!(words[4].to_le_bytes(), LETTERBOX);
    }
}
