// Derived from noclip.website (https://github.com/magcius/noclip.website),
// src/gx/gx_render.ts at commit 2cebac2a5d1d7c44535397a14f51e764ca823767.
// Copyright (c) 2018 Jasper St. Pierre. MIT License: see THIRD-PARTY.md.
// Translated from TypeScript (noclip's Gfx layer) to Rust (wgpu) for rustcube-gx.
//! The wgpu side: GX blend / depth / cull state to pipeline state, the
//! pipeline cache (one pipeline per material and vertex layout), textures
//! with their mip chains, GX samplers, and the draw of a list of
//! [`GpuDraw`]s. Runs on WebGPU, on WebGL2 through wgpu's fallback, and on
//! native backends for tests.
use crate::api::FastMap;
use crate::enums::*;
use crate::material::GxMaterial;
use crate::params::{self, MaterialParams, SamplerDesc, SceneParams, TextureId};
use crate::shader::{self, MaterialHacks};
use crate::vtx::{InputFormat, LoadedVertexLayout};
use std::sync::Arc;
use wgpu::util::DeviceExt;

pub const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Offsets into the per-frame uniform buffer are 256-byte aligned.
const UNIFORM_ALIGN: usize = 256;
const MATERIAL_SLOT: usize = params::MATERIAL_PARAMS_SIZE.div_ceil(UNIFORM_ALIGN) * UNIFORM_ALIGN;
const DRAW_SLOT: usize = params::DRAW_PARAMS_SIZE.div_ceil(UNIFORM_ALIGN) * UNIFORM_ALIGN;
const SCENE_SLOT: usize = UNIFORM_ALIGN;

pub fn translate_compare(c: u32) -> wgpu::CompareFunction {
    match c {
        compare_type::NEVER => wgpu::CompareFunction::Never,
        compare_type::LESS => wgpu::CompareFunction::Less,
        compare_type::EQUAL => wgpu::CompareFunction::Equal,
        compare_type::LEQUAL => wgpu::CompareFunction::LessEqual,
        compare_type::GREATER => wgpu::CompareFunction::Greater,
        compare_type::NEQUAL => wgpu::CompareFunction::NotEqual,
        compare_type::GEQUAL => wgpu::CompareFunction::GreaterEqual,
        _ => wgpu::CompareFunction::Always,
    }
}

pub fn translate_cull(c: u32) -> Option<wgpu::Face> {
    match c {
        cull_mode::FRONT => Some(wgpu::Face::Front),
        cull_mode::BACK => Some(wgpu::Face::Back),
        _ => None,
    }
}

fn blend_common(f: u32) -> wgpu::BlendFactor {
    match f {
        blend_factor::ZERO => wgpu::BlendFactor::Zero,
        blend_factor::ONE => wgpu::BlendFactor::One,
        blend_factor::SRCALPHA => wgpu::BlendFactor::SrcAlpha,
        blend_factor::INVSRCALPHA => wgpu::BlendFactor::OneMinusSrcAlpha,
        blend_factor::DSTALPHA => wgpu::BlendFactor::DstAlpha,
        _ => wgpu::BlendFactor::OneMinusDstAlpha,
    }
}

/// GX's SRCCLR means the destination color for the source factor...
pub fn translate_blend_src(f: u32) -> wgpu::BlendFactor {
    match f {
        blend_factor::SRCCLR => wgpu::BlendFactor::Dst,
        blend_factor::INVSRCCLR => wgpu::BlendFactor::OneMinusDst,
        _ => blend_common(f),
    }
}

/// ...and the source color for the destination factor.
pub fn translate_blend_dst(f: u32) -> wgpu::BlendFactor {
    match f {
        blend_factor::SRCCLR => wgpu::BlendFactor::Src,
        blend_factor::INVSRCCLR => wgpu::BlendFactor::OneMinusSrc,
        _ => blend_common(f),
    }
}

/// Blend and write mask of a material. LOGIC blending is not available on
/// the web; like noclip it draws as no blend.
pub fn translate_color_target(
    m: &GxMaterial,
    format: wgpu::TextureFormat,
) -> wgpu::ColorTargetState {
    let r = &m.rop;
    let comp = |src, dst, op| wgpu::BlendComponent {
        src_factor: src,
        dst_factor: dst,
        operation: op,
    };
    let c = match r.blend_mode {
        blend_mode::BLEND => comp(
            translate_blend_src(r.blend_src),
            translate_blend_dst(r.blend_dst),
            wgpu::BlendOperation::Add,
        ),
        blend_mode::SUBTRACT => comp(
            wgpu::BlendFactor::One,
            wgpu::BlendFactor::One,
            wgpu::BlendOperation::ReverseSubtract,
        ),
        _ => comp(
            wgpu::BlendFactor::One,
            wgpu::BlendFactor::Zero,
            wgpu::BlendOperation::Add,
        ),
    };
    let mut mask = wgpu::ColorWrites::empty();
    if r.color_update {
        mask |= wgpu::ColorWrites::COLOR;
    }
    if r.alpha_update {
        mask |= wgpu::ColorWrites::ALPHA;
    }
    wgpu::ColorTargetState {
        format,
        blend: Some(wgpu::BlendState { color: c, alpha: c }),
        write_mask: mask,
    }
}

pub fn translate_wrap(w: u32) -> wgpu::AddressMode {
    match w {
        wrap_mode::CLAMP => wgpu::AddressMode::ClampToEdge,
        wrap_mode::MIRROR => wgpu::AddressMode::MirrorRepeat,
        _ => wgpu::AddressMode::Repeat,
    }
}

/// (texel filter, mip filter).
pub fn translate_tex_filter(f: u32) -> (wgpu::FilterMode, wgpu::MipmapFilterMode) {
    use wgpu::{FilterMode as F, MipmapFilterMode as M};
    match f {
        tex_filter::NEAR => (F::Nearest, M::Nearest),
        tex_filter::LINEAR => (F::Linear, M::Nearest),
        tex_filter::NEAR_MIP_NEAR => (F::Nearest, M::Nearest),
        tex_filter::LIN_MIP_NEAR => (F::Linear, M::Nearest),
        tex_filter::NEAR_MIP_LIN => (F::Nearest, M::Linear),
        _ => (F::Linear, M::Linear),
    }
}

pub fn translate_max_anisotropy(a: u32) -> u16 {
    match a {
        anisotropy::_2 => 2,
        anisotropy::_4 => 4,
        _ => 1,
    }
}

fn vertex_format(f: InputFormat) -> wgpu::VertexFormat {
    match f {
        InputFormat::Unorm8x4 => wgpu::VertexFormat::Unorm8x4,
        InputFormat::F32(1) => wgpu::VertexFormat::Float32,
        InputFormat::F32(2) => wgpu::VertexFormat::Float32x2,
        InputFormat::F32(3) => wgpu::VertexFormat::Float32x3,
        InputFormat::F32(_) => wgpu::VertexFormat::Float32x4,
    }
}

/// One draw for [`GxRenderer::render`].
pub struct GpuDraw<'a> {
    pub material: &'a GxMaterial,
    pub layout: &'a LoadedVertexLayout,
    /// Vertex data in `layout`.
    pub vertices: &'a [u8],
    /// Triangle list indices into `vertices`.
    pub indices: &'a [u32],
    pub material_params: &'a MaterialParams,
    pub draw_params: &'a params::DrawParams,
}

#[derive(Hash, PartialEq, Eq)]
struct PipelineKey {
    material: GxMaterial,
    layout: LoadedVertexLayout,
    hacks: MaterialHacks,
}

struct GpuTexture {
    view: wgpu::TextureView,
    /// An uploaded texture (`None` for an EFB copy, which `efb_copies` owns).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    texture: Option<wgpu::Texture>,
}

/// A draw ready to record: everything but its position matrices.
struct Prepared {
    /// Index into the pass's draws (for its matrices).
    draw: usize,
    pipeline: wgpu::RenderPipeline,
    textures: wgpu::BindGroup,
    material_offset: u32,
    vertex_offset: u64,
    /// The vertex layout's stride (draws with the same stride share a binding).
    stride: u64,
    first_index: u32,
    index_count: u32,
}

/// One prepared pass: its draws, and the uniform bind group, vertex, index
/// and draw-matrix buffers.
struct Pass {
    prepared: Vec<Prepared>,
    buffers: Option<(wgpu::BindGroup, wgpu::Buffer, wgpu::Buffer, wgpu::Buffer)>,
}

/// Most passes a recorded frame keeps buffers for (render-to-texture passes
/// and the picture); passes past it get buffers of their own each frame.
const GEO_SLOTS: usize = 32;

/// A pass's GPU buffers, kept from one recorded frame to the next: the
/// buffers grow when a frame needs more and are otherwise written in place,
/// and only the bytes that changed since the last frame are uploaded (level
/// geometry is the same frame after frame; the camera and objects move
/// through the matrices).
#[derive(Default)]
struct Geo {
    uniforms: Kept,
    vertices: Kept,
    indices: Kept,
    matrices: Kept,
    bind: Option<wgpu::BindGroup>,
}

/// A buffer and the bytes last written to it.
#[derive(Default)]
struct Kept {
    buffer: Option<wgpu::Buffer>,
    last: Vec<u8>,
}

impl Kept {
    /// A buffer of at least `size` bytes; true when it is a new one.
    fn reserve(
        &mut self,
        device: &wgpu::Device,
        size: usize,
        usage: wgpu::BufferUsages,
    ) -> (wgpu::Buffer, bool) {
        let size = size.max(4).next_multiple_of(4);
        match &self.buffer {
            Some(b) if b.size() >= size as u64 => (b.clone(), false),
            _ => {
                // Room to grow: frames vary a little in size.
                let b = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("gx kept"),
                    size: (size + size / 4).next_multiple_of(256) as u64,
                    usage,
                    mapped_at_creation: false,
                });
                if let Some(old) = self.buffer.take() {
                    old.destroy();
                }
                self.buffer = Some(b.clone());
                self.last.clear();
                (b, true)
            }
        }
    }

    /// Make the buffer hold `data`, uploading only the span that differs.
    fn sync(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        data: &[u8],
        usage: wgpu::BufferUsages,
    ) -> (wgpu::Buffer, bool) {
        let (b, new) = self.reserve(device, data.len(), usage);
        if let Some((lo, hi)) = changed(&self.last, data) {
            if hi > lo {
                let mut chunk = data[lo..hi].to_vec();
                chunk.resize(chunk.len().next_multiple_of(4), 0);
                queue.write_buffer(&b, lo as u64, &chunk);
            }
            self.last.clear();
            self.last.extend_from_slice(data);
        }
        (b, new)
    }
}

/// The span of `new` to write over `old` (4-byte aligned), or None when the
/// two are the same.
fn changed(old: &[u8], new: &[u8]) -> Option<(usize, usize)> {
    if old == new {
        return None;
    }
    let same = |a: &u8, b: &u8| a == b;
    let lo = old.iter().zip(new).take_while(|(a, b)| same(a, b)).count() / 4 * 4;
    let hi = if old.len() == new.len() {
        new.len()
            - old
                .iter()
                .rev()
                .zip(new.iter().rev())
                .take_while(|(a, b)| same(a, b))
                .count()
    } else {
        new.len()
    };
    // The end goes up to a whole word too: the upload is padded to 4 bytes,
    // and padding inside the data would zero bytes that didn't change (warped
    // meshes, wrong glyphs). Past the data's end the padding is harmless.
    let hi = hi.next_multiple_of(4).min(new.len());
    Some((lo, hi.max(lo)))
}

pub struct GxRenderer {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub format: wgpu::TextureFormat,
    pub hacks: MaterialHacks,
    uniform_layout: wgpu::BindGroupLayout,
    texture_layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    pipelines: FastMap<Arc<PipelineKey>, wgpu::RenderPipeline>,
    textures: FastMap<TextureId, GpuTexture>,
    next_texture: TextureId,
    samplers: FastMap<SamplerDesc, wgpu::Sampler>,
    texture_binds: FastMap<[(Option<TextureId>, SamplerDesc); 8], wgpu::BindGroup>,
    white: wgpu::TextureView,
    depth: Vec<(wgpu::TextureView, wgpu::Texture, [u32; 2])>,
    /// Offscreen EFB for render-to-texture passes.
    efb: Option<wgpu::Texture>,
    /// Viewport for the next [`Self::render`] (x, y, w, h, near, far in
    /// target pixels); `None` covers the target.
    pub viewport: Option<[f32; 6]>,
    /// Passes of the frame drawn last (by its serial and draw range: an empty
    /// clear-only pass starts where the next real one does).
    passes: FastMap<(u64, usize, usize), Pass>,
    passes_of: u64,
    /// Recorded passes not yet submitted.
    pending: Vec<wgpu::CommandBuffer>,
    /// Textures made by EFB copies, by id (reused by the next copy).
    efb_copies: FastMap<TextureId, wgpu::Texture>,
    /// Buffers kept between recorded frames, by pass (see [`Geo`]).
    geo: Vec<Geo>,
    /// The next pass's slot in `geo` this frame.
    geo_next: usize,
}

impl GxRenderer {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let ub = |binding, size: usize| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: wgpu::BufferSize::new(size as u64),
            },
            count: None,
        };
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gx uniforms"),
            entries: &[
                ub(0, params::SCENE_PARAMS_SIZE),
                ub(1, params::MATERIAL_PARAMS_SIZE),
                ub(2, params::DRAW_PARAMS_SIZE),
            ],
        });
        let mut tex_entries = Vec::new();
        for i in 0..8u32 {
            tex_entries.push(wgpu::BindGroupLayoutEntry {
                binding: 2 * i,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
            tex_entries.push(wgpu::BindGroupLayoutEntry {
                binding: 2 * i + 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            });
        }
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gx textures"),
            entries: &tex_entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gx"),
            bind_group_layouts: &[Some(&uniform_layout), Some(&texture_layout)],
            immediate_size: 0,
        });
        let white = device
            .create_texture_with_data(
                &queue,
                &wgpu::TextureDescriptor {
                    label: Some("gx white"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                &[255; 4],
            )
            .create_view(&Default::default());
        Self {
            device,
            queue,
            format,
            hacks: MaterialHacks::default(),
            uniform_layout,
            texture_layout,
            pipeline_layout,
            pipelines: FastMap::default(),
            textures: FastMap::default(),
            next_texture: 0,
            samplers: FastMap::default(),
            texture_binds: FastMap::default(),
            white,
            depth: Vec::new(),
            efb: None,
            viewport: None,
            passes: FastMap::default(),
            passes_of: 0,
            pending: Vec::new(),
            efb_copies: FastMap::default(),
            geo: Vec::new(),
            geo_next: 0,
        }
    }

    /// Cache sizes (pipelines, textures, samplers, texture bind groups), for
    /// finding what a long run keeps.
    pub fn cache_counts(&self) -> [usize; 4] {
        [
            self.pipelines.len(),
            self.textures.len(),
            self.samplers.len(),
            self.texture_binds.len(),
        ]
    }

    /// Upload a decoded texture: RGBA8 levels, largest first. GX textures
    /// are stored as linear RGBA (Rgba8Unorm), like noclip; the output view
    /// decides sRGB encoding.
    pub fn create_texture(&mut self, levels: &[(u32, u32, Vec<u8>)]) -> TextureId {
        let id = self.next_texture;
        self.next_texture += 1;
        self.replace_texture(id, levels);
        id
    }

    /// Replace the contents of texture `id` (or create it under that id).
    pub fn replace_texture(&mut self, id: TextureId, levels: &[(u32, u32, Vec<u8>)]) {
        // An upload under this id replaces any EFB copy made there.
        self.efb_copies.remove(&id);
        let (w, h) = levels.first().map_or((1, 1), |l| (l.0.max(1), l.1.max(1)));
        // Keep only levels that halve properly, as wgpu requires.
        let mut data = Vec::new();
        let mut count = 0u32;
        for (k, (lw, lh, px)) in levels.iter().enumerate() {
            let (ew, eh) = ((w >> k).max(1), (h >> k).max(1));
            if *lw != ew || *lh != eh || px.len() != (ew * eh * 4) as usize {
                break;
            }
            data.extend_from_slice(px);
            count += 1;
        }
        let (w, h) = if count == 0 {
            data = vec![255; 4];
            count = 1;
            (1, 1)
        } else {
            (w, h)
        };
        let t = self.device.create_texture_with_data(
            &self.queue,
            &wgpu::TextureDescriptor {
                label: Some("gx texture"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: count,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &data,
        );
        let old = self.textures.insert(
            id,
            GpuTexture {
                view: t.create_view(&Default::default()),
                texture: Some(t),
            },
        );
        // In a browser a dropped texture lives until garbage collection, and
        // the game uploads textures again every frame: free the old one now.
        // (Native wgpu frees it on drop, once nothing in flight uses it.)
        #[cfg(target_arch = "wasm32")]
        if let Some(t) = old.and_then(|o| o.texture) {
            t.destroy();
        }
        #[cfg(not(target_arch = "wasm32"))]
        drop(old);
        self.next_texture = self.next_texture.max(id + 1);
        self.texture_binds
            .retain(|k, _| !k.iter().any(|(t, _)| *t == Some(id)));
    }

    pub fn destroy_texture(&mut self, id: TextureId) {
        self.textures.remove(&id);
        self.texture_binds
            .retain(|k, _| !k.iter().any(|(t, _)| *t == Some(id)));
    }

    fn sampler(&mut self, d: SamplerDesc) -> wgpu::Sampler {
        self.samplers
            .entry(d)
            .or_insert_with(|| {
                let (min, mip) = translate_tex_filter(d.min_filter);
                let (mag, _) = translate_tex_filter(d.mag_filter);
                let aniso = translate_max_anisotropy(d.max_aniso);
                // wgpu requires linear filtering for anisotropy.
                let aniso = if min == wgpu::FilterMode::Linear
                    && mag == wgpu::FilterMode::Linear
                    && mip == wgpu::MipmapFilterMode::Linear
                {
                    aniso
                } else {
                    1
                };
                self.device.create_sampler(&wgpu::SamplerDescriptor {
                    label: Some("gx sampler"),
                    address_mode_u: translate_wrap(d.wrap_s),
                    address_mode_v: translate_wrap(d.wrap_t),
                    mag_filter: mag,
                    min_filter: min,
                    mipmap_filter: mip,
                    lod_min_clamp: f32::from(d.min_lod_x16) / 16.0,
                    lod_max_clamp: (f32::from(d.max_lod_x16) / 16.0)
                        .max(f32::from(d.min_lod_x16) / 16.0),
                    anisotropy_clamp: aniso,
                    ..Default::default()
                })
            })
            .clone()
    }

    fn texture_bind(&mut self, p: &MaterialParams) -> wgpu::BindGroup {
        let key: [(Option<TextureId>, SamplerDesc); 8] = std::array::from_fn(|i| {
            let m = &p.texture_mappings[i];
            let t = m.texture.filter(|t| self.textures.contains_key(t));
            (t, m.sampler)
        });
        if let Some(b) = self.texture_binds.get(&key) {
            return b.clone();
        }
        let samplers: Vec<wgpu::Sampler> = key.iter().map(|(_, s)| self.sampler(*s)).collect();
        let views: Vec<wgpu::TextureView> = key
            .iter()
            .map(|(t, _)| {
                t.and_then(|t| self.textures.get(&t))
                    .map_or_else(|| self.white.clone(), |g| g.view.clone())
            })
            .collect();
        let mut entries = Vec::new();
        for i in 0..8 {
            entries.push(wgpu::BindGroupEntry {
                binding: 2 * i as u32,
                resource: wgpu::BindingResource::TextureView(&views[i]),
            });
            entries.push(wgpu::BindGroupEntry {
                binding: 2 * i as u32 + 1,
                resource: wgpu::BindingResource::Sampler(&samplers[i]),
            });
        }
        let b = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gx textures"),
            layout: &self.texture_layout,
            entries: &entries,
        });
        self.texture_binds.insert(key, b.clone());
        b
    }

    /// The pipeline for a material and vertex layout (built on first use).
    pub fn pipeline(
        &mut self,
        material: &GxMaterial,
        layout: &LoadedVertexLayout,
    ) -> wgpu::RenderPipeline {
        let key = PipelineKey {
            material: material.clone(),
            layout: layout.clone(),
            hacks: self.hacks,
        };
        if let Some(p) = self.pipelines.get(&key) {
            return p.clone();
        }
        let has = |i| layout.input(i).is_some();
        let program = shader::generate(material, self.hacks, &has);
        let module = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("gx material"),
                source: wgpu::ShaderSource::Wgsl(program.source.into()),
            });
        let attrs: Vec<wgpu::VertexAttribute> = layout
            .inputs
            .iter()
            .map(|l| wgpu::VertexAttribute {
                format: vertex_format(l.format),
                offset: l.offset as u64,
                shader_location: l.input.location(),
            })
            .collect();
        let r = &material.rop;
        let pipeline = self
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("gx material"),
                layout: Some(&self.pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: layout.stride.max(4) as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &attrs,
                    })],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    front_face: wgpu::FrontFace::Cw,
                    cull_mode: translate_cull(material.cull_mode),
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH,
                    depth_write_enabled: Some(r.depth_test && r.depth_write),
                    depth_compare: Some(if r.depth_test {
                        translate_compare(r.depth_func)
                    } else {
                        wgpu::CompareFunction::Always
                    }),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(translate_color_target(material, self.format))],
                }),
                multiview_mask: None,
                cache: None,
            });
        self.pipelines.insert(Arc::new(key), pipeline.clone());
        pipeline
    }

    fn depth_view(&mut self, size: [u32; 2]) -> wgpu::TextureView {
        // One depth buffer per size: render-to-texture passes (640x480)
        // and the picture (the window's size) alternate every frame, and
        // a single slot made two new depth textures a picture.
        if let Some((v, _, _)) = self.depth.iter().find(|d| d.2 == size) {
            return v.clone();
        }
        let t = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gx depth"),
            size: wgpu::Extent3d {
                width: size[0].max(1),
                height: size[1].max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let v = t.create_view(&Default::default());
        // A few sizes at most (a resized window leaves its old one behind).
        if self.depth.len() >= 3 {
            self.depth.remove(0);
        }
        self.depth.push((v.clone(), t, size));
        v
    }

    /// Draw `draws` in order into `target` (of `size`). `clear` clears color
    /// (and depth) first; `None` keeps what is there.
    pub fn render(
        &mut self,
        target: &wgpu::TextureView,
        size: [u32; 2],
        clear: Option<[f64; 4]>,
        scene: &SceneParams,
        draws: &[GpuDraw<'_>],
    ) {
        self.render_pass(None, target, size, clear, scene, draws);
        self.flush();
    }

    /// Submit the passes recorded since the last flush, in order.
    fn flush(&mut self) {
        if !self.pending.is_empty() {
            self.queue.submit(self.pending.drain(..));
        }
    }

    /// Everything a pass needs except the draws' position matrices.
    fn prepare(&mut self, scene: &SceneParams, draws: &[GpuDraw<'_>], slot: Option<usize>) -> Pass {
        let mut uniforms = params::fill_scene_params(scene);
        uniforms.reserve(SCENE_SLOT + draws.len() * MATERIAL_SLOT);
        uniforms.resize(SCENE_SLOT, 0);
        let mut vertices: Vec<u8> =
            Vec::with_capacity(draws.iter().map(|d| d.vertices.len() + 64).sum());
        let mut indices: Vec<u32> = Vec::new();
        let mut prepared = Vec::new();
        let mut last_material: Option<(&MaterialParams, u32)> = None;
        let mut last_pipeline: Option<(&GxMaterial, &LoadedVertexLayout, wgpu::RenderPipeline)> =
            None;
        for (k, d) in draws.iter().enumerate() {
            if !d.material.is_valid() || d.indices.is_empty() || d.vertices.is_empty() {
                continue;
            }
            // The same material and layout as the draw before (shared through
            // the GX state's caches): its pipeline, without hashing both again.
            let pipeline = match &last_pipeline {
                Some((m, l, p)) if std::ptr::eq(*m, d.material) && std::ptr::eq(*l, d.layout) => {
                    p.clone()
                }
                _ => {
                    let p = self.pipeline(d.material, d.layout);
                    last_pipeline = Some((d.material, d.layout, p.clone()));
                    p
                }
            };
            let textures = self.texture_bind(d.material_params);
            let material_offset = match last_material {
                Some((p, o)) if p == d.material_params => o,
                _ => {
                    let o = uniforms.len() as u32;
                    params::fill_material_params_into(d.material_params, &mut uniforms);
                    uniforms.resize(o as usize + MATERIAL_SLOT, 0);
                    last_material = Some((d.material_params, o));
                    o
                }
            };
            // Each draw's vertices start on a multiple of its stride, so one
            // binding of the buffer reaches them through a base vertex.
            let stride = d.layout.stride.max(1);
            let vertex_offset = vertices.len().div_ceil(stride) * stride;
            vertices.resize(vertex_offset, 0);
            vertices.extend_from_slice(d.vertices);
            let first_index = indices.len() as u32;
            indices.extend_from_slice(d.indices);
            prepared.push(Prepared {
                draw: k,
                pipeline,
                textures,
                material_offset,
                vertex_offset: vertex_offset as u64,
                stride: stride as u64,
                first_index,
                index_count: d.indices.len() as u32,
            });
        }
        if prepared.is_empty() {
            return Pass {
                prepared,
                buffers: None,
            };
        }
        let mats = (prepared.len() * DRAW_SLOT).max(DRAW_SLOT);
        let usage = |u| u | wgpu::BufferUsages::COPY_DST;
        let (bufs, fresh) = match slot.and_then(|s| self.geo.get_mut(s).map(|g| (s, g))) {
            // A pass of a recorded frame: its slot's buffers, kept between
            // frames, take only the bytes that changed since the last frame.
            Some((_, g)) => {
                let (u, a) = g.uniforms.sync(
                    &self.device,
                    &self.queue,
                    &uniforms,
                    usage(wgpu::BufferUsages::UNIFORM),
                );
                let (v, b) = g.vertices.sync(
                    &self.device,
                    &self.queue,
                    &vertices,
                    usage(wgpu::BufferUsages::VERTEX),
                );
                let (i, c) = g.indices.sync(
                    &self.device,
                    &self.queue,
                    bytemuck::cast_slice(&indices),
                    usage(wgpu::BufferUsages::INDEX),
                );
                let (m, d) =
                    g.matrices
                        .reserve(&self.device, mats, usage(wgpu::BufferUsages::UNIFORM));
                let _ = (b, c);
                ((u, v, i, m), (a || d).then_some(slot).flatten())
            }
            None => {
                let mk = |label, contents: &[u8], usage| {
                    self.device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some(label),
                            contents,
                            usage,
                        })
                };
                let u = mk("gx uniforms", &uniforms, wgpu::BufferUsages::UNIFORM);
                let v = mk("gx vertices", &vertices, wgpu::BufferUsages::VERTEX);
                let i = mk(
                    "gx indices",
                    bytemuck::cast_slice(&indices),
                    wgpu::BufferUsages::INDEX,
                );
                let m = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("gx draw matrices"),
                    size: mats as u64,
                    usage: usage(wgpu::BufferUsages::UNIFORM),
                    mapped_at_creation: false,
                });
                ((u, v, i, m), None)
            }
        };
        let (u, v, i, m) = bufs;
        // The bind group names the uniform and matrix buffers: made again
        // only when one of them was replaced by a bigger one.
        let kept = slot
            .and_then(|s| self.geo.get(s))
            .and_then(|g| g.bind.clone());
        let bind = match (kept, fresh) {
            (Some(b), None) => b,
            _ => {
                let b = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("gx uniforms"),
                    layout: &self.uniform_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: binding(&u, params::SCENE_PARAMS_SIZE),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: binding(&u, params::MATERIAL_PARAMS_SIZE),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: binding(&m, params::DRAW_PARAMS_SIZE),
                        },
                    ],
                });
                if let Some(g) = slot.and_then(|s| self.geo.get_mut(s)) {
                    g.bind = Some(b.clone());
                }
                b
            }
        };
        Pass {
            prepared,
            buffers: Some((bind, v, i, m)),
        }
    }

    /// Draw a pass. With `key` (a recorded frame's serial and the pass's
    /// draw range), a pass drawn before is drawn again from what it prepared
    /// then, with only the position matrices written anew: pictures between
    /// game frames move matrices and nothing else.
    fn render_pass(
        &mut self,
        key: Option<(u64, usize, usize)>,
        target: &wgpu::TextureView,
        size: [u32; 2],
        clear: Option<[f64; 4]>,
        scene: &SceneParams,
        draws: &[GpuDraw<'_>],
    ) {
        let pass = match key.and_then(|k| self.passes.remove(&k)) {
            Some(p) => p,
            None => {
                let slot = key.map(|_| self.geo_next).filter(|s| *s < GEO_SLOTS);
                if let Some(s) = slot {
                    self.geo_next += 1;
                    if self.geo.len() <= s {
                        self.geo.resize_with(s + 1, Geo::default);
                    }
                }
                self.prepare(scene, draws, slot)
            }
        };
        if let Some((_, _, _, m)) = &pass.buffers {
            let mut mats = Vec::with_capacity(pass.prepared.len() * DRAW_SLOT);
            for p in &pass.prepared {
                let at = mats.len();
                if let Some(d) = draws.get(p.draw) {
                    params::fill_draw_params_into(d.draw_params, &mut mats);
                }
                mats.resize(at + DRAW_SLOT, 0);
            }
            self.queue.write_buffer(m, 0, &mats);
        }
        let depth = self.depth_view(size);
        let viewport = self.viewport.and_then(|v| {
            let w = size[0] as f32;
            let h = size[1] as f32;
            let x = v[0].clamp(0.0, w);
            let y = v[1].clamp(0.0, h);
            let vw = v[2].clamp(1.0, w - x).max(1.0);
            let vh = v[3].clamp(1.0, h - y).max(1.0);
            (x < w && y < h).then_some([x, y, vw, vh, v[4].min(v[5]), v[5].max(v[4])])
        });
        let buffers = pass.buffers.as_ref().map(|(b, v, i, _)| (b, v, i));
        let all = 0..pass.prepared.len();
        let cmd = record_slice(
            &self.device,
            target,
            &depth,
            clear,
            viewport,
            buffers,
            &pass.prepared,
            all,
        );
        // Submitted with the frame's other passes (one submit a frame).
        self.pending.push(cmd);
        if let Some(k) = key {
            self.passes.insert(k, pass);
        }
    }

    /// Apply a recorded frame's texture uploads and draw it into `target`.
    /// Render-to-texture passes (each ending in an EFB copy to a texture)
    /// draw into an offscreen EFB first, and the copied rectangle becomes the
    /// texture later draws sample; what follows the last copy is the picture.
    pub fn render_frame(
        &mut self,
        frame: &crate::api::Frame,
        target: &wgpu::TextureView,
        size: [u32; 2],
    ) {
        for u in &frame.uploads {
            self.replace_texture(u.id, &u.levels);
        }
        if frame.serial != self.passes_of || !frame.uploads.is_empty() {
            self.passes.clear();
            self.passes_of = frame.serial;
            self.geo_next = 0;
        }
        let c = frame.clear_color;
        let clear_color = [c[0] as f64, c[1] as f64, c[2] as f64, c[3] as f64];
        let mut clear = Some(clear_color);
        let mut start = 0;
        let efb_size = [params::EFB_WIDTH as u32, 480];
        for copy in &frame.tex_copies {
            let end = copy.draw.min(frame.draws.len());
            let efb = self.efb_target(efb_size);
            let view = efb.create_view(&Default::default());
            self.draw_range(frame, start..end, &view, efb_size, clear.take());
            self.copy_to_texture(&efb, copy);
            if copy.clear {
                clear = Some(clear_color);
            }
            start = end;
        }
        self.draw_range(
            frame,
            start..frame.draws.len(),
            target,
            size,
            Some(clear_color),
        );
        self.flush();
    }

    /// The offscreen EFB for render-to-texture passes.
    fn efb_target(&mut self, size: [u32; 2]) -> wgpu::Texture {
        if let Some(t) = &self.efb {
            if t.width() == size[0] && t.height() == size[1] {
                return t.clone();
            }
        }
        let t = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gx efb"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        self.efb = Some(t.clone());
        t
    }

    /// GXCopyTex: the EFB rectangle becomes texture `copy.id`.
    /// shortcut: the copy's own scale and texture format are not applied
    /// (full resolution, the target's colour format).
    fn copy_to_texture(&mut self, efb: &wgpu::Texture, copy: &crate::api::TexCopy) {
        self.flush();
        let [x, y, w, h] = copy.src;
        let x = x.min(efb.width().saturating_sub(1));
        let y = y.min(efb.height().saturating_sub(1));
        let w = w.min(efb.width() - x).max(1);
        let h = h.min(efb.height() - y).max(1);
        // The same copy again (the picture drawn again between game frames,
        // or the next frame's shadow): into the texture it made before, so
        // bind groups that sample it see the new copy.
        let reuse = self
            .efb_copies
            .get(&copy.id)
            .filter(|t| t.width() == w && t.height() == h)
            .cloned();
        let fresh = reuse.is_none();
        let t = reuse.unwrap_or_else(|| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("gx efb copy"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: efb,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            t.as_image_copy(),
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([enc.finish()]);
        let id = copy.id;
        if !fresh {
            return;
        }
        self.efb_copies.insert(id, t.clone());
        self.textures.insert(
            id,
            GpuTexture {
                view: t.create_view(&Default::default()),
                texture: None,
            },
        );
        self.next_texture = self.next_texture.max(id + 1);
        self.texture_binds
            .retain(|k, _| !k.iter().any(|(t, _)| *t == Some(id)));
    }

    /// Draw `frame.draws[range]` into `target`. Runs of draws with the same
    /// projection and viewport share one pass; `clear` clears the first.
    fn draw_range(
        &mut self,
        frame: &crate::api::Frame,
        range: std::ops::Range<usize>,
        target: &wgpu::TextureView,
        size: [u32; 2],
        mut clear: Option<[f64; 4]>,
    ) {
        // Only the game's own per-texture LOD bias: mips follow the real output size (as a PC port would).
        let lod_bias = 0.0;
        let mut start = range.start;
        while start < range.end || clear.is_some() {
            let first = frame.draws.get(start).filter(|_| start < range.end);
            let mut end = start;
            while end < range.end
                && first.is_some_and(|f| {
                    f.projection == frame.draws[end].projection
                        && f.viewport == frame.draws[end].viewport
                })
            {
                end += 1;
            }
            let scene = SceneParams {
                projection: first.map_or(SceneParams::default().projection, |f| f.projection),
                lod_bias,
            };
            let draws: Vec<GpuDraw<'_>> = frame.draws[start..end]
                .iter()
                .map(|d| GpuDraw {
                    material: &d.material,
                    layout: &d.layout,
                    vertices: &d.vertices,
                    indices: &d.indices,
                    material_params: &d.material_params,
                    draw_params: &d.draw_params,
                })
                .collect();
            self.viewport = first.map(|f| {
                let (sx, sy) = (size[0] as f32 / params::EFB_WIDTH, size[1] as f32 / 480.0);
                let v = f.viewport;
                [
                    v[0] * sx,
                    v[1] * sy,
                    v[2] * sx,
                    v[3] * sy,
                    v[4].clamp(0.0, 1.0),
                    v[5].clamp(0.0, 1.0),
                ]
            });
            let key = (frame.serial != 0).then_some((frame.serial, start, end));
            self.render_pass(key, target, size, clear.take(), &scene, &draws);
            if end == start {
                break;
            }
            start = end;
        }
        self.viewport = None;
    }
}

fn binding(buffer: &wgpu::Buffer, size: usize) -> wgpu::BindingResource<'_> {
    wgpu::BindingResource::Buffer(wgpu::BufferBinding {
        buffer,
        offset: 0,
        size: wgpu::BufferSize::new(size as u64),
    })
}

/// Record `prepared[range]` as one render pass (clearing first with `clear`).
#[allow(clippy::too_many_arguments)]
fn record_slice(
    device: &wgpu::Device,
    target: &wgpu::TextureView,
    depth: &wgpu::TextureView,
    clear: Option<[f64; 4]>,
    viewport: Option<[f32; 6]>,
    buffers: Option<(&wgpu::BindGroup, &wgpu::Buffer, &wgpu::Buffer)>,
    prepared: &[Prepared],
    range: std::ops::Range<usize>,
) -> wgpu::CommandBuffer {
    let mut enc = device.create_command_encoder(&Default::default());
    {
        let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("gx"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: match clear {
                        Some(c) => wgpu::LoadOp::Clear(wgpu::Color {
                            r: c[0],
                            g: c[1],
                            b: c[2],
                            a: c[3],
                        }),
                        None => wgpu::LoadOp::Load,
                    },
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: if clear.is_some() {
                        wgpu::LoadOp::Clear(1.0)
                    } else {
                        wgpu::LoadOp::Load
                    },
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if let Some([x, y, w, h, near, far]) = viewport {
            rp.set_viewport(x, y, w, h, near, far);
        }
        if let Some((bind, v, i)) = buffers {
            // A pipeline, textures or vertex buffer the draw before already
            // set isn't set again; draws of one vertex layout share a binding
            // and start at a base vertex.
            rp.set_index_buffer(i.slice(..), wgpu::IndexFormat::Uint32);
            let mut pipeline: Option<&wgpu::RenderPipeline> = None;
            let mut textures: Option<&wgpu::BindGroup> = None;
            let mut stride = 0u64;
            for (n, p) in prepared
                .iter()
                .enumerate()
                .take(range.end)
                .skip(range.start)
            {
                if pipeline != Some(&p.pipeline) {
                    rp.set_pipeline(&p.pipeline);
                    pipeline = Some(&p.pipeline);
                }
                let draw_offset = (n * DRAW_SLOT) as u32;
                rp.set_bind_group(0, bind, &[0, p.material_offset, draw_offset]);
                if textures != Some(&p.textures) {
                    rp.set_bind_group(1, &p.textures, &[]);
                    textures = Some(&p.textures);
                }
                if p.stride != stride {
                    rp.set_vertex_buffer(0, v.slice(..));
                    stride = p.stride;
                }
                let base = (p.vertex_offset / p.stride.max(1)) as i32;
                rp.draw_indexed(p.first_index..p.first_index + p.index_count, base, 0..1);
            }
        }
    }
    enc.finish()
}

#[cfg(test)]
mod kept_tests {
    use super::changed;

    #[test]
    fn uploads_only_what_changed() {
        let a = [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        assert_eq!(changed(&a, &a), None);
        let mut b = a;
        b[6] = 0;
        // Whole words around the change.
        assert_eq!(changed(&a, &b), Some((4, 8)));
        // Longer or shorter: to the end of the new bytes.
        assert_eq!(
            changed(&a, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]),
            Some((8, 11))
        );
        assert_eq!(changed(&a, &a[..6]), Some((4, 6)));
        assert_eq!(changed(&[], &a), Some((0, 10)));
    }

    #[test]
    fn a_padded_upload_never_zeroes_unchanged_bytes() {
        // As Kept::sync uploads: data[lo..hi] padded with zeros to 4 bytes,
        // over a buffer holding the old bytes. The buffer must equal the new.
        let old: Vec<u8> = (1..=40).collect();
        for at in 0..old.len() {
            for len in [old.len(), old.len() + 3, at + 1] {
                let mut new: Vec<u8> = (1..=len as u8).collect();
                if let Some(b) = new.get_mut(at) {
                    *b = 0xEE;
                }
                let mut gpu = old.clone();
                gpu.resize(64, 0);
                if let Some((lo, hi)) = changed(&old, &new) {
                    let mut chunk = new[lo..hi].to_vec();
                    chunk.resize(chunk.len().next_multiple_of(4), 0);
                    gpu[lo..lo + chunk.len()].copy_from_slice(&chunk);
                }
                assert_eq!(&gpu[..new.len()], &new[..], "change at {at}, length {len}");
            }
        }
    }
}
