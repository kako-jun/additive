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
//! | group(0) binding | resource                         | shape |
//! |------------------|----------------------------------|-------|
//! | 0                | `from` texture (`texture_2d<f32>`) | both |
//! | 1                | `to` texture (`texture_2d<f32>`)   | both |
//! | 2                | sampler                          | both |
//! | 3                | uniform (`Params { t }` / `OrbParams`) | both |
//! | 4                | uniform `OrbArray` (live orbs)   | orb only |
//!
//! Binding 4 is present only for the orb-dissolve path; which bindings a pipeline
//! declares is selected by [`BindShape`] (`Crossfade` = 0..=3, `Orb` = 0..=4).

use std::cell::RefCell;
use std::collections::HashMap;

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

/// Maximum orbs the orb-dissolve shader iterates. Must match `MAX_ORBS` in
/// `orb_dissolve.wgsl` (and `transitions::orb_dissolve::MAX_ORBS`).
const MAX_ORBS: usize = 128;

/// Params block for the orb-dissolve shader: `t`, the live orb count, the
/// UV→isotropic aspect scales (`width/short`, `height/short`) so the shader's orb
/// distance matches the CPU oracle on non-square frames, plus the directional
/// **sweep** state — the wipe-front position (`front`, positive-axis sense) and a
/// direction code (`dir_code`: 0 lr, 1 rl, 2 tb, 3 bt). Padded to 32 bytes.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct OrbParams {
    t: f32,
    orb_count: f32,
    aspect_x: f32,
    aspect_y: f32,
    front: f32,
    dir_code: f32,
    _pad0: f32,
    _pad1: f32,
}

/// One orb as the shader sees it: `pos.xy`, `radius`, `alpha` packed in the first
/// vec4; straight sRGB color in the second. Matches `struct Orb` in the WGSL.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuOrb {
    pub pos_radius_alpha: [f32; 4],
    pub color: [f32; 4],
}

/// The orb-array uniform: a fixed-size `[GpuOrb; MAX_ORBS]` (unused tail zeroed).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct OrbArray {
    orbs: [GpuOrb; MAX_ORBS],
}

/// A render pipeline plus its bind-group layout and sampler, compiled once per
/// distinct shader source. Caching this keeps shader compilation and pipeline
/// creation off the per-frame path (issue #13): a long video renders the same
/// `shader_wgsl` for every frame, so we compile it exactly once.
struct CachedPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

/// The bind-group-layout *shape* a render pipeline needs.
///
/// Every additive's pipeline binds `from`/`to` textures (0, 1), a sampler (2),
/// and the `t` uniform (3); the orb path binds one extra uniform (4) for its orb
/// array. Encoding that one-bit difference here — rather than as a forked
/// `build_*_pipeline` + parallel cache per effect — is what lets the crossfade
/// and orb paths (and every No.14+ effect) share a single pipeline builder and
/// cache (#24). A new effect reuses a variant or adds one; it never forks the
/// builder.
///
/// Invariant for new variants: [`Self::bind_group_layout_entries`] must list
/// entries in ascending `binding` order and keep binding numbers consistent
/// across shapes (binding `n` means the same resource everywhere), so the WGSL
/// and the bind groups in `render` / `render_orbs` stay in lockstep.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum BindShape {
    /// `from`/`to` textures (0, 1), sampler (2), `Params` uniform (3). No.0
    /// crossfade and any plain `(from, to, t)` effect.
    Crossfade,
    /// `Crossfade` plus the `OrbArray` uniform (4). No.13 orb-dissolve.
    Orb,
}

impl BindShape {
    /// The bind-group-layout entries for this shape, in binding order.
    fn bind_group_layout_entries(self) -> Vec<wgpu::BindGroupLayoutEntry> {
        let mut entries = vec![
            texture_entry(0),
            texture_entry(1),
            sampler_entry(2),
            uniform_entry(3),
        ];
        if matches!(self, BindShape::Orb) {
            // Orb-dissolve's extra uniform: the live orb array (binding 4).
            entries.push(uniform_entry(4));
        }
        entries
    }

    /// Suffix for this shape's diagnostic resource labels (`""` / `"-orb"`).
    fn label_suffix(self) -> &'static str {
        match self {
            BindShape::Crossfade => "",
            BindShape::Orb => "-orb",
        }
    }
}

/// Per-dimension GPU resources reused across same-sized frames (issue #13): the
/// uploaded `from`/`to` textures, the render target, and the padded read-back
/// buffer. Reallocating these every frame is the other half of the per-frame
/// cost the cache removes; a fixed-resolution clip allocates them once.
struct SizedResources {
    width: u32,
    height: u32,
    from_texture: wgpu::Texture,
    from_view: wgpu::TextureView,
    to_texture: wgpu::Texture,
    to_view: wgpu::TextureView,
    target: wgpu::Texture,
    target_view: wgpu::TextureView,
    output_buffer: wgpu::Buffer,
    padded_bytes_per_row: u32,
}

/// Headless wgpu renderer. Holds a device/queue plus per-shader pipeline and
/// per-size resource caches, so a multi-frame render (a long `--duration-ms`
/// video) compiles each shader and allocates each texture/buffer only once
/// instead of every frame (issue #13). Render any transition's WGSL against a
/// `from`/`to` pair via [`GpuRenderer::render`].
///
/// The caches are unbounded and never evict: a renderer that renders many
/// distinct `(width, height)` sizes (or many distinct shader sources) keeps the
/// textures/buffers/pipelines for every one of them alive for its whole
/// lifetime. This is intentional for the single-resolution clip use case the
/// renderer targets; a caller that streams arbitrarily many sizes through one
/// long-lived renderer should drop and rebuild it to release memory.
pub struct GpuRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    adapter_name: String,
    /// Render pipelines, keyed by `(shader source, bind-group-layout shape)`.
    /// The crossfade and orb paths share this one cache; the [`BindShape`]
    /// disambiguates their bind-group layouts (the orb path binds an extra orb
    /// uniform), so a shader never reuses the wrong layout. (#13, unified in #24.)
    pipeline_cache: RefCell<HashMap<(String, BindShape), CachedPipeline>>,
    /// Per-size resources for the crossfade path, keyed by `(width, height)`.
    sized_cache: RefCell<HashMap<(u32, u32), SizedResources>>,
    /// Per-size resources for the orb path, keyed by `(width, height)`.
    orb_sized_cache: RefCell<HashMap<(u32, u32), SizedResources>>,
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
            pipeline_cache: RefCell::new(HashMap::new()),
            sized_cache: RefCell::new(HashMap::new()),
            orb_sized_cache: RefCell::new(HashMap::new()),
        })
    }

    /// Name of the underlying adapter (for diagnostics / proving the GPU path ran).
    pub fn adapter_name(&self) -> &str {
        &self.adapter_name
    }

    /// Live entry counts of the resource caches, in the order
    /// `(pipelines, crossfade sizes, orb sizes)`. The single pipeline cache holds
    /// one entry per distinct `(shader source, layout shape)`, so a clip that
    /// drives both the crossfade and orb paths leaves exactly two pipelines.
    /// Exposed for the cache-effectiveness test (issue #13): rendering a clip of
    /// many frames at one size with one shader must leave exactly one pipeline
    /// per path and one sized entry, proving compilation/allocation stayed off
    /// the per-frame path.
    #[cfg(test)]
    fn cache_sizes(&self) -> (usize, usize, usize) {
        (
            self.pipeline_cache.borrow().len(),
            self.sized_cache.borrow().len(),
            self.orb_sized_cache.borrow().len(),
        )
    }

    /// Get-or-build the pipeline for `(shader_wgsl, shape)`, compiling the shader
    /// and pipeline only on first use of that pair. The crossfade and orb paths
    /// share this one cache (and builder); `shape` selects the bind-group layout
    /// so a shader never reuses the wrong one. The closure runs at most once per
    /// distinct `(shader source, shape)` for the life of the renderer.
    ///
    /// A new effect (No.14+) reuses an existing [`BindShape`] or adds a variant —
    /// it does **not** fork a parallel `build_*_pipeline` / pipeline cache (#24).
    fn pipeline<R>(
        &self,
        shader_wgsl: &str,
        shape: BindShape,
        f: impl FnOnce(&CachedPipeline) -> R,
    ) -> R {
        let mut cache = self.pipeline_cache.borrow_mut();
        let entry = cache
            .entry((shader_wgsl.to_owned(), shape))
            .or_insert_with(|| self.build_pipeline(shader_wgsl, shape));
        f(entry)
    }

    /// Compile the render pipeline for `(shader_wgsl, shape)`: a sampler, the
    /// shape's bind-group layout, the shader module, and the full-screen-triangle
    /// pipeline. Called once per `(shader, shape)` by [`Self::pipeline`].
    ///
    /// Resource labels carry the shape suffix (`""` / `"-orb"`) so the crossfade
    /// and orb pipelines stay distinguishable in GPU validation logs / RenderDoc;
    /// labels are diagnostic only and never affect the rendered output.
    fn build_pipeline(&self, shader_wgsl: &str, shape: BindShape) -> CachedPipeline {
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let tag = shape.label_suffix();
        let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some(&format!("additive{tag}-sampler")),
            ..Default::default()
        });
        let entries = shape.bind_group_layout_entries();
        let bind_group_layout =
            self.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some(&format!("additive{tag}-bgl")),
                    entries: &entries,
                });
        let shader = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(&format!("additive{tag}-shader")),
                source: wgpu::ShaderSource::Wgsl(shader_wgsl.into()),
            });
        let pipeline_layout = self
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(&format!("additive{tag}-pl")),
                bind_group_layouts: &[Some(&bind_group_layout)],
                immediate_size: 0,
            });
        let pipeline = self.build_render_pipeline(
            &format!("additive{tag}-pipeline"),
            &pipeline_layout,
            &shader,
            format,
        );
        CachedPipeline {
            pipeline,
            bind_group_layout,
            sampler,
        }
    }

    /// Create the full-screen-triangle render pipeline shared by both render
    /// paths (`vs_main`/`fs_main`, single `Rgba8Unorm` target, no blend).
    fn build_render_pipeline(
        &self,
        label: &str,
        layout: &wgpu::PipelineLayout,
        shader: &wgpu::ShaderModule,
        format: wgpu::TextureFormat,
    ) -> wgpu::RenderPipeline {
        self.device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: shader,
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
            })
    }

    /// Get-or-build the per-size resources for the given `cache`, allocating the
    /// `from`/`to`/target textures and read-back buffer only on first use of a
    /// `(width, height)`. The closure observes the (possibly freshly allocated)
    /// resources.
    fn sized_resources<R>(
        cache: &RefCell<HashMap<(u32, u32), SizedResources>>,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        target_label: &str,
        readback_label: &str,
        f: impl FnOnce(&SizedResources) -> R,
    ) -> R {
        let mut map = cache.borrow_mut();
        let entry = map.entry((width, height)).or_insert_with(|| {
            Self::build_sized_resources(device, width, height, target_label, readback_label)
        });
        f(entry)
    }

    /// Allocate the `from`/`to`/target textures and the padded read-back buffer
    /// for a `(width, height)`. Called once per size by [`Self::sized_resources`].
    fn build_sized_resources(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        target_label: &str,
        readback_label: &str,
    ) -> SizedResources {
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let extent = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let make_input = |label: &str| {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            (texture, view)
        };
        let (from_texture, from_view) = make_input("additive-from");
        let (to_texture, to_view) = make_input("additive-to");

        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(target_label),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());

        let unpadded_bytes_per_row = width * BYTES_PER_PIXEL;
        let padded_bytes_per_row = align_up(unpadded_bytes_per_row, ROW_ALIGNMENT);
        let output_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(readback_label),
            size: (padded_bytes_per_row * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        SizedResources {
            width,
            height,
            from_texture,
            from_view,
            to_texture,
            to_view,
            target,
            target_view,
            output_buffer,
            padded_bytes_per_row,
        }
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

        let params = Params { t, _pad: [0.0; 3] };
        let uniform_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("additive-params"),
                contents: bytemuck::bytes_of(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });

        // Pipeline (shader compile) cached per (shader, layout shape); textures
        // and the read-back buffer cached per size. Only the texture contents and
        // the tiny uniform/bind group are rebuilt per frame (issue #13).
        self.pipeline(shader_wgsl, BindShape::Crossfade, |cached| {
            Self::sized_resources(
                &self.sized_cache,
                &self.device,
                width,
                height,
                "additive-target",
                "additive-readback",
                |res| {
                    self.write_texture_data(&res.from_texture, from);
                    self.write_texture_data(&res.to_texture, to);

                    let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("additive-bg"),
                        layout: &cached.bind_group_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(&res.from_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::TextureView(&res.to_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::Sampler(&cached.sampler),
                            },
                            wgpu::BindGroupEntry {
                                binding: 3,
                                resource: uniform_buffer.as_entire_binding(),
                            },
                        ],
                    });

                    self.run_pass_and_readback(
                        &cached.pipeline,
                        &bind_group,
                        res,
                        "additive-encoder",
                        "additive-pass",
                    )
                },
            )
        })
    }

    /// Render one orb-dissolve frame: same `from`/`to`/`t` contract as
    /// [`render`](Self::render), plus a slice of live orbs blended on top by the
    /// orb-dissolve WGSL (binding 4) and the directional sweep state (`front` =
    /// wipe-front position in the positive-axis sense, `dir_code` = 0 lr / 1 rl /
    /// 2 tb / 3 bt). At most [`MAX_ORBS`] orbs are used.
    ///
    /// This is a deliberate sibling of `render` (not a generalization of it): the
    /// two share one pipeline builder (keyed by [`BindShape`], #24) but keep
    /// separate bodies, so the No.0 crossfade path — and its strict parity test —
    /// is byte-for-byte untouched (`BindShape::Crossfade` produces the exact same
    /// layout as before).
    #[allow(clippy::too_many_arguments)]
    pub fn render_orbs(
        &self,
        from: &RgbaImage,
        to: &RgbaImage,
        shader_wgsl: &str,
        t: f32,
        orbs: &[GpuOrb],
        front: f32,
        dir_code: u32,
    ) -> RgbaImage {
        assert_eq!(
            from.dimensions(),
            to.dimensions(),
            "from and to must share dimensions"
        );
        let (width, height) = from.dimensions();
        let t = t.clamp(0.0, 1.0);
        if width == 0 || height == 0 {
            return RgbaImage::new(width, height);
        }

        let orb_count = orbs.len().min(MAX_ORBS);
        // UV→isotropic scales: radii are normalized by the shorter axis, so the
        // shader scales UV deltas by w/short and h/short before measuring distance
        // (mirrors `render_cpu_cfg`). Guard the degenerate zero-area case above.
        let short = width.min(height) as f32;
        let params = OrbParams {
            t,
            orb_count: orb_count as f32,
            aspect_x: width as f32 / short,
            aspect_y: height as f32 / short,
            front,
            dir_code: dir_code as f32,
            _pad0: 0.0,
            _pad1: 0.0,
        };
        let params_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("additive-orb-params"),
                contents: bytemuck::bytes_of(&params),
                usage: wgpu::BufferUsages::UNIFORM,
            });

        let mut orb_array = OrbArray {
            orbs: [GpuOrb {
                pos_radius_alpha: [0.0; 4],
                color: [0.0; 4],
            }; MAX_ORBS],
        };
        orb_array.orbs[..orb_count].copy_from_slice(&orbs[..orb_count]);
        let orb_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("additive-orb-array"),
                contents: bytemuck::bytes_of(&orb_array),
                usage: wgpu::BufferUsages::UNIFORM,
            });

        // Pipeline (shader compile) cached per (shader, layout shape) in the
        // shared cache; the orb path's `BindShape::Orb` adds binding 4 (the orb
        // array) on top of the crossfade layout. Textures and the read-back
        // buffer keep their own per-size cache (issue #13).
        self.pipeline(shader_wgsl, BindShape::Orb, |cached| {
            Self::sized_resources(
                &self.orb_sized_cache,
                &self.device,
                width,
                height,
                "additive-orb-target",
                "additive-orb-readback",
                |res| {
                    self.write_texture_data(&res.from_texture, from);
                    self.write_texture_data(&res.to_texture, to);

                    let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("additive-orb-bg"),
                        layout: &cached.bind_group_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(&res.from_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::TextureView(&res.to_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::Sampler(&cached.sampler),
                            },
                            wgpu::BindGroupEntry {
                                binding: 3,
                                resource: params_buffer.as_entire_binding(),
                            },
                            wgpu::BindGroupEntry {
                                binding: 4,
                                resource: orb_buffer.as_entire_binding(),
                            },
                        ],
                    });

                    self.run_pass_and_readback(
                        &cached.pipeline,
                        &bind_group,
                        res,
                        "additive-orb-encoder",
                        "additive-orb-pass",
                    )
                },
            )
        })
    }

    /// Upload an `RgbaImage`'s bytes into an existing same-sized texture. The
    /// texture is allocated once per size and reused across frames; only its
    /// contents change per frame (issue #13).
    fn write_texture_data(&self, texture: &wgpu::Texture, img: &RgbaImage) {
        let (width, height) = img.dimensions();
        let extent = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
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
    }

    /// Render one full-screen pass into `res.target`, copy it into the read-back
    /// buffer, map it, and strip wgpu's row padding into a tight `RgbaImage`.
    /// Shared by [`Self::render`] and [`Self::render_orbs`] once their bind
    /// group is built.
    fn run_pass_and_readback(
        &self,
        pipeline: &wgpu::RenderPipeline,
        bind_group: &wgpu::BindGroup,
        res: &SizedResources,
        encoder_label: &str,
        pass_label: &str,
    ) -> RgbaImage {
        let (width, height) = (res.width, res.height);
        let extent = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some(encoder_label),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(pass_label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &res.target_view,
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
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &res.target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &res.output_buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(res.padded_bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            extent,
        );
        self.queue.submit(Some(encoder.finish()));

        let slice = res.output_buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("device poll failed");

        let unpadded_bytes_per_row = width * BYTES_PER_PIXEL;
        let mapped = slice.get_mapped_range();
        let mut pixels = Vec::with_capacity((unpadded_bytes_per_row * height) as usize);
        for row in 0..height {
            let start = (row * res.padded_bytes_per_row) as usize;
            let end = start + unpadded_bytes_per_row as usize;
            pixels.extend_from_slice(&mapped[start..end]);
        }
        drop(mapped);
        res.output_buffer.unmap();

        RgbaImage::from_raw(width, height, pixels)
            .expect("read-back buffer matches image dimensions")
    }
}

/// A fragment-visible uniform-buffer bind-group-layout entry.
fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// A non-filtering sampler bind-group-layout entry, fragment-visible. Pairs with
/// the non-filtering [`texture_entry`]s (we sample at exact pixel centers).
fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
        count: None,
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

    /// orb-dissolve GPU mechanism: the `render_orbs` path must run on a real
    /// adapter and behave like a sweep-wipe — t=0 ≈ `from`, t=1 ≈ `to` (the band
    /// is off-frame at both ends), and a mid-clip frame's `to` region must grow
    /// monotonically (the front sweeps one way). No strict CPU↔GPU pixel parity is
    /// asserted (orb drawing intentionally diverges between rasterizers).
    #[test]
    fn gpu_orb_dissolve_mechanism() {
        use crate::transitions::orb_dissolve::{OrbConfig, OrbDissolve, ORB_DISSOLVE_WGSL};

        let Some(renderer) = GpuRenderer::new() else {
            eprintln!("SKIP gpu_orb_dissolve_mechanism: no GPU adapter available");
            return;
        };
        eprintln!(
            "orb-dissolve GPU test running on adapter: {}",
            renderer.adapter_name()
        );

        // Solid red `from`, solid blue `to`: a pixel's base color tells which side
        // of the seam it lies on.
        let (w, h) = (64u32, 64u32);
        let from = gradient(w, h, [200, 40, 40, 255]);
        let to = gradient(w, h, [20, 60, 200, 255]);
        let cfg = OrbConfig::default();

        let mean_rgb_diff = |a: &RgbaImage, b: &RgbaImage| -> f32 {
            let mut sum = 0u64;
            let mut n = 0u64;
            for (ap, bp) in a.pixels().zip(b.pixels()) {
                for c in 0..3 {
                    sum += ap.0[c].abs_diff(bp.0[c]) as u64;
                    n += 1;
                }
            }
            sum as f32 / n as f32
        };
        let to_fraction = |frame: &RgbaImage| -> f32 {
            let mut to_px = 0u64;
            let mut n = 0u64;
            for ((p, f), g) in frame.pixels().zip(from.pixels()).zip(to.pixels()) {
                let df: u32 = (0..3).map(|c| p.0[c].abs_diff(f.0[c]) as u32).sum();
                let dg: u32 = (0..3).map(|c| p.0[c].abs_diff(g.0[c]) as u32).sum();
                if dg < df {
                    to_px += 1;
                }
                n += 1;
            }
            to_px as f32 / n as f32
        };

        let pool = OrbDissolve::orb_pool(&from);
        assert!(!pool.is_empty(), "orb pool should be non-empty");

        let render_at = |t: f32| -> RgbaImage {
            let orbs = OrbDissolve::gpu_orbs(&from, &cfg, t);
            let (front, code) = OrbDissolve::sweep_params(&cfg, t);
            renderer.render_orbs(&from, &to, ORB_DISSOLVE_WGSL, t, &orbs, front, code)
        };

        // t=0: band off the entry edge -> ≈ from.
        let f0 = render_at(0.0);
        let d0_from = mean_rgb_diff(&f0, &from);
        eprintln!("gpu t=0: mean diff to from = {d0_from:.2}");
        assert!(d0_from < 3.0, "gpu t=0 should be ≈ from");

        // t=1: band off the exit edge -> ≈ to.
        let f1 = render_at(1.0);
        let d1_to = mean_rgb_diff(&f1, &to);
        eprintln!("gpu t=1: mean diff to to = {d1_to:.2}");
        assert!(d1_to < 3.0, "gpu t=1 should be ≈ to");

        // Monotone sweep: to-fraction grows across t.
        let mut prev = -1.0f32;
        for k in 0..=10 {
            let t = k as f32 / 10.0;
            let frac = to_fraction(&render_at(t));
            eprintln!("gpu t={t:.1}: to-fraction = {frac:.3}");
            assert!(
                frac >= prev - 0.05,
                "gpu to-fraction must not retreat: t={t} frac={frac} prev={prev}"
            );
            prev = frac;
        }
    }

    /// **Cache-effectiveness test (issue #13).** Rendering a whole clip — many
    /// frames at a fixed size with a fixed shader — must compile the pipeline and
    /// allocate the per-size textures/read-back buffer exactly once, not per
    /// frame. We assert the four resource caches each hold a single entry after
    /// the clip, and that adding a second distinct size grows only the sized
    /// caches (the pipeline caches stay at one). This is the mechanism the issue
    /// asks for: shader compilation and texture allocation stay off the hot path.
    #[test]
    fn caches_resources_across_a_clip() {
        use crate::transitions::crossfade::CROSSFADE_WGSL;
        use crate::transitions::orb_dissolve::{OrbConfig, OrbDissolve, ORB_DISSOLVE_WGSL};

        let Some(renderer) = GpuRenderer::new() else {
            eprintln!("SKIP caches_resources_across_a_clip: no GPU adapter available");
            return;
        };

        let (w, h) = (48u32, 32u32);
        let from = gradient(w, h, [30, 60, 90, 255]);
        let to = gradient(w, h, [180, 120, 40, 255]);
        let cfg = OrbConfig::default();

        // A 16-frame clip on both paths. Same shader, same size every frame.
        for k in 0..16 {
            let t = k as f32 / 15.0;
            let _ = renderer.render(&from, &to, CROSSFADE_WGSL, t);
            let orbs = OrbDissolve::gpu_orbs(&from, &cfg, t);
            let (front, code) = OrbDissolve::sweep_params(&cfg, t);
            let _ = renderer.render_orbs(&from, &to, ORB_DISSOLVE_WGSL, t, &orbs, front, code);
        }

        let (pipes, cf_sizes, orb_sizes) = renderer.cache_sizes();
        eprintln!("after 16-frame clip: pipes={pipes} cf_sizes={cf_sizes} orb_sizes={orb_sizes}");
        assert_eq!(
            pipes, 2,
            "crossfade and orb shaders must each compile exactly once (2 in the shared cache)"
        );
        assert_eq!(cf_sizes, 1, "crossfade size must allocate exactly once");
        assert_eq!(orb_sizes, 1, "orb size must allocate exactly once");

        // A second, different size adds one sized entry per path but reuses the
        // already-compiled pipelines.
        let (w2, h2) = (24u32, 24u32);
        let from2 = gradient(w2, h2, [30, 60, 90, 255]);
        let to2 = gradient(w2, h2, [180, 120, 40, 255]);
        let _ = renderer.render(&from2, &to2, CROSSFADE_WGSL, 0.5);
        let orbs2 = OrbDissolve::gpu_orbs(&from2, &cfg, 0.5);
        let (front2, code2) = OrbDissolve::sweep_params(&cfg, 0.5);
        let _ = renderer.render_orbs(&from2, &to2, ORB_DISSOLVE_WGSL, 0.5, &orbs2, front2, code2);

        let (pipes, cf_sizes, orb_sizes) = renderer.cache_sizes();
        eprintln!("after second size: pipes={pipes} cf_sizes={cf_sizes} orb_sizes={orb_sizes}");
        assert_eq!(
            pipes, 2,
            "second size must not recompile either shader (still 2 pipelines)"
        );
        assert_eq!(
            cf_sizes, 2,
            "second size must add one crossfade sized entry"
        );
        assert_eq!(orb_sizes, 2, "second size must add one orb sized entry");
    }

    /// **Core GPU seam test.** Mid-clip, the from/to seam in the base must be
    /// hidden under the orb band on the GPU path: along the seam line the pixels
    /// must be orb-painted (neither pure `from` nor pure `to`). Proves the GPU
    /// sweep hides the boundary like the CPU oracle.
    #[test]
    fn gpu_seam_is_covered_by_orbs() {
        use crate::transitions::orb_dissolve::{OrbConfig, OrbDissolve, ORB_DISSOLVE_WGSL};

        let Some(renderer) = GpuRenderer::new() else {
            eprintln!("SKIP gpu_seam_is_covered_by_orbs: no GPU adapter available");
            return;
        };
        eprintln!(
            "orb-dissolve GPU seam test running on adapter: {}",
            renderer.adapter_name()
        );

        // Solid green `from` / blue `to`: orbs carry green, the base behind the
        // front is blue, so the seam being green ⇒ orbs hide it.
        let (w, h) = (96u32, 96u32);
        let mut from = RgbaImage::new(w, h);
        for px in from.pixels_mut() {
            *px = Rgba([0, 220, 0, 255]);
        }
        let mut to = RgbaImage::new(w, h);
        for px in to.pixels_mut() {
            *px = Rgba([0, 0, 255, 255]);
        }
        let cfg = OrbConfig::default();
        let t = 0.5;

        let orbs = OrbDissolve::gpu_orbs(&from, &cfg, t);
        let (front, code) = OrbDissolve::sweep_params(&cfg, t);
        let frame = renderer.render_orbs(&from, &to, ORB_DISSOLVE_WGSL, t, &orbs, front, code);

        let seam_x = ((front.clamp(0.0, 1.0) * w as f32) as u32).min(w - 1);
        let mut green = 0u32;
        for y in 0..h {
            let p = frame.get_pixel(seam_x, y).0;
            if p[1] > 120 && p[1] > p[2] {
                green += 1;
            }
        }
        let frac = green as f32 / h as f32;
        eprintln!("gpu seam_x={seam_x} (front={front:.3}): orb (green) coverage = {frac:.3}");
        assert!(
            frac > 0.7,
            "gpu seam must be hidden under the orb band (covered {frac:.2})"
        );
    }

    /// **Cache-reuse correctness (issue #13, state transition).** The per-size
    /// textures and read-back buffer are now allocated once and reused for every
    /// subsequent same-sized frame. The risk that introduces is *stale data*: a
    /// reused `from`/`to` texture or read-back buffer that still carries the
    /// previous frame's bytes. This renders two frames with **different inputs at
    /// the same size** on one renderer (so the second frame hits the cached
    /// resources) and asserts each output matches a *fresh* renderer's single
    /// render of the same inputs. If reuse leaked stale texture/buffer contents,
    /// the second frame would diverge from the fresh oracle.
    #[test]
    fn cached_resources_do_not_leak_previous_frame() {
        let Some(renderer) = GpuRenderer::new() else {
            eprintln!("SKIP cached_resources_do_not_leak_previous_frame: no GPU adapter available");
            return;
        };

        let (w, h) = (40u32, 24u32);
        // Two visually distinct (from, to) pairs rendered back-to-back at one size.
        let from_a = gradient(w, h, [10, 200, 30, 255]);
        let to_a = gradient(w, h, [220, 10, 40, 255]);
        let from_b = gradient(w, h, [5, 15, 240, 255]);
        let to_b = gradient(w, h, [250, 240, 5, 255]);

        // Frame A allocates the cache; frame B reuses every cached resource.
        let a_cached = renderer.render(&from_a, &to_a, CROSSFADE_WGSL, 0.3);
        let b_cached = renderer.render(&from_b, &to_b, CROSSFADE_WGSL, 0.7);

        // Each cache entry must exist exactly once (reuse really happened).
        let (pipes, cf_sizes, _) = renderer.cache_sizes();
        assert_eq!(pipes, 1, "both frames must share one cached pipeline");
        assert_eq!(cf_sizes, 1, "both frames must share one cached size");

        // Oracle: a brand-new renderer that has never seen frame A's inputs, so
        // its textures/buffer start clean.
        let Some(fresh) = GpuRenderer::new() else {
            eprintln!("SKIP oracle leg: no second GPU adapter available");
            return;
        };
        let b_fresh = fresh.render(&from_b, &to_b, CROSSFADE_WGSL, 0.7);

        // The reused-resource render of frame B must equal the clean render of B.
        let max_diff = assert_within_tolerance(
            &b_fresh,
            &b_cached,
            "reused frame B must match a clean render",
        );
        eprintln!("reuse-vs-fresh frame B: max per-channel diff = {max_diff}");

        // And frame A itself must still be a valid crossfade (sanity: the two
        // frames are genuinely different, so this isn't a no-op comparison).
        let a_fresh = fresh.render(&from_a, &to_a, CROSSFADE_WGSL, 0.3);
        assert_within_tolerance(
            &a_fresh,
            &a_cached,
            "frame A must also match a clean render",
        );
        assert!(
            a_cached
                .pixels()
                .zip(b_cached.pixels())
                .any(|(p, q)| p.0 != q.0),
            "frames A and B must actually differ (otherwise reuse is untested)"
        );
    }

    /// **Cache-reuse correctness, orb path (issue #13, state transition).** The
    /// symmetric counterpart of [`cached_resources_do_not_leak_previous_frame`]
    /// for the orb-dissolve path: `orb_sized_cache` reuses the same
    /// `write_texture_data`/`SizedResources` mechanism, so it carries the same
    /// stale-data risk on its reused `from`/`to` textures and read-back buffer.
    /// `caches_resources_across_a_clip` only checks orb *entry counts*, never orb
    /// reuse *correctness*. Here we render two orb frames with **different inputs
    /// at the same size** back-to-back on one renderer (the second hits the cached
    /// resources) and assert each matches a *fresh* renderer's single render of
    /// the same inputs. If reuse leaked stale texture/buffer contents, the second
    /// frame would diverge from the fresh oracle.
    #[test]
    fn cached_orb_resources_do_not_leak_previous_frame() {
        use crate::transitions::orb_dissolve::{OrbConfig, OrbDissolve, ORB_DISSOLVE_WGSL};

        let Some(renderer) = GpuRenderer::new() else {
            eprintln!(
                "SKIP cached_orb_resources_do_not_leak_previous_frame: no GPU adapter available"
            );
            return;
        };

        let (w, h) = (40u32, 24u32);
        // Two visually distinct (from, to) pairs rendered back-to-back at one size.
        let from_a = gradient(w, h, [10, 200, 30, 255]);
        let to_a = gradient(w, h, [220, 10, 40, 255]);
        let from_b = gradient(w, h, [5, 15, 240, 255]);
        let to_b = gradient(w, h, [250, 240, 5, 255]);
        let cfg = OrbConfig::default();

        let orb_frame = |r: &GpuRenderer, from: &RgbaImage, to: &RgbaImage, t: f32| {
            let orbs = OrbDissolve::gpu_orbs(from, &cfg, t);
            let (front, code) = OrbDissolve::sweep_params(&cfg, t);
            r.render_orbs(from, to, ORB_DISSOLVE_WGSL, t, &orbs, front, code)
        };

        // Frame A allocates the orb cache; frame B reuses every cached resource.
        let a_cached = orb_frame(&renderer, &from_a, &to_a, 0.3);
        let b_cached = orb_frame(&renderer, &from_b, &to_b, 0.7);

        // The orb caches must each hold exactly one entry (reuse really happened).
        let (pipes, _, orb_sizes) = renderer.cache_sizes();
        assert_eq!(pipes, 1, "both frames must share one cached orb pipeline");
        assert_eq!(orb_sizes, 1, "both frames must share one cached orb size");

        // Oracle: a brand-new renderer that has never seen frame A's inputs, so
        // its textures/buffer start clean.
        let Some(fresh) = GpuRenderer::new() else {
            eprintln!("SKIP oracle leg: no second GPU adapter available");
            return;
        };

        // The reused-resource render of frame B must equal the clean render of B.
        let b_fresh = orb_frame(&fresh, &from_b, &to_b, 0.7);
        let max_diff = assert_within_tolerance(
            &b_fresh,
            &b_cached,
            "reused orb frame B must match a clean render",
        );
        eprintln!("orb reuse-vs-fresh frame B: max per-channel diff = {max_diff}");

        // And frame A must still match a clean render too — and the two frames
        // must genuinely differ, so this isn't a no-op comparison.
        let a_fresh = orb_frame(&fresh, &from_a, &to_a, 0.3);
        assert_within_tolerance(
            &a_fresh,
            &a_cached,
            "orb frame A must also match a clean render",
        );
        assert!(
            a_cached
                .pixels()
                .zip(b_cached.pixels())
                .any(|(p, q)| p.0 != q.0),
            "orb frames A and B must actually differ (otherwise reuse is untested)"
        );
    }

    /// **Pipeline cache keying (issue #13, branch/keying).** The pipeline cache is
    /// keyed by shader *source string*. A second, byte-distinct shader source must
    /// produce a second cache entry (a separate compilation), while re-rendering
    /// the first source must not. The existing clip test only ever feeds one
    /// shader per path, so the keying itself is otherwise unexercised. The second
    /// source here is `CROSSFADE_WGSL` plus a trailing comment: behaviorally
    /// identical, lexically different — so we also confirm its output still matches
    /// the CPU oracle, proving the variant compiled and ran rather than silently
    /// collided with the first entry.
    #[test]
    fn distinct_shader_sources_each_get_a_pipeline() {
        let Some(renderer) = GpuRenderer::new() else {
            eprintln!("SKIP distinct_shader_sources_each_get_a_pipeline: no GPU adapter available");
            return;
        };

        let (w, h) = (32u32, 20u32);
        let from = gradient(w, h, [20, 90, 60, 255]);
        let to = gradient(w, h, [180, 30, 120, 255]);

        // Byte-distinct but behaviorally identical: a trailing comment changes the
        // source string (cache key) without changing the rendered result.
        let variant_wgsl = format!("{CROSSFADE_WGSL}\n// cache-key variant\n");
        assert_ne!(
            variant_wgsl, CROSSFADE_WGSL,
            "the variant must be a distinct source string"
        );

        // First source compiles -> one pipeline.
        let _ = renderer.render(&from, &to, CROSSFADE_WGSL, 0.5);
        let (pipes, ..) = renderer.cache_sizes();
        assert_eq!(pipes, 1, "first shader source -> one pipeline");

        // Re-rendering the SAME source must not add a pipeline.
        let _ = renderer.render(&from, &to, CROSSFADE_WGSL, 0.6);
        let (pipes, ..) = renderer.cache_sizes();
        assert_eq!(pipes, 1, "same source re-render must not recompile");

        // A distinct source string -> a second pipeline entry.
        let variant_out = renderer.render(&from, &to, &variant_wgsl, 0.5);
        let (pipes, ..) = renderer.cache_sizes();
        assert_eq!(
            pipes, 2,
            "a byte-distinct shader source must compile a second pipeline"
        );

        // The variant actually ran correctly (matches the CPU crossfade oracle),
        // proving it got its own working pipeline rather than a stale collision.
        let cpu = Crossfade.render_cpu(&from, &to, 0.5);
        let max_diff = assert_within_tolerance(
            &cpu,
            &variant_out,
            "variant shader must match CPU crossfade",
        );
        eprintln!("variant-shader vs CPU: max per-channel diff = {max_diff}");
    }

    /// **Sized-cache revisit (issue #13, A→B→A state transition).** The clip test
    /// proves a *new* size grows the sized cache. It never returns to an earlier
    /// size, so it can't catch a keying bug that re-allocates on revisit. Here we
    /// render at size A, then B, then A again, and assert the crossfade sized cache
    /// holds exactly two entries throughout the revisit — the second render at A
    /// reused its entry rather than inserting a duplicate or a third.
    #[test]
    fn revisiting_a_size_reuses_its_cached_entry() {
        let Some(renderer) = GpuRenderer::new() else {
            eprintln!("SKIP revisiting_a_size_reuses_its_cached_entry: no GPU adapter available");
            return;
        };

        let (wa, ha) = (40u32, 24u32);
        let (wb, hb) = (24u32, 40u32);
        let from_a = gradient(wa, ha, [10, 40, 80, 255]);
        let to_a = gradient(wa, ha, [200, 90, 20, 255]);
        let from_b = gradient(wb, hb, [10, 40, 80, 255]);
        let to_b = gradient(wb, hb, [200, 90, 20, 255]);

        let _ = renderer.render(&from_a, &to_a, CROSSFADE_WGSL, 0.4);
        let (.., cf_sizes, _) = renderer.cache_sizes();
        assert_eq!(cf_sizes, 1, "size A -> one sized entry");

        let _ = renderer.render(&from_b, &to_b, CROSSFADE_WGSL, 0.4);
        let (.., cf_sizes, _) = renderer.cache_sizes();
        assert_eq!(cf_sizes, 2, "size B -> a second sized entry");

        // Revisit A: must reuse, not grow to three.
        let _ = renderer.render(&from_a, &to_a, CROSSFADE_WGSL, 0.6);
        let (pipes, cf_sizes, _) = renderer.cache_sizes();
        eprintln!("after A,B,A: pipes={pipes} cf_sizes={cf_sizes}");
        assert_eq!(
            cf_sizes, 2,
            "revisiting size A must reuse its entry, not allocate a third"
        );
        assert_eq!(pipes, 1, "one shader source -> still one pipeline");
    }
}
