//! wgpu (Rust + WGSL) production render path — issue #1.
//!
//! [`GpuRenderer`] is the headless, native side of the shared renderer described
//! in `docs/overview.md`: the same WGSL that the browser will run (issue #4) also
//! runs here on the desktop GPU, so the CLI and the web produce matching frames.
//!
//! ## Parity contract (must match [`crate::Transition::render_cpu`])
//!
//! The CPU oracle mixes the two images as **raw sRGB bytes**
//! (`out = from·(1−t) + to·t`, no gamma). To match it the GPU path:
//!
//! - uploads `from`/`to` as **[`wgpu::TextureFormat::Rgba8Unorm`]** (NOT `*Srgb`)
//!   and renders into an `Rgba8Unorm` target, so no sRGB↔linear conversion ever
//!   happens — sampling yields `byte / 255` and the shader's `mix` is the same
//!   arithmetic as the CPU loop;
//! - reads the result back accounting for wgpu's 256-byte row-alignment
//!   requirement on `copy_texture_to_buffer`.
//!
//! ## Shader binding contract
//!
//! The WGSL supplied by a [`crate::Transition`] (see
//! [`crate::transitions::crossfade`]) must expose `vs_main` / `fs_main` and bind:
//!
//! | group(0) binding | resource                         |
//! |------------------|----------------------------------|
//! | 0                | `from` texture (`texture_2d<f32>`) |
//! | 1                | `to` texture (`texture_2d<f32>`)   |
//! | 2                | sampler                          |
//! | 3                | uniform `{ t: f32 }`             |

use image::RgbaImage;
use wgpu::util::DeviceExt;

/// Bytes per pixel for `Rgba8Unorm`.
const BYTES_PER_PIXEL: u32 = 4;
/// wgpu requires `bytes_per_row` of a texture→buffer copy to be a multiple of
/// this (`COPY_BYTES_PER_ROW_ALIGNMENT`).
const ROW_ALIGNMENT: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;

/// Round `value` up to the next multiple of `align` (a power of two).
fn align_up(value: u32, align: u32) -> u32 {
    value.div_ceil(align) * align
}

/// Uniform block handed to the fragment shader. `t` is padded to 16 bytes to
/// satisfy WGSL uniform-buffer alignment.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    t: f32,
    _pad: [f32; 3],
}

/// Headless wgpu renderer. Holds a device/queue; render any transition's WGSL
/// against a `from`/`to` pair via [`GpuRenderer::render`].
pub struct GpuRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    adapter_name: String,
}

impl GpuRenderer {
    /// Bring up a headless GPU context (no surface). Returns `None` when no
    /// adapter is available (e.g. CI without a GPU / software rasterizer), so
    /// callers can fall back to the CPU path instead of hard-failing.
    pub fn new() -> Option<Self> {
        pollster::block_on(Self::new_async())
    }

    async fn new_async() -> Option<Self> {
        // Headless: no window/display handle is needed (backends still come from env).
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .ok()?;
        let adapter_name = adapter.get_info().name;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("additive-gpu-device"),
                ..Default::default()
            })
            .await
            .ok()?;
        Some(Self {
            device,
            queue,
            adapter_name,
        })
    }

    /// Name of the underlying adapter (for diagnostics / proving the GPU path ran).
    pub fn adapter_name(&self) -> &str {
        &self.adapter_name
    }

    /// Render one frame: upload `from`/`to`, run `shader_wgsl` over a full-screen
    /// triangle with uniform `t`, and read the `Rgba8Unorm` target back.
    ///
    /// `from` and `to` must share dimensions; `t` is clamped to `0.0..=1.0`.
    pub fn render(&self, from: &RgbaImage, to: &RgbaImage, shader_wgsl: &str, t: f32) -> RgbaImage {
        assert_eq!(
            from.dimensions(),
            to.dimensions(),
            "from and to must share dimensions"
        );
        let (width, height) = from.dimensions();
        let t = t.clamp(0.0, 1.0);

        // Guard empty inputs: wgpu rejects zero-sized textures, and the CPU oracle
        // (`render_cpu`) simply yields an empty image here, so mirror that.
        if width == 0 || height == 0 {
            return RgbaImage::new(width, height);
        }

        // sRGB-byte parity: NOT *Srgb. Sampling and rendering stay in raw bytes.
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let extent = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };

        let from_view = self.upload_texture(from, format, "additive-from");
        let to_view = self.upload_texture(to, format, "additive-to");

        let target = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("additive-target"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());

        let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("additive-sampler"),
            ..Default::default()
        });

        let params = Params { t, _pad: [0.0; 3] };
        let uniform_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("additive-params"),
                contents: bytemuck::bytes_of(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });

        let bind_group_layout =
            self.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("additive-bgl"),
                    entries: &[
                        texture_entry(0),
                        texture_entry(1),
                        wgpu::BindGroupLayoutEntry {
                            binding: 2,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 3,
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

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("additive-bg"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&from_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&to_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: uniform_buffer.as_entire_binding(),
                },
            ],
        });

        let shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("additive-shader"),
                source: wgpu::ShaderSource::Wgsl(shader_wgsl.into()),
            });

        let pipeline_layout = self
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("additive-pl"),
                bind_group_layouts: &[Some(&bind_group_layout)],
                immediate_size: 0,
            });

        let pipeline = self
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("additive-pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });

        // Read-back buffer: each row padded up to ROW_ALIGNMENT bytes.
        let unpadded_bytes_per_row = width * BYTES_PER_PIXEL;
        let padded_bytes_per_row = align_up(unpadded_bytes_per_row, ROW_ALIGNMENT);
        let output_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("additive-readback"),
            size: (padded_bytes_per_row * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("additive-encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("additive-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &output_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            extent,
        );
        self.queue.submit(Some(encoder.finish()));

        // Map and block until ready.
        let slice = output_buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("device poll failed");

        let mapped = slice.get_mapped_range();
        // Strip the row padding: copy the leading `unpadded_bytes_per_row` of each row.
        let mut pixels = Vec::with_capacity((unpadded_bytes_per_row * height) as usize);
        for row in 0..height {
            let start = (row * padded_bytes_per_row) as usize;
            let end = start + unpadded_bytes_per_row as usize;
            pixels.extend_from_slice(&mapped[start..end]);
        }
        drop(mapped);
        output_buffer.unmap();

        RgbaImage::from_raw(width, height, pixels)
            .expect("read-back buffer matches image dimensions")
    }

    /// Upload an `RgbaImage` into a sampled texture and return its view.
    fn upload_texture(
        &self,
        img: &RgbaImage,
        format: wgpu::TextureFormat,
        label: &str,
    ) -> wgpu::TextureView {
        let (width, height) = img.dimensions();
        let extent = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            img.as_raw(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * BYTES_PER_PIXEL),
                rows_per_image: Some(height),
            },
            extent,
        );
        texture.create_view(&wgpu::TextureViewDescriptor::default())
    }
}

/// A `texture_2d<f32>` bind-group-layout entry, fragment-visible, non-filtering
/// (we sample at exact pixel centers so no filtering is needed).
fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transitions::crossfade::{Crossfade, CROSSFADE_WGSL};
    use crate::Transition;
    use image::Rgba;

    /// A small, varied test image so per-pixel parity isn't trivially satisfied
    /// by a flat color.
    fn gradient(w: u32, h: u32, base: [u8; 4]) -> RgbaImage {
        let mut img = RgbaImage::new(w, h);
        for (x, y, px) in img.enumerate_pixels_mut() {
            *px = Rgba([
                base[0].wrapping_add((x * 7) as u8),
                base[1].wrapping_add((y * 11) as u8),
                base[2].wrapping_add((x * y) as u8),
                base[3],
            ]);
        }
        img
    }

    /// Assert every channel of `a`/`b` agrees within `±2`, returning the max diff.
    fn assert_within_tolerance(a: &RgbaImage, b: &RgbaImage, ctx: &str) -> u8 {
        assert_eq!(a.dimensions(), b.dimensions(), "{ctx}: dimension mismatch");
        let mut max_diff = 0u8;
        for (x, y, ap) in a.enumerate_pixels() {
            let bp = b.get_pixel(x, y);
            for ch in 0..4 {
                let d = ap.0[ch].abs_diff(bp.0[ch]);
                max_diff = max_diff.max(d);
                assert!(
                    d <= 2,
                    "{ctx}: pixel ({x},{y}) channel {ch} differs by {d} (a={:?} b={:?})",
                    ap.0,
                    bp.0
                );
            }
        }
        max_diff
    }

    #[test]
    fn gpu_matches_cpu_within_tolerance() {
        let Some(renderer) = GpuRenderer::new() else {
            eprintln!("SKIP gpu_matches_cpu_within_tolerance: no GPU adapter available");
            return;
        };
        eprintln!(
            "GPU parity test running on adapter: {}",
            renderer.adapter_name()
        );

        let cf = Crossfade;
        // Cover the read-back strip boundary both ways:
        //   - 37x23 / 1x1: width not a multiple of 64, so rows ARE padded;
        //   - 64x16: width*4 = 256 is already row-aligned, so NO padding.
        for &(w, h) in &[(37u32, 23u32), (64, 16), (1, 1)] {
            let from = gradient(w, h, [10, 40, 80, 255]);
            let to = gradient(w, h, [200, 90, 20, 200]);

            for &t in &[0.0_f32, 0.25, 0.5, 0.75, 1.0] {
                let cpu = cf.render_cpu(&from, &to, t);
                let gpu = renderer.render(&from, &to, CROSSFADE_WGSL, t);
                let max_diff = assert_within_tolerance(&cpu, &gpu, &format!("{w}x{h} t={t}"));
                eprintln!("{w}x{h} t={t}: max per-channel diff = {max_diff}");
            }
        }
    }

    /// Orientation regression: with the WGSL `v` flip in place, the GPU output at
    /// `t = 0` must equal `from` and at `t = 1` must equal `to`, pixel-for-pixel.
    /// A `y`-asymmetric gradient (top rows differ sharply from bottom rows) makes
    /// an accidental vertical flip show up as a large diff rather than canceling.
    #[test]
    fn gpu_orientation_matches_inputs() {
        let Some(renderer) = GpuRenderer::new() else {
            eprintln!("SKIP gpu_orientation_matches_inputs: no GPU adapter available");
            return;
        };

        // Strong top↔bottom asymmetry: red ramps with y, blue is the inverse ramp.
        let (w, h) = (8u32, 13u32);
        let mut from = RgbaImage::new(w, h);
        for (_x, y, px) in from.enumerate_pixels_mut() {
            let up = ((y * 255) / (h - 1)) as u8;
            *px = Rgba([up, 0, 255 - up, 255]);
        }
        let mut to = RgbaImage::new(w, h);
        for (x, _y, px) in to.enumerate_pixels_mut() {
            let across = ((x * 255) / (w - 1)) as u8;
            *px = Rgba([0, across, 0, 255]);
        }

        let at0 = renderer.render(&from, &to, CROSSFADE_WGSL, 0.0);
        assert_within_tolerance(&from, &at0, "t=0 must equal `from` (no vertical flip)");
        let at1 = renderer.render(&from, &to, CROSSFADE_WGSL, 1.0);
        assert_within_tolerance(&to, &at1, "t=1 must equal `to` (no vertical flip)");
        eprintln!("orientation regression: t=0==from, t=1==to confirmed");
    }
}
