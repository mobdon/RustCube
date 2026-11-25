// Partly derived from noclip.website (https://github.com/magcius/noclip.website),
// src/gx/DDraw.ts and src/gx/GXMaterialBuilder.ts at commit
// 2cebac2a5d1d7c44535397a14f51e764ca823767.
// Copyright (c) 2018 Jasper St. Pierre. MIT License: see THIRD-PARTY.md.
// Translated from TypeScript to Rust and extended for rustcube-gx.
//! A GX-API-shaped state machine for the recompiled game.
//!
//! Each `GXSet*` / `GXLoad*` / `GXBegin` call has a method of the same name
//! in snake case. Draws are recorded, not submitted: [`Gx::take_frame`]
//! returns the frame's draws and texture uploads, and `GxRenderer::
//! render_frame` (in `gpu.rs`) draws them. Nothing here needs a GPU, so
//! the recording runs headless in tests.
//!
//! Immediate-mode vertices (`GXPosition3f32` etc.) are written to a byte
//! FIFO exactly as the CPU writes them to the GX FIFO, then go through the
//! same vertex loader as display lists. `GXCallDisplayList` runs draw
//! commands, indexed loads and BP/CP/XF register loads (decoded with the
//! public register layouts).
use crate::enums::*;
use crate::material::{GxMaterial, GxMaterialBuilder, Light, SwapTable, DEFAULT_SWAP_TABLES};
use crate::params::{
    self, color_kind, DrawParams, MaterialParams, Mtx34, SamplerDesc, TextureId, TextureMapping,
    IDENTITY34,
};
use crate::texture::{self, TextureInput};
use crate::vtx::{
    self, GxArray, LoadedVertexLayout, VatFormat, Vcd, VtxAttrFmt, VtxDesc, VtxLoader, ATTR_COUNT,
};
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

/// A map with a multiply-rotate hash: the GX lookups (vertex loaders,
/// textures, palettes) run thousands of times a frame on small keys, where
/// the standard SipHash was most of their cost. The keys come from the game's
/// own GX state, not from anyone who could pick colliding ones.
pub type FastMap<K, V> = HashMap<K, V, BuildHasherDefault<FastHasher>>;

#[derive(Default)]
pub struct FastHasher(u64);

impl Hasher for FastHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_u64(u64::from(b));
        }
    }
    fn write_u8(&mut self, v: u8) {
        self.write_u64(u64::from(v));
    }
    fn write_u16(&mut self, v: u16) {
        self.write_u64(u64::from(v));
    }
    fn write_u32(&mut self, v: u32) {
        self.write_u64(u64::from(v));
    }
    fn write_u64(&mut self, v: u64) {
        self.0 = (self.0.rotate_left(5) ^ v).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
    fn write_usize(&mut self, v: usize) {
        self.write_u64(v as u64);
    }
}

/// Textures resolved since the last texture change kept for the next draws.
const RECENT_TEXTURES: usize = 32;
use std::sync::Arc;

/// Read access to the game's memory (GameCube addresses).
pub trait GxMemory {
    fn read(&self, addr: u32, len: usize) -> Option<&[u8]>;
    /// A read that only sets up a view (a vertex array's whole window); the
    /// bytes actually used are reported with [`Self::touch`].
    fn view(&self, addr: u32, len: usize) -> Option<&[u8]> {
        self.read(addr, len)
    }
    /// `len` bytes at `addr` were used through a [`Self::view`].
    fn touch(&self, _addr: u32, _len: usize) {}
}

/// Main RAM as one slice starting at physical address 0; cached
/// (0x8...) and uncached (0xC...) addresses map to it.
pub struct Ram<'a>(pub &'a [u8]);

impl GxMemory for Ram<'_> {
    fn read(&self, addr: u32, len: usize) -> Option<&[u8]> {
        let p = (addr & 0x3FFF_FFFF) as usize;
        self.0.get(p..p.checked_add(len)?)
    }
}

/// `GXTevMode` presets for [`Gx::set_tev_op`].
pub mod tev_mode {
    pub const MODULATE: u32 = 0;
    pub const DECAL: u32 = 1;
    pub const BLEND: u32 = 2;
    pub const REPLACE: u32 = 3;
    pub const PASSCLR: u32 = 4;
}

/// `GXTexObj`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TexObj {
    pub image_addr: u32,
    pub width: u32,
    pub height: u32,
    pub format: u32,
    pub wrap_s: u32,
    pub wrap_t: u32,
    pub mipmap: bool,
    pub min_filter: u32,
    pub mag_filter: u32,
    pub min_lod: f32,
    pub max_lod: f32,
    pub lod_bias: f32,
    pub max_aniso: u32,
    /// TLUT name for C4/C8/C14X2.
    pub tlut_name: Option<u32>,
}

impl TexObj {
    /// `GXInitTexObj`.
    pub fn new(
        image_addr: u32,
        width: u32,
        height: u32,
        format: u32,
        wrap_s: u32,
        wrap_t: u32,
        mipmap: bool,
    ) -> Self {
        Self {
            image_addr,
            width,
            height,
            format,
            wrap_s,
            wrap_t,
            mipmap,
            min_filter: if mipmap {
                tex_filter::LIN_MIP_LIN
            } else {
                tex_filter::LINEAR
            },
            mag_filter: tex_filter::LINEAR,
            min_lod: 0.0,
            max_lod: if mipmap { 10.0 } else { 0.0 },
            lod_bias: 0.0,
            max_aniso: anisotropy::_1,
            tlut_name: None,
        }
    }

    /// `GXInitTexObjCI`.
    #[allow(clippy::too_many_arguments)]
    pub fn new_ci(
        image_addr: u32,
        width: u32,
        height: u32,
        format: u32,
        wrap_s: u32,
        wrap_t: u32,
        mipmap: bool,
        tlut_name: u32,
    ) -> Self {
        Self {
            tlut_name: Some(tlut_name),
            ..Self::new(image_addr, width, height, format, wrap_s, wrap_t, mipmap)
        }
    }

    /// `GXInitTexObjLOD`.
    #[allow(clippy::too_many_arguments)]
    pub fn init_lod(
        &mut self,
        min_filter: u32,
        mag_filter: u32,
        min_lod: f32,
        max_lod: f32,
        lod_bias: f32,
        _bias_clamp: bool,
        _edge_lod: bool,
        max_aniso: u32,
    ) {
        self.min_filter = min_filter;
        self.mag_filter = mag_filter;
        self.min_lod = min_lod;
        self.max_lod = max_lod;
        self.lod_bias = lod_bias;
        self.max_aniso = max_aniso;
    }
}

/// `GXTlutObj`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TlutObj {
    pub addr: u32,
    pub format: u32,
    pub entries: u32,
}

/// One recorded draw.
#[derive(Debug, Clone)]
pub struct DrawCmd {
    pub material: Arc<GxMaterial>,
    pub layout: Arc<LoadedVertexLayout>,
    pub vertices: Vec<u8>,
    pub indices: Vec<u32>,
    pub material_params: MaterialParams,
    pub draw_params: DrawParams,
    /// Column-major, wgpu depth range.
    pub projection: [[f32; 4]; 4],
    /// x, y, width, height, near, far in EFB pixels (640x528).
    pub viewport: [f32; 6],
    /// x, y, width, height in EFB pixels.
    pub scissor: [u32; 4],
}

/// A decoded texture the renderer must hold under `id`.
#[derive(Debug, Clone)]
pub struct TextureUpload {
    pub id: TextureId,
    /// RGBA8 levels, largest first.
    pub levels: Vec<(u32, u32, Vec<u8>)>,
}

/// One GXCopyTex: after `draw` draws, the EFB rectangle `src` (x, y, width,
/// height in EFB pixels) becomes texture `id`; `clear` clears the EFB after.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TexCopy {
    pub draw: usize,
    pub src: [u32; 4],
    pub id: TextureId,
    pub clear: bool,
}

/// What a frame drew.
#[derive(Debug, Clone, Default)]
pub struct Frame {
    pub clear_color: [f32; 4],
    pub draws: Vec<DrawCmd>,
    /// EFB copies to a texture (GXCopyTex: render to texture, e.g. shadows),
    /// in order. Draws before the last one belong to those passes, not to the
    /// picture copied to the screen.
    pub tex_copies: Vec<TexCopy>,
    pub uploads: Vec<TextureUpload>,
    /// Which recorded frame this is (from 1; 0 for frames built by hand),
    /// so a renderer drawing it again can reuse what it prepared.
    pub serial: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct TexKey {
    addr: u32,
    format: u32,
    width: u32,
    height: u32,
    levels: u32,
    tlut: Option<(u32, u32, u32)>,
}

struct Immediate {
    prim: u32,
    vtx_fmt: u32,
    count: u32,
    fifo: Vec<u8>,
    /// Bytes per vertex for this format, to know when the batch is full.
    vertex_size: usize,
}

/// The GX state machine.
pub struct Gx {
    vcd: Vcd,
    vat: [VatFormat; 8],
    arrays: [(u32, usize); 16],
    loaders: FastMap<(Vcd, [VatFormat; 8]), Arc<VtxLoader>>,
    // Transform unit.
    xf_matrices: Vec<f32>,
    post_tex_mtx: [Mtx34; 20],
    current_mtx: u32,
    /// Each texgen's matrix (GXTexMtx: TEXMTX0 30 ... IDENTITY 60), from
    /// XF_MATRIXINDEX0/1 beside the position matrix index.
    tex_mtx_idx: [u32; 8],
    projection: [[f32; 4]; 4],
    viewport: [f32; 6],
    scissor: [u32; 4],
    // Material.
    mat: GxMaterialBuilder,
    params: MaterialParams,
    swap_tables: [SwapTable; 4],
    swap_sel: [(u32, u32); 16],
    num_tev_stages: usize,
    num_tex_gens: usize,
    num_ind_stages: usize,
    num_chans: usize,
    dst_alpha: Option<u8>,
    // Textures.
    tex_maps: [Option<TexObj>; 8],
    tluts: FastMap<u32, TlutObj>,
    /// Decoded textures: key -> (id, invalidate generation). An invalidate
    /// makes them stale; the next use decodes again under the same id, so the
    /// renderer replaces the texture instead of keeping a new one each frame.
    /// Texture id, the invalidation generation it was checked in, and a hash
    /// of its bytes (so an invalidation re-decodes only what changed).
    tex_cache: FastMap<TexKey, (TextureId, u64, u64)>,
    tex_gen: u64,
    /// Bumped whenever what a texture object resolves to may change (texture
    /// invalidation, an EFB copy to a new address, a different palette).
    tex_epoch: u64,
    /// Texture objects resolved this epoch, most recent first: a draw's
    /// textures are nearly always the last draw's.
    tex_recent: [Option<(TexObj, TextureMapping)>; RECENT_TEXTURES],
    /// Textures made by EFB copies, by destination (physical) address: a
    /// texture loaded from there is the copy, not the bytes in RAM.
    efb_copies: FastMap<u32, TextureId>,
    next_texture: TextureId,
    // Register mirrors for display-list loads.
    regs: crate::vtx::DisplayListRegisters,
    dual_tex: [u32; 8],
    chan_regs: [u32; 4],
    // XF projection (0x1020..=0x1026) and viewport (0x101A..=0x101F) words.
    proj_regs: [f32; 7],
    vp_regs: [f32; 6],
    /// Palettes loaded into TMEM through BP 0x64/0x65: offset -> (address, entries).
    tlut_loads: FastMap<u32, (u32, u32)>,
    /// Inside a `GXCallDisplayList` (lists can't call lists).
    in_call: bool,
    // Output.
    immediate: Option<Immediate>,
    /// No register was written since the last FIFO draw, and that draw is a
    /// 3D one: the next FIFO draw joins it (see [`Self::run_draws`]).
    joinable: bool,
    /// Registers whose last write was decoded and still stands: writing the
    /// same value again changes nothing, so it is dropped and the next draw
    /// can still join. A register is dropped from here when state its
    /// decoding reads changes elsewhere. BP by address; CP and XF hold the
    /// value. The `set_*`/`load_*` methods don't keep these: a caller mixing
    /// them with FIFO writes of an unchanged value keeps the method's state.
    bp_fresh: [bool; 0x100],
    cp_seen: [Option<u32>; 0x100],
    xf_seen: Vec<Option<u32>>,
    frame: Frame,
    /// What this state machine ignored, with counts: unknown FIFO opcodes
    /// and BP/CP/XF registers it doesn't model ("BP 0x45", ...).
    /// Counted by kind and register (see [`Gx::unhandled`]); a string per
    /// write cost more than the rest of the register handling.
    unhandled: FastMap<(u8, u32), u64>,
    /// The last vertex loader and its key (consecutive draws share one).
    last_loader: Option<(LoaderKey, Arc<VtxLoader>, Arc<LoadedVertexLayout>)>,
    /// The last draw's material and what it was built from: consecutive draws
    /// mostly share one, and building it cost more than the rest of a draw.
    last_material: Option<(GxMaterialBuilder, MatKey, Arc<GxMaterial>)>,
}

impl Default for Gx {
    fn default() -> Self {
        Self::new()
    }
}

fn mtx_from(words: &[f32]) -> Mtx34 {
    let w = |i: usize| words.get(i).copied().unwrap_or(0.0);
    [
        [w(0), w(1), w(2), w(3)],
        [w(4), w(5), w(6), w(7)],
        [w(8), w(9), w(10), w(11)],
    ]
}

fn rgba8(c: [u8; 4]) -> [f32; 4] {
    c.map(|v| f32::from(v) / 255.0)
}

impl Gx {
    /// GX after `GXInit`: identity matrices, one TEV stage, default swap
    /// tables, z test LEQUAL with writes, no blending.
    pub fn new() -> Self {
        let mut xf = vec![0.0; 0x400];
        for m in 0..(0x400 / 12) {
            for r in 0..3 {
                xf[m * 12 + r * 4 + r] = 1.0;
            }
        }
        let mut gx = Self {
            vcd: [VtxDesc::default(); ATTR_COUNT],
            vat: [[VtxAttrFmt::default(); ATTR_COUNT]; 8],
            arrays: [(0, 0); 16],
            loaders: FastMap::default(),
            xf_matrices: xf,
            post_tex_mtx: [IDENTITY34; 20],
            current_mtx: 0,
            tex_mtx_idx: [tex_gen_matrix::IDENTITY; 8],
            projection: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            viewport: [0.0, 0.0, 640.0, 480.0, 0.0, 1.0],
            scissor: [0, 0, 640, 480],
            mat: GxMaterialBuilder::new(),
            params: MaterialParams::default(),
            swap_tables: DEFAULT_SWAP_TABLES,
            swap_sel: [(0, 0); 16],
            num_tev_stages: 1,
            num_tex_gens: 1,
            num_ind_stages: 0,
            num_chans: 1,
            dst_alpha: None,
            tex_maps: [None; 8],
            tluts: FastMap::default(),
            tex_cache: FastMap::default(),
            tex_gen: 0,
            tex_epoch: 0,
            tex_recent: [None; RECENT_TEXTURES],
            next_texture: 0,
            efb_copies: FastMap::default(),
            regs: {
                let mut r = crate::vtx::DisplayListRegisters::default();
                r.init_gx();
                r
            },
            dual_tex: [0; 8],
            chan_regs: [0; 4],
            proj_regs: [1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0],
            vp_regs: [320.0, -240.0, 16_777_215.0, 662.0, 582.0, 16_777_215.0],
            tlut_loads: FastMap::default(),
            in_call: false,
            immediate: None,
            joinable: false,
            bp_fresh: [false; 0x100],
            cp_seen: [None; 0x100],
            xf_seen: vec![None; 0x1100],
            frame: Frame::default(),
            unhandled: FastMap::default(),
            last_loader: None,
            last_material: None,
        };
        gx.mat.set_num_chans(1);
        gx.mat.set_tex_coord_gen(
            0,
            tex_gen_type::MTX2x4,
            tex_gen_src::TEX0,
            tex_gen_matrix::IDENTITY,
            false,
            post_tex_gen_matrix::PTIDENTITY,
        );
        gx.mat.set_cull_mode(cull_mode::BACK);
        gx
    }

    /// Hand over the frame recorded so far and start a new one.
    pub fn take_frame(&mut self) -> Frame {
        let clear = self.frame.clear_color;
        let mut f = std::mem::take(&mut self.frame);
        self.frame.clear_color = clear;
        // Growing the list a draw at a time copied it over and over.
        self.frame.draws.reserve(f.draws.len());
        static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        f.serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        f
    }

    // ---- vertex format ---------------------------------------------------

    pub fn clear_vtx_desc(&mut self) {
        self.vcd = [VtxDesc::default(); ATTR_COUNT];
    }

    /// `GXSetVtxDesc`. GX_VA_NBT sets the normal with NBT data.
    pub fn set_vtx_desc(&mut self, attr_id: u32, ty: u32) {
        let a = if attr_id == attr::_NBT {
            attr::NRM
        } else {
            attr_id
        };
        if let Some(d) = self.vcd.get_mut(a as usize) {
            d.ty = ty;
        }
    }

    /// `GXSetVtxAttrFmt`.
    pub fn set_vtx_attr_fmt(
        &mut self,
        vtx_fmt: u32,
        attr_id: u32,
        cnt: u32,
        comp_type: u32,
        frac: u32,
    ) {
        let a = if attr_id == attr::_NBT {
            attr::NRM
        } else {
            attr_id
        };
        if let Some(f) = self
            .vat
            .get_mut(vtx_fmt as usize & 7)
            .and_then(|v| v.get_mut(a as usize))
        {
            *f = VtxAttrFmt {
                comp_type,
                comp_cnt: cnt,
                comp_shift: frac,
            };
        }
    }

    /// `GXSetArray` for POS..TEX7 (array 0 = POS).
    pub fn set_array(&mut self, attr_id: u32, base: u32, stride: u8) {
        if let Some(i) = attr_id.checked_sub(attr::POS) {
            if let Some(a) = self.arrays.get_mut(i as usize) {
                *a = (base, usize::from(stride));
            }
        }
    }

    pub fn invalidate_vtx_cache(&mut self) {}

    /// What this state machine ignored, with counts: unknown FIFO opcodes
    /// and BP/CP/XF registers it doesn't model ("BP 0x45", ...).
    pub fn unhandled(&self) -> std::collections::BTreeMap<String, u64> {
        self.unhandled
            .iter()
            .map(|(&(k, a), &n)| {
                let name = match k {
                    b'o' => format!("opcode {a:#04x}"),
                    b'C' => format!("CP {a:#04x}"),
                    b'B' => format!("BP {a:#04x}"),
                    _ => format!("XF {a:#06x}"),
                };
                (name, n)
            })
            .collect()
    }

    fn loader(&mut self) -> (Arc<VtxLoader>, Arc<LoadedVertexLayout>) {
        let key = (self.vcd, self.vat);
        if let Some((k, l, layout)) = &self.last_loader {
            if *k == key {
                return (l.clone(), layout.clone());
            }
        }
        let l = self.loader_for(key);
        let layout = Arc::new(l.layout.clone());
        self.last_loader = Some((key, l.clone(), layout.clone()));
        (l, layout)
    }

    fn loader_for(&mut self, key: (Vcd, [VatFormat; 8])) -> Arc<VtxLoader> {
        self.loaders
            .entry(key)
            .or_insert_with(|| {
                let vat = std::array::from_fn(|i| Some(self.vat[i]));
                Arc::new(VtxLoader::new_multi(vat, self.vcd))
            })
            .clone()
    }

    fn array_views<'m>(
        &self,
        mem: &'m dyn GxMemory,
        used: &'m [std::cell::Cell<usize>; ATTR_COUNT],
    ) -> [GxArray<'m>; ATTR_COUNT] {
        let mut out = [GxArray::default(); ATTR_COUNT];
        for a in attr::POS..=attr::TEX7 {
            let (base, stride) = self.arrays[(a - attr::POS) as usize];
            if base == 0 && stride == 0 {
                continue;
            }
            // The array runs to the end of RAM; reads past it give zeros.
            let mut len = 0x40_0000usize;
            let data = loop {
                if let Some(d) = mem.view(base, len) {
                    break d;
                }
                len /= 2;
                if len == 0 {
                    break &[][..];
                }
            };
            out[a as usize] = GxArray {
                data,
                stride,
                used: Some(&used[a as usize]),
            };
        }
        out
    }

    // ---- immediate mode --------------------------------------------------

    /// `GXBegin`. The batch ends (and is drawn) when `count` vertices have
    /// been written, or at [`Self::end`].
    pub fn begin(&mut self, prim: u32, vtx_fmt: u32, count: u16) {
        let vertex_size = vtx::source_vertex_size(&self.vat[vtx_fmt as usize & 7], &self.vcd);
        let mut fifo = Vec::with_capacity(3 + vertex_size * usize::from(count));
        fifo.push((prim as u8 & 0xF8) | (vtx_fmt as u8 & 7));
        fifo.extend(count.to_be_bytes());
        self.immediate = Some(Immediate {
            prim,
            vtx_fmt,
            count: u32::from(count),
            fifo,
            vertex_size,
        });
    }

    fn push(&mut self, bytes: &[u8]) {
        if let Some(im) = &mut self.immediate {
            im.fifo.extend_from_slice(bytes);
        }
    }

    pub fn position3f32(&mut self, x: f32, y: f32, z: f32) {
        for v in [x, y, z] {
            self.push(&v.to_be_bytes());
        }
    }
    pub fn position2f32(&mut self, x: f32, y: f32) {
        for v in [x, y] {
            self.push(&v.to_be_bytes());
        }
    }
    pub fn position3s16(&mut self, x: i16, y: i16, z: i16) {
        for v in [x, y, z] {
            self.push(&v.to_be_bytes());
        }
    }
    pub fn position3u16(&mut self, x: u16, y: u16, z: u16) {
        for v in [x, y, z] {
            self.push(&v.to_be_bytes());
        }
    }
    pub fn position2s16(&mut self, x: i16, y: i16) {
        for v in [x, y] {
            self.push(&v.to_be_bytes());
        }
    }
    pub fn position3s8(&mut self, x: i8, y: i8, z: i8) {
        self.push(&[x as u8, y as u8, z as u8]);
    }
    pub fn position1x8(&mut self, i: u8) {
        self.push(&[i]);
    }
    pub fn position1x16(&mut self, i: u16) {
        self.push(&i.to_be_bytes());
    }
    pub fn normal3f32(&mut self, x: f32, y: f32, z: f32) {
        self.position3f32(x, y, z);
    }
    pub fn normal3s16(&mut self, x: i16, y: i16, z: i16) {
        self.position3s16(x, y, z);
    }
    pub fn normal3s8(&mut self, x: i8, y: i8, z: i8) {
        self.position3s8(x, y, z);
    }
    pub fn normal1x8(&mut self, i: u8) {
        self.push(&[i]);
    }
    pub fn normal1x16(&mut self, i: u16) {
        self.push(&i.to_be_bytes());
    }
    pub fn color4u8(&mut self, r: u8, g: u8, b: u8, a: u8) {
        self.push(&[r, g, b, a]);
    }
    pub fn color3u8(&mut self, r: u8, g: u8, b: u8) {
        self.push(&[r, g, b]);
    }
    pub fn color1u32(&mut self, c: u32) {
        self.push(&c.to_be_bytes());
    }
    pub fn color1u16(&mut self, c: u16) {
        self.push(&c.to_be_bytes());
    }
    pub fn color1x8(&mut self, i: u8) {
        self.push(&[i]);
    }
    pub fn color1x16(&mut self, i: u16) {
        self.push(&i.to_be_bytes());
    }
    pub fn tex_coord2f32(&mut self, s: f32, t: f32) {
        self.position2f32(s, t);
    }
    pub fn tex_coord1f32(&mut self, s: f32) {
        self.push(&s.to_be_bytes());
    }
    pub fn tex_coord2s16(&mut self, s: i16, t: i16) {
        self.position2s16(s, t);
    }
    pub fn tex_coord2u16(&mut self, s: u16, t: u16) {
        self.push(&s.to_be_bytes());
        self.push(&t.to_be_bytes());
    }
    pub fn tex_coord2s8(&mut self, s: i8, t: i8) {
        self.push(&[s as u8, t as u8]);
    }
    pub fn tex_coord2u8(&mut self, s: u8, t: u8) {
        self.push(&[s, t]);
    }
    pub fn tex_coord1x8(&mut self, i: u8) {
        self.push(&[i]);
    }
    pub fn tex_coord1x16(&mut self, i: u16) {
        self.push(&i.to_be_bytes());
    }
    pub fn matrix_index1x8(&mut self, i: u8) {
        self.push(&[i]);
    }

    /// `GXEnd`: draw the batch written since [`Self::begin`].
    pub fn end(&mut self, mem: &dyn GxMemory) {
        let Some(im) = self.immediate.take() else {
            return;
        };
        let _ = (im.prim, im.vtx_fmt);
        let needed = 3 + im.vertex_size * im.count as usize;
        let mut fifo = im.fifo;
        fifo.resize(needed, 0);
        fifo.push(0);
        self.run_draws(&fifo, mem, false);
    }

    // ---- display lists -------------------------------------------------

    /// `GXCallDisplayList`: run the list at `addr` (`size` bytes).
    pub fn call_display_list(&mut self, addr: u32, size: u32, mem: &dyn GxMemory) {
        if self.in_call {
            return;
        }
        let Some(dl) = mem.read(addr, size as usize) else {
            return;
        };
        let dl = dl.to_vec();
        self.in_call = true;
        self.run_display_list(&dl, mem);
        self.in_call = false;
    }

    /// Bytes in the command at the start of `b` under the current vertex
    /// format, or `None` while it is incomplete. An unknown opcode is one
    /// byte (the hardware skips it with an error).
    pub fn command_len(&self, b: &[u8]) -> Option<usize> {
        let cmd = u32::from(*b.first()?);
        let be16 = |o: usize| {
            b.get(o..o + 2)
                .map(|x| usize::from(u16::from_be_bytes([x[0], x[1]])))
        };
        let n = match cmd {
            command::LOAD_BP_REG => 5,
            command::LOAD_CP_REG => 6,
            command::LOAD_XF_REG => 5 + 4 * (be16(1)? + 1),
            command::LOAD_INDX_A
            | command::LOAD_INDX_B
            | command::LOAD_INDX_C
            | command::LOAD_INDX_D => 5,
            command::CALL_DL => 9,
            _ if (command::DRAW_QUADS..=command::DRAW_POINTS + 7).contains(&cmd) => {
                let size = vtx::source_vertex_size(&self.vat[(cmd & 7) as usize], &self.vcd);
                3 + size * be16(1)?
            }
            _ => 1,
        };
        (n <= b.len()).then_some(n)
    }

    /// Run a display list already in hand.
    pub fn run_display_list(&mut self, dl: &[u8], mem: &dyn GxMemory) {
        let be16 = |o: usize| {
            dl.get(o..o + 2)
                .map_or(0, |b| u32::from(u16::from_be_bytes([b[0], b[1]])))
        };
        let be32 = |o: usize| {
            dl.get(o..o + 4)
                .map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
        };
        let mut i = 0usize;
        while i < dl.len() {
            let cmd = u32::from(dl[i]);
            match cmd {
                command::NOOP => i += 1,
                command::LOAD_BP_REG => {
                    self.bp_write(be32(i + 1));
                    i += 5;
                }
                command::LOAD_CP_REG => {
                    self.cp_write(u32::from(dl.get(i + 1).copied().unwrap_or(0)), be32(i + 2));
                    i += 6;
                }
                command::LOAD_XF_REG => {
                    let n = be16(i + 1) as usize + 1;
                    let addr = be16(i + 3);
                    for k in 0..n {
                        self.xf_write(addr + k as u32, be32(i + 5 + 4 * k));
                    }
                    i += 5 + 4 * n;
                }
                command::LOAD_INDX_A
                | command::LOAD_INDX_B
                | command::LOAD_INDX_C
                | command::LOAD_INDX_D => {
                    let array = 12 + ((cmd - command::LOAD_INDX_A) >> 3) as usize;
                    let index = be16(i + 1);
                    let al = be16(i + 3);
                    let (len, addr) = ((al >> 12) + 1, al & 0x0FFF);
                    let (base, stride) = self.arrays[array];
                    let src = base.wrapping_add(index * stride as u32);
                    if let Some(words) = mem.read(src, 4 * len as usize) {
                        let words: Vec<u32> = words
                            .chunks(4)
                            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
                            .collect();
                        for (k, w) in words.into_iter().enumerate() {
                            self.xf_write(addr + k as u32, w);
                        }
                    }
                    i += 5;
                }
                _ if (command::DRAW_QUADS..=command::DRAW_POINTS + 7).contains(&cmd) => {
                    let fmt = (cmd & 7) as usize;
                    let n = be16(i + 1) as usize;
                    if n == 0 {
                        // A draw of no vertices (the game sends thousands).
                        i += 3;
                        continue;
                    }
                    let size = vtx::source_vertex_size(&self.vat[fmt], &self.vcd);
                    let end = (i + 3 + size * n).min(dl.len());
                    let mut one = dl[i..end].to_vec();
                    one.resize(3 + size * n, 0);
                    one.push(0);
                    self.run_draws(&one, mem, true);
                    i = end;
                }
                command::CALL_DL => {
                    self.call_display_list(be32(i + 1), be32(i + 5), mem);
                    i += 9;
                }
                command::INVAL_VTX_CACHE => i += 1,
                _ => {
                    *self.unhandled.entry((b'o', cmd)).or_default() += 1;
                    break;
                }
            }
        }
    }

    /// Record the draw commands of `dl`. A FIFO draw (`fifo`) that follows
    /// another with no register write between them has the same material,
    /// matrices and vertex format, so its triangles are added to that draw:
    /// a model drawn as many short strips costs one draw's state, not one
    /// each. HUD and view-space draws stay apart (widescreen places each by
    /// its own extent). Callers that change state through the `set_*`/`load_*`
    /// methods between FIFO draws don't clear `joinable`; the FIFO's own
    /// state changes all arrive as BP/CP/XF writes, which do.
    fn run_draws(&mut self, dl: &[u8], mem: &dyn GxMemory, fifo: bool) {
        let (loader, layout) = self.loader();
        let used: [std::cell::Cell<usize>; ATTR_COUNT] = Default::default();
        let arrays = self.array_views(mem, &used);
        let join = std::mem::take(&mut self.joinable) && fifo;
        if let Some(last) = self
            .frame
            .draws
            .last_mut()
            .filter(|d| join && Arc::ptr_eq(&d.layout, &layout))
        {
            let base = (last.vertices.len() / layout.stride.max(1)) as u32;
            let mut data = loader.parse_display_list(dl, base);
            loader.load_vertex_data(&mut data, dl, &arrays);
            last.vertices.extend_from_slice(&data.vertices);
            last.indices.extend_from_slice(&data.indices);
            // The part of each array window this joined draw indexed.
            for a in attr::POS..=attr::TEX7 {
                let n = used[a as usize].get();
                if n > 0 {
                    mem.touch(self.arrays[(a - attr::POS) as usize].0, n);
                }
            }
            self.joinable = true;
            return;
        }
        let mut data = loader.parse_display_list(dl, 0);
        if data.indices.is_empty() {
            return;
        }
        loader.load_vertex_data(&mut data, dl, &arrays);
        // The part of each array window the draws indexed.
        for a in attr::POS..=attr::TEX7 {
            let n = used[a as usize].get();
            if n > 0 {
                mem.touch(self.arrays[(a - attr::POS) as usize].0, n);
            }
        }
        let (material, params, draw_params) = self.snapshot(mem);
        let d = DrawCmd {
            material,
            layout,
            vertices: data.vertices,
            indices: data.indices,
            material_params: params,
            draw_params,
            projection: self.wgpu_projection(),
            viewport: self.viewport,
            scissor: self.scissor,
        };
        self.joinable =
            fifo && !crate::widescreen::is_hud(&d) && !crate::widescreen::is_view_space(&d);
        self.frame.draws.push(d);
    }

    fn material(&mut self) -> Arc<GxMaterial> {
        let key = MatKey {
            counts: [
                self.num_tev_stages,
                self.num_tex_gens,
                self.num_ind_stages,
                self.num_chans,
            ],
            swap_sel: self.swap_sel,
            swap_tables: self.swap_tables,
            dst_alpha: self.dst_alpha,
            tex_mtx_idx: self.tex_mtx_idx,
            pn: self.vcd[attr::PNMTXIDX as usize].ty,
            tex_mtx: std::array::from_fn(|t| self.vcd[attr::TEX0MTXIDX as usize + t].ty),
        };
        if let Some((b, k, m)) = &self.last_material {
            if *k == key && *b == self.mat {
                return m.clone();
            }
        }
        let m = Arc::new(self.build_material());
        self.last_material = Some((self.mat.clone(), key, m.clone()));
        m
    }

    fn build_material(&self) -> GxMaterial {
        let mut b = self.mat.clone();
        b.set_num_tev_stages(self.num_tev_stages.max(1));
        b.set_num_tex_gens(self.num_tex_gens);
        for (i, &m) in self.tex_mtx_idx.iter().enumerate() {
            b.set_tex_gen_matrix(i, m);
        }
        b.set_num_ind_stages(self.num_ind_stages);
        b.set_num_chans(self.num_chans);
        for s in 0..self.num_tev_stages.max(1) {
            let (r, t) = self.swap_sel[s];
            b.set_tev_swap_mode(
                s as u32,
                Some(self.swap_tables[r as usize & 3]),
                Some(self.swap_tables[t as usize & 3]),
            );
        }
        b.set_dst_alpha(self.dst_alpha);
        let use_pn = self.vcd[attr::PNMTXIDX as usize].ty != attr_type::NONE;
        b.set_use_pn_mtx_idx(use_pn);
        for t in 0..8 {
            b.set_use_tex_mtx_idx(
                t,
                self.vcd[(attr::TEX0MTXIDX + t as u32) as usize].ty != attr_type::NONE,
            );
        }
        b.finish()
    }

    fn snapshot(&mut self, mem: &dyn GxMemory) -> (Arc<GxMaterial>, MaterialParams, DrawParams) {
        let material = self.material();
        let use_pn = self.vcd[attr::PNMTXIDX as usize].ty != attr_type::NONE;
        let mut params = self.params.clone();
        for i in 0..10 {
            params.tex_mtx[i] = mtx_from(&self.xf_matrices[(30 + 3 * i) * 4..]);
        }
        params.post_tex_mtx = self.post_tex_mtx;
        // Only the maps this draw samples: the others hold whatever an
        // earlier draw loaded, and resolving all eight cost a third of a draw.
        let mut used = [false; 8];
        let maps = material.tev_stages.iter().map(|s| s.tex_map);
        for m in maps.chain(material.ind_tex_stages.iter().map(|s| s.texture)) {
            if let Some(u) = used.get_mut(m as usize) {
                *u = true;
            }
        }
        for map in 0..8 {
            params.texture_mappings[map] = match self.tex_maps[map].filter(|_| used[map]) {
                Some(obj) => self.texture_mapping(&obj, mem),
                None => TextureMapping::default(),
            };
        }
        let mut draw = DrawParams::default();
        for i in 0..10 {
            draw.pos_mtx[i] = mtx_from(&self.xf_matrices[i * 12..]);
        }
        if !use_pn {
            draw.pos_mtx[0] = mtx_from(&self.xf_matrices[(self.current_mtx as usize / 3) * 12..]);
        }
        (material, params, draw)
    }

    fn texture_mapping(&mut self, obj: &TexObj, mem: &dyn GxMemory) -> TextureMapping {
        if let Some(m) = self.tex_recent.iter().flatten().find(|(o, _)| o == obj) {
            return m.1;
        }
        let epoch = self.tex_epoch;
        let m = self.resolve_texture(obj, mem);
        if self.tex_epoch == epoch {
            self.tex_recent.rotate_right(1);
            self.tex_recent[0] = Some((*obj, m));
        }
        m
    }

    fn resolve_texture(&mut self, obj: &TexObj, mem: &dyn GxMemory) -> TextureMapping {
        let levels = if obj.mipmap {
            (obj.max_lod.max(0.0) as u32 + 1).min(11)
        } else {
            1
        };
        let tlut = obj
            .tlut_name
            .and_then(|n| self.tluts.get(&n))
            .map(|t| (t.addr, t.format, t.entries));
        let key = TexKey {
            addr: obj.image_addr,
            format: obj.format,
            width: obj.width,
            height: obj.height,
            levels,
            tlut,
        };
        let copied = self.efb_copies.get(&obj.image_addr).copied();
        let cached = self.tex_cache.get(&key).copied();
        let id = match (copied, cached) {
            (Some(id), _) => id,
            (None, Some((id, generation, _))) if generation == self.tex_gen => id,
            (None, stale) => {
                let id = stale.map_or_else(
                    || {
                        self.next_texture += 1;
                        self.next_texture - 1
                    },
                    |s| s.0,
                );
                let mut size = 0usize;
                let (mut w, mut h) = (obj.width, obj.height);
                for _ in 0..levels {
                    size += texture::calc_texture_size(obj.format, w.max(1), h.max(1)).max(32);
                    w /= 2;
                    h /= 2;
                }
                let data = mem.read(obj.image_addr, size).unwrap_or(&[]);
                let palette = tlut
                    .and_then(|(a, _, n)| mem.read(a, n as usize * 2))
                    .unwrap_or(&[]);
                // The game invalidates texture memory every frame; the bytes
                // behind most textures stay the same.
                let hash = hash_bytes(data) ^ hash_bytes(palette).rotate_left(1);
                if stale.is_some_and(|s| s.2 == hash) {
                    self.tex_cache.insert(key, (id, self.tex_gen, hash));
                    return self.mapping(obj, id);
                }
                let input = TextureInput {
                    format: obj.format,
                    width: obj.width,
                    height: obj.height,
                    data,
                    mip_count: levels,
                    palette_format: tlut.map_or(0, |t| t.1),
                    palette,
                };
                self.frame.uploads.push(crate::api::TextureUpload {
                    id,
                    levels: texture::decode_mip_chain(&input),
                });
                self.tex_cache.insert(key, (id, self.tex_gen, hash));
                id
            }
        };
        self.mapping(obj, id)
    }

    fn mapping(&self, obj: &TexObj, id: TextureId) -> TextureMapping {
        TextureMapping {
            texture: Some(id),
            sampler: SamplerDesc {
                wrap_s: obj.wrap_s,
                wrap_t: obj.wrap_t,
                min_filter: obj.min_filter,
                mag_filter: obj.mag_filter,
                min_lod_x16: (obj.min_lod.clamp(0.0, 10.0) * 16.0) as u16,
                max_lod_x16: (obj.max_lod.clamp(0.0, 10.0) * 16.0) as u16,
                max_aniso: obj.max_aniso,
            },
            width: obj.width as f32,
            height: obj.height as f32,
            lod_bias: obj.lod_bias,
            flip_y: false,
        }
    }

    /// GX projection (row major, z to -1..0 with near at -1) as a
    /// column-major wgpu matrix (z 0..1, near at 0).
    fn wgpu_projection(&self) -> [[f32; 4]; 4] {
        let mut p = self.projection;
        for c in 0..4 {
            p[2][c] += p[3][c];
        }
        std::array::from_fn(|c| std::array::from_fn(|r| p[r][c]))
    }

    // ---- registers (display lists) ---------------------------------------

    fn xf_write(&mut self, addr: u32, v: u32) {
        let f = f32::from_bits(v);
        if let Some(m) = self.xf_matrices.get_mut(addr as usize) {
            if m.to_bits() != v {
                *m = f;
                self.joinable = false;
            }
            return;
        }
        if self.xf_seen.get(addr as usize) == Some(&Some(v)) {
            return;
        }
        let was = std::mem::take(&mut self.joinable);
        match addr {
            0x500..=0x5FF => {
                let k = (addr - 0x500) as usize;
                if let Some(m) = self.post_tex_mtx.get_mut(k / 12) {
                    m[(k % 12) / 4][k % 4] = f;
                }
            }
            0x600..=0x67F => {
                let k = (addr - 0x600) as usize;
                let l = &mut self.params.lights[k / 16];
                match k % 16 {
                    3 => l.color = rgba8(v.to_be_bytes()),
                    4..=6 => l.cos_atten[k % 16 - 4] = f,
                    7..=9 => l.dist_atten[k % 16 - 7] = f,
                    10..=12 => l.position[k % 16 - 10] = f,
                    13..=15 => l.direction[k % 16 - 13] = f,
                    _ => {}
                }
            }
            a if a == xf_register::XF_MATRIXINDEX0_ID => {
                self.current_mtx = v & 0x3F;
                for (t, m) in self.tex_mtx_idx[..4].iter_mut().enumerate() {
                    *m = (v >> (6 + 6 * t)) & 0x3F;
                }
            }
            a if a == xf_register::XF_MATRIXINDEX1_ID => {
                for (t, m) in self.tex_mtx_idx[4..].iter_mut().enumerate() {
                    *m = (v >> (6 * t)) & 0x3F;
                }
            }
            a if (xf_register::XF_VPSCALEX_ID..=xf_register::XF_VPOFFSETZ_ID).contains(&a) => {
                self.vp_regs[(a - xf_register::XF_VPSCALEX_ID) as usize] = f;
                // Undo GXSetViewport: scale (w/2, -h/2, (far-near)*zmax),
                // offset (x+w/2+342, y+h/2+342, far*zmax).
                let [sx, sy, sz, ox, oy, oz] = self.vp_regs;
                let zmax = 16_777_215.0;
                let far = oz / zmax;
                self.viewport = [
                    ox - 342.0 - sx,
                    oy - 342.0 + sy,
                    2.0 * sx,
                    -2.0 * sy,
                    far - sz / zmax,
                    far,
                ];
            }
            a if (xf_register::XF_PROJECTIONA_ID..=xf_register::XF_PROJECTORTHO_ID)
                .contains(&a) =>
            {
                self.proj_regs[(a - xf_register::XF_PROJECTIONA_ID) as usize] = f;
                let [p0, p1, p2, p3, p4, p5, ortho] = self.proj_regs;
                // The ortho flag is written as an integer word.
                self.projection = if ortho.to_bits() != 0 {
                    [
                        [p0, 0.0, 0.0, p1],
                        [0.0, p2, 0.0, p3],
                        [0.0, 0.0, p4, p5],
                        [0.0, 0.0, 0.0, 1.0],
                    ]
                } else {
                    [
                        [p0, 0.0, p1, 0.0],
                        [0.0, p2, p3, 0.0],
                        [0.0, 0.0, p4, p5],
                        [0.0, 0.0, -1.0, 0.0],
                    ]
                };
            }
            // BP GEN_MODE holds these counts too: its next write counts again.
            a if a == xf_register::XF_NUMCOLORS_ID => {
                self.num_chans = v as usize & 3;
                self.bp_fresh[bp_register::GEN_MODE_ID as usize] = false;
            }
            a if a == xf_register::XF_NUMTEX_ID => {
                self.num_tex_gens = v as usize & 0xF;
                self.bp_fresh[bp_register::GEN_MODE_ID as usize] = false;
            }
            a if a == xf_register::XF_AMBIENT0_ID || a == xf_register::XF_AMBIENT1_ID => {
                self.params.colors[color_kind::AMB0 + (a - xf_register::XF_AMBIENT0_ID) as usize] =
                    rgba8(v.to_be_bytes());
            }
            a if a == xf_register::XF_MATERIAL0_ID || a == xf_register::XF_MATERIAL1_ID => {
                self.params.colors
                    [color_kind::MAT0 + (a - xf_register::XF_MATERIAL0_ID) as usize] =
                    rgba8(v.to_be_bytes());
            }
            a if (xf_register::XF_COLOR0CNTRL_ID..=xf_register::XF_ALPHA1CNTRL_ID).contains(&a)
                || (xf_register::XF_TEX0_ID..xf_register::XF_TEX0_ID + 8).contains(&a)
                || (xf_register::XF_DUALTEX0_ID..xf_register::XF_DUALTEX0_ID + 8).contains(&a) =>
            {
                let mut r = crate::vtx::DisplayListRegisters::default();
                // Decode through the material builder's register readers.
                r.xf_set(a, 0, v);
                if a >= xf_register::XF_DUALTEX0_ID {
                    let i = a - xf_register::XF_DUALTEX0_ID;
                    self.mat_tex_dual(i, v);
                    // The texgen register decodes with this one.
                    self.xf_seen[(xf_register::XF_TEX0_ID + i) as usize] = None;
                } else if a >= xf_register::XF_TEX0_ID {
                    let i = a - xf_register::XF_TEX0_ID;
                    let dual = self.dual_tex[i as usize];
                    r.xf_set(xf_register::XF_DUALTEX0_ID + i, 0, dual);
                    self.mat.set_tex_gen_from_registers(&r, i);
                } else {
                    let i = (a - xf_register::XF_COLOR0CNTRL_ID) & 1;
                    let is_alpha = a >= xf_register::XF_ALPHA0CNTRL_ID;
                    self.chan_regs[(is_alpha as usize) * 2 + i as usize] = v;
                    r.xf_set(
                        xf_register::XF_COLOR0CNTRL_ID + i,
                        0,
                        self.chan_regs[i as usize],
                    );
                    r.xf_set(
                        xf_register::XF_ALPHA0CNTRL_ID + i,
                        0,
                        self.chan_regs[2 + i as usize],
                    );
                    self.mat.set_color_channel_from_registers(&r, i);
                }
            }
            _ => {
                // Not modelled: nothing a draw reads changed.
                *self.unhandled.entry((b'X', addr)).or_default() += 1;
                self.joinable = was;
                return;
            }
        }
        if let Some(s) = self.xf_seen.get_mut(addr as usize) {
            *s = Some(v);
        }
    }

    fn mat_tex_dual(&mut self, i: u32, v: u32) {
        self.dual_tex[i as usize] = v;
    }

    fn cp_write(&mut self, addr: u32, v: u32) {
        // The bits of the address the decoding below tells apart.
        let key = (match addr & 0xF0 {
            0x50 | 0x60 => addr & 0xF0,
            0x70 | 0x80 | 0x90 => addr & 0xF7,
            _ => addr,
        } & 0xFF) as usize;
        if self.cp_seen[key] == Some(v) {
            return;
        }
        let was = std::mem::take(&mut self.joinable);
        let two = |v: u32, at: u32| (v >> at) & 3;
        match addr & 0xF0 {
            0x50 => {
                self.vcd[attr::PNMTXIDX as usize].ty = v & 1;
                for t in 0..8 {
                    self.vcd[(attr::TEX0MTXIDX + t) as usize].ty = (v >> (1 + t)) & 1;
                }
                self.vcd[attr::POS as usize].ty = two(v, 9);
                self.vcd[attr::NRM as usize].ty = two(v, 11);
                self.vcd[attr::CLR0 as usize].ty = two(v, 13);
                self.vcd[attr::CLR1 as usize].ty = two(v, 15);
            }
            0x60 => {
                for t in 0..8 {
                    self.vcd[(attr::TEX0 + t) as usize].ty = two(v, 2 * t);
                }
            }
            0x70 | 0x80 | 0x90 => {
                let f = &mut self.vat[(addr & 7) as usize];
                let set = |f: &mut VatFormat, a: u32, cnt: u32, ty: u32, shift: u32| {
                    f[a as usize] = VtxAttrFmt {
                        comp_cnt: cnt,
                        comp_type: ty,
                        comp_shift: shift,
                    }
                };
                let b = |at: u32, n: u32| (v >> at) & ((1 << n) - 1);
                match addr & 0xF0 {
                    0x70 => {
                        set(f, attr::POS, b(0, 1), b(1, 3), b(4, 5));
                        set(f, attr::NRM, b(9, 1) | (b(31, 1) << 1), b(10, 3), 0);
                        set(f, attr::CLR0, b(13, 1), b(14, 3), 0);
                        set(f, attr::CLR1, b(17, 1), b(18, 3), 0);
                        set(f, attr::TEX0, b(21, 1), b(22, 3), b(25, 5));
                    }
                    0x80 => {
                        set(f, attr::TEX1, b(0, 1), b(1, 3), b(4, 5));
                        set(f, attr::TEX2, b(9, 1), b(10, 3), b(13, 5));
                        set(f, attr::TEX3, b(18, 1), b(19, 3), b(22, 5));
                        let t4 = f[attr::TEX4 as usize].comp_shift;
                        set(f, attr::TEX4, b(27, 1), b(28, 3), t4);
                    }
                    _ => {
                        f[attr::TEX4 as usize].comp_shift = b(0, 5);
                        set(f, attr::TEX5, b(5, 1), b(6, 3), b(9, 5));
                        set(f, attr::TEX6, b(14, 1), b(15, 3), b(18, 5));
                        set(f, attr::TEX7, b(23, 1), b(24, 3), b(27, 5));
                    }
                }
            }
            0xA0 => self.arrays[(addr & 0xF) as usize].0 = v,
            0xB0 => self.arrays[(addr & 0xF) as usize].1 = (v & 0xFF) as usize,
            _ => {
                *self.unhandled.entry((b'C', addr)).or_default() += 1;
                self.joinable = was;
                return;
            }
        }
        self.cp_seen[key] = Some(v);
    }

    fn bp_write(&mut self, bag: u32) {
        let addr = bag >> 24;
        let old = self.regs.bp[addr as usize];
        self.regs.bp_set(bag);
        // Palette loads, texture invalidation, EFB copies and the write mask
        // act on every write.
        if self.bp_fresh[addr as usize]
            && self.regs.bp[addr as usize] == old
            && !matches!(addr, 0x52 | 0x65 | 0x66 | 0xFE)
        {
            return;
        }
        let was = std::mem::take(&mut self.joinable);
        let r = &self.regs;
        let v = r.bp[addr as usize] & 0x00FF_FFFF;
        use bp_register as bp;
        match addr {
            bp::GEN_MODE_ID => {
                self.num_tex_gens = (v & 0xF) as usize;
                self.num_chans = ((v >> 4) & 3) as usize;
                self.num_tev_stages = ((v >> 10) & 0xF) as usize + 1;
                self.num_ind_stages = ((v >> 16) & 7) as usize;
                let hw = [
                    cull_mode::NONE,
                    cull_mode::BACK,
                    cull_mode::FRONT,
                    cull_mode::ALL,
                ];
                self.mat.set_cull_mode(hw[((v >> 14) & 3) as usize]);
                // XF holds the texgen and channel counts too.
                self.xf_seen[xf_register::XF_NUMCOLORS_ID as usize] = None;
                self.xf_seen[xf_register::XF_NUMTEX_ID as usize] = None;
            }
            0xC0..=0xDF => {
                let i = (addr - 0xC0) / 2;
                if addr & 1 == 1 {
                    self.swap_sel[i as usize] = (v & 3, (v >> 2) & 3);
                }
                // Decode the whole stage (order, ksel and indirect included).
                self.mat.set_tev_stage_from_registers(r, i);
            }
            0x28..=0x2F | 0x10..=0x1F => {
                let stages: Vec<u32> = if addr >= 0x28 {
                    vec![(addr - 0x28) * 2, (addr - 0x28) * 2 + 1]
                } else {
                    vec![addr - 0x10]
                };
                for i in stages {
                    self.mat.set_tev_stage_from_registers(r, i);
                }
            }
            0xF6..=0xFD => {
                let k = (addr - 0xF6) as usize;
                // Swap table halves: even regs hold R/G, odd hold B/A.
                let t = &mut self.swap_tables[k / 2];
                if k.is_multiple_of(2) {
                    t[0] = v & 3;
                    t[1] = (v >> 2) & 3;
                } else {
                    t[2] = v & 3;
                    t[3] = (v >> 2) & 3;
                }
                for i in [(k * 2) as u32, (k * 2 + 1) as u32] {
                    let odd = i & 1 != 0;
                    self.mat
                        .set_tev_k_color_sel(i, (if odd { v >> 14 } else { v >> 4 }) & 0x1F);
                    self.mat
                        .set_tev_k_alpha_sel(i, (if odd { v >> 19 } else { v >> 9 }) & 0x1F);
                }
                // Every TEV stage decodes its swap mode from these tables.
                self.bp_fresh[0xC0..=0xDF].fill(false);
                self.bp_fresh[0x28..=0x2F].fill(false);
                self.bp_fresh[0x10..=0x1F].fill(false);
            }
            bp::RAS1_IREF_ID | 0x25 | 0x26 => {
                for i in 0..4 {
                    self.mat.set_ind_tex_stage_from_registers(r, i);
                }
            }
            0x06..=0x0E => {
                // Indirect matrices: three registers per matrix, 11-bit
                // signed fixed point (s.10) plus two bits of scale each.
                let m = ((addr - 0x06) / 3) as usize;
                let base = 0x06 + 3 * m as u32;
                let regs = [0, 1, 2].map(|k| r.bp[(base + k) as usize] & 0x00FF_FFFF);
                let s11 = |x: u32| ((((x & 0x7FF) as i32) << 21) >> 21) as f32 / 1024.0;
                let scale_exp = ((regs[0] >> 22) & 3)
                    | (((regs[1] >> 22) & 3) << 2)
                    | (((regs[2] >> 22) & 1) << 4);
                let mtx = [
                    [s11(regs[0]), s11(regs[1]), s11(regs[2])],
                    [s11(regs[0] >> 11), s11(regs[1] >> 11), s11(regs[2] >> 11)],
                ];
                self.params.ind_tex_mtx[m] = params::ind_tex_mtx(mtx, scale_exp as i8 - 17);
            }
            0xE0..=0xE7 => {
                let k = ((addr - 0xE0) / 2) as usize;
                let lo = r.bp[0xE0 + 2 * k];
                let hi = r.bp[0xE1 + 2 * k];
                let s11 = |x: u32| ((((x & 0x7FF) as i32) << 21) >> 21) as f32 / 255.0;
                let konst = (lo >> 23) & 1 != 0;
                let rgba = [s11(lo), s11(hi >> 12), s11(hi), s11(lo >> 12)];
                if konst {
                    self.params.colors[color_kind::K0 + k] = rgba;
                } else {
                    // TEV registers are ordered C0, C1, C2, CPREV... in
                    // hardware: register 0 is PREV.
                    self.params.colors[color_kind::CPREV + k] = rgba;
                }
            }
            0x20 | 0x21 => {
                // SU_SCIS0/1: top/left and bottom/right, 11 bits each.
                // shortcut: assumes GX's default scissor offset of 342; read BP 0x59 if a game moves it.
                let (a, b) = (r.bp[0x20], r.bp[0x21]);
                let (top, left) = (a & 0x7FF, (a >> 12) & 0x7FF);
                let (bottom, right) = (b & 0x7FF, (b >> 12) & 0x7FF);
                self.scissor = [
                    left.saturating_sub(342),
                    top.saturating_sub(342),
                    (right + 1).saturating_sub(left),
                    (bottom + 1).saturating_sub(top),
                ];
            }
            0x4F | 0x50 => {
                // PE copy clear: 0x4F is A,R and 0x50 is G,B.
                let (ar, gb) = (r.bp[0x4F], r.bp[0x50]);
                self.frame.clear_color =
                    rgba8([ar as u8, (gb >> 8) as u8, gb as u8, (ar >> 8) as u8]);
            }
            0x65 => {
                // TX_LOADTLUT1: TMEM offset and count (16-entry units);
                // the source address came in TX_LOADTLUT0.
                let addr = (r.bp[0x64] & 0x1F_FFFF) << 5;
                self.tlut_loads
                    .insert(v & 0x3FF, (addr, ((v >> 10) & 0x7FF) * 16));
                // Texture maps pick their palette up when their registers
                // are written.
                self.bp_fresh[0x80..=0xBB].fill(false);
            }
            0x66 => self.invalidate_tex_all(),
            0x80..=0x9B | 0xA0..=0xBB => {
                let i = ((addr & 3) + if addr >= 0xA0 { 4 } else { 0 }) as usize;
                self.tex_map_from_regs(i);
            }
            bp::PE_ZMODE_ID | bp::PE_CMODE0_ID | bp::TEV_ALPHAFUNC_ID => {
                self.mat.set_rop_state_from_registers(r)
            }
            0x42 => {
                self.dst_alpha = if (v >> 8) & 1 != 0 {
                    Some((v & 0xFF) as u8)
                } else {
                    None
                }
            }
            // PE copy execute: bit 14 set copies to the XFB (the screen),
            // clear copies to a texture.
            0x52 if v & 0x4000 == 0 => {
                let (tl, wh) = (r.bp[0x49], r.bp[0x4A]);
                let src = [
                    tl & 0x3FF,
                    (tl >> 10) & 0x3FF,
                    (wh & 0x3FF) + 1,
                    ((wh >> 10) & 0x3FF) + 1,
                ];
                let addr = (r.bp[0x4B] & 0x1F_FFFF) << 5;
                let id = match self.efb_copies.get(&addr) {
                    Some(id) => *id,
                    None => {
                        let id = self.next_texture;
                        self.next_texture += 1;
                        self.efb_copies.insert(addr, id);
                        self.tex_changed();
                        id
                    }
                };
                let draw = self.frame.draws.len();
                self.frame.tex_copies.push(TexCopy {
                    draw,
                    src,
                    id,
                    clear: v & 0x800 != 0,
                });
            }
            _ => {
                *self.unhandled.entry((b'B', addr)).or_default() += 1;
                self.joinable = was;
                return;
            }
        }
        self.bp_fresh[addr as usize] = true;
    }

    /// Texture map `i` from its BP registers (SETMODE0/1, SETIMAGE0/3, SETTLUT).
    fn tex_map_from_regs(&mut self, i: usize) {
        let base = if i < 4 { i as u32 } else { 0x20 + i as u32 - 4 };
        let reg = |at: u32| self.regs.bp[(at + base) as usize] & 0x00FF_FFFF;
        let (m0, m1, im0, im3, tl) = (reg(0x80), reg(0x84), reg(0x88), reg(0x94), reg(0x98));
        let format = (im0 >> 20) & 0xF;
        let hw_min = ((m0 >> 5) & 7) as usize;
        use tex_filter as tf;
        let min_filter = [
            tf::NEAR,
            tf::NEAR_MIP_NEAR,
            tf::NEAR_MIP_LIN,
            tf::NEAR,
            tf::LINEAR,
            tf::LIN_MIP_NEAR,
            tf::LIN_MIP_LIN,
            tf::LINEAR,
        ][hw_min];
        let tlut_name =
            matches!(format, tex_format::C4 | tex_format::C8 | tex_format::C14X2).then(|| {
                let tmem = tl & 0x3FF;
                let name = 0x1_0000 | tmem;
                if let Some(&(addr, entries)) = self.tlut_loads.get(&tmem) {
                    self.load_tlut(
                        TlutObj {
                            addr,
                            format: (tl >> 10) & 3,
                            entries,
                        },
                        name,
                    );
                }
                name
            });
        self.tex_maps[i] = Some(TexObj {
            image_addr: (im3 & 0x1F_FFFF) << 5,
            width: (im0 & 0x3FF) + 1,
            height: ((im0 >> 10) & 0x3FF) + 1,
            format,
            wrap_s: m0 & 3,
            wrap_t: (m0 >> 2) & 3,
            mipmap: matches!(hw_min, 1 | 2 | 5 | 6),
            min_filter,
            mag_filter: (m0 >> 4) & 1,
            min_lod: (m1 & 0xFF) as f32 / 16.0,
            max_lod: ((m1 >> 8) & 0xFF) as f32 / 16.0,
            // s2.5 fixed point.
            lod_bias: f32::from((m0 >> 9) as u8 as i8) / 32.0,
            max_aniso: (m0 >> 19) & 3,
            tlut_name,
        });
    }

    // ---- transform -------------------------------------------------------

    /// `GXLoadPosMtxImm`: `id` is a GX_PNMTXn value (0, 3, ... 27).
    pub fn load_pos_mtx_imm(&mut self, m: &Mtx34, id: u32) {
        let at = id as usize * 4;
        for (k, v) in m.iter().flatten().enumerate() {
            if let Some(s) = self.xf_matrices.get_mut(at + k) {
                *s = *v;
            }
        }
    }

    /// `GXLoadNrmMtxImm`. Normals are transformed by the position matrix
    /// with its scale removed (as noclip does), so this is not stored.
    pub fn load_nrm_mtx_imm(&mut self, _m: &Mtx34, _id: u32) {}

    /// `GXLoadTexMtxImm`: ids below 64 go to matrix memory (GX_TEXMTXn or
    /// PNMTX), 64 and up to the post-transform matrices. A 2x4 matrix
    /// leaves the third row as (0, 0, 1, 0)... GX keeps its old value; here
    /// it is reset for determinism.
    pub fn load_tex_mtx_imm(&mut self, m: &Mtx34, id: u32, ty: u32) {
        let mut m = *m;
        if ty == tex_gen_type::MTX2x4 {
            m[2] = [0.0, 0.0, 1.0, 0.0];
        }
        if id >= post_tex_gen_matrix::PTTEXMTX0 {
            if let Some(s) = self
                .post_tex_mtx
                .get_mut(((id - post_tex_gen_matrix::PTTEXMTX0) / 3) as usize)
            {
                *s = m;
            }
        } else {
            self.load_pos_mtx_imm(&m, id);
        }
    }

    /// `GXSetCurrentMtx`.
    pub fn set_current_mtx(&mut self, id: u32) {
        self.current_mtx = id;
    }

    /// `GXSetProjection`: a row-major 4x4 (GX_PERSPECTIVE or
    /// GX_ORTHOGRAPHIC; both are taken as full matrices).
    pub fn set_projection(&mut self, m: [[f32; 4]; 4], _ty: u32) {
        self.projection = m;
    }

    /// `GXSetViewport` (EFB pixels).
    pub fn set_viewport(&mut self, x: f32, y: f32, w: f32, h: f32, near: f32, far: f32) {
        self.viewport = [x, y, w, h, near, far];
    }

    /// `GXSetScissor` (EFB pixels).
    pub fn set_scissor(&mut self, x: u32, y: u32, w: u32, h: u32) {
        self.scissor = [x, y, w, h];
    }

    // ---- lighting --------------------------------------------------------

    pub fn set_num_chans(&mut self, n: u8) {
        self.num_chans = usize::from(n);
    }

    /// `GXSetChanCtrl`.
    #[allow(clippy::too_many_arguments)]
    pub fn set_chan_ctrl(
        &mut self,
        chan: u32,
        enable: bool,
        amb_src: u32,
        mat_src: u32,
        light_mask: u32,
        diff_fn: u32,
        attn_fn: u32,
    ) {
        self.mat
            .set_chan_ctrl(chan, enable, amb_src, mat_src, light_mask, diff_fn, attn_fn);
    }

    fn set_chan_color(&mut self, base: usize, chan: u32, c: [u8; 4]) {
        let c = rgba8(c);
        let i = (chan & 1) as usize;
        let slot = &mut self.params.colors[base + i];
        match chan {
            color_channel_id::COLOR0 | color_channel_id::COLOR1 => {
                slot[..3].copy_from_slice(&c[..3])
            }
            color_channel_id::ALPHA0 | color_channel_id::ALPHA1 => slot[3] = c[3],
            _ => *slot = c,
        }
    }

    /// `GXSetChanAmbColor`.
    pub fn set_chan_amb_color(&mut self, chan: u32, c: [u8; 4]) {
        self.set_chan_color(color_kind::AMB0, chan, c);
    }

    /// `GXSetChanMatColor`.
    pub fn set_chan_mat_color(&mut self, chan: u32, c: [u8; 4]) {
        self.set_chan_color(color_kind::MAT0, chan, c);
    }

    /// `GXLoadLightObjImm`: `light_id` is a GX_LIGHTn bit.
    pub fn load_light_obj_imm(&mut self, light: &Light, light_id: u32) {
        let i = light_id.trailing_zeros() as usize;
        if let Some(l) = self.params.lights.get_mut(i) {
            *l = *light;
        }
    }

    // ---- texgen --------------------------------------------------------

    pub fn set_num_tex_gens(&mut self, n: u8) {
        self.num_tex_gens = usize::from(n);
    }

    /// `GXSetTexCoordGen2`.
    pub fn set_tex_coord_gen2(
        &mut self,
        dst: u32,
        ty: u32,
        src: u32,
        mtx: u32,
        normalize: bool,
        post_mtx: u32,
    ) {
        self.mat
            .set_tex_coord_gen(dst, ty, src, mtx, normalize, post_mtx);
        if let Some(m) = self.tex_mtx_idx.get_mut(dst as usize) {
            *m = mtx;
        }
    }

    /// `GXSetTexCoordGen`.
    pub fn set_tex_coord_gen(&mut self, dst: u32, ty: u32, src: u32, mtx: u32) {
        self.set_tex_coord_gen2(dst, ty, src, mtx, false, post_tex_gen_matrix::PTIDENTITY);
    }

    // ---- textures --------------------------------------------------------

    /// `GXLoadTexObj`.
    pub fn load_tex_obj(&mut self, obj: &TexObj, map: u32) {
        if let Some(s) = self.tex_maps.get_mut(map as usize) {
            *s = Some(*obj);
        }
    }

    /// `GXInitTlutObj` + `GXLoadTlut`.
    pub fn load_tlut(&mut self, obj: TlutObj, name: u32) {
        if self.tluts.insert(name, obj) != Some(obj) {
            self.tex_changed();
        }
    }

    /// `GXInvalidateTexAll`: decode textures again on next use.
    pub fn invalidate_tex_all(&mut self) {
        self.tex_gen += 1;
        self.tex_changed();
    }

    fn tex_changed(&mut self) {
        self.tex_epoch += 1;
        self.tex_recent = [None; RECENT_TEXTURES];
    }

    // ---- TEV -------------------------------------------------------------

    pub fn set_num_tev_stages(&mut self, n: u8) {
        self.num_tev_stages = usize::from(n.max(1));
    }
    pub fn set_tev_order(&mut self, stage: u32, coord: u32, map: u32, color: u32) {
        let map = if map == 0xFF || map & 0x100 != 0 {
            if map == 0xFF {
                tex_map_id::TEXMAP_NULL
            } else {
                map & 7
            }
        } else {
            map
        };
        self.mat.set_tev_order(
            stage,
            coord,
            map,
            crate::material::ras_color_channel_id(color),
        );
    }
    pub fn set_tev_color_in(&mut self, stage: u32, a: u32, b: u32, c: u32, d: u32) {
        self.mat.set_tev_color_in(stage, a, b, c, d);
    }
    pub fn set_tev_alpha_in(&mut self, stage: u32, a: u32, b: u32, c: u32, d: u32) {
        self.mat.set_tev_alpha_in(stage, a, b, c, d);
    }
    pub fn set_tev_color_op(
        &mut self,
        stage: u32,
        op: u32,
        bias: u32,
        scale: u32,
        clamp: bool,
        reg: u32,
    ) {
        self.mat
            .set_tev_color_op(stage, op, bias, scale, clamp, reg);
    }
    pub fn set_tev_alpha_op(
        &mut self,
        stage: u32,
        op: u32,
        bias: u32,
        scale: u32,
        clamp: bool,
        reg: u32,
    ) {
        self.mat
            .set_tev_alpha_op(stage, op, bias, scale, clamp, reg);
    }

    /// `GXSetTevOp`: the SDK's preset combiners. Stage 0 takes the
    /// rasterized color where later stages take the previous stage.
    pub fn set_tev_op(&mut self, stage: u32, mode: u32) {
        let (cin, ain) = if stage == 0 {
            (cc::RASC, ca::RASA)
        } else {
            (cc::CPREV, ca::APREV)
        };
        let (c, a) = match mode {
            tev_mode::MODULATE => (
                [cc::ZERO, cc::TEXC, cin, cc::ZERO],
                [ca::ZERO, ca::TEXA, ain, ca::ZERO],
            ),
            tev_mode::DECAL => (
                [cin, cc::TEXC, cc::TEXA, cc::ZERO],
                [ca::ZERO, ca::ZERO, ca::ZERO, ain],
            ),
            tev_mode::BLEND => (
                [cin, cc::ONE, cc::TEXC, cc::ZERO],
                [ca::ZERO, ca::TEXA, ain, ca::ZERO],
            ),
            tev_mode::REPLACE => (
                [cc::ZERO, cc::ZERO, cc::ZERO, cc::TEXC],
                [ca::ZERO, ca::ZERO, ca::ZERO, ca::TEXA],
            ),
            _ => (
                [cc::ZERO, cc::ZERO, cc::ZERO, cin],
                [ca::ZERO, ca::ZERO, ca::ZERO, ain],
            ),
        };
        self.set_tev_color_in(stage, c[0], c[1], c[2], c[3]);
        self.set_tev_alpha_in(stage, a[0], a[1], a[2], a[3]);
        self.set_tev_color_op(
            stage,
            tev_op::ADD,
            tev_bias::ZERO,
            tev_scale::SCALE_1,
            true,
            register::PREV,
        );
        self.set_tev_alpha_op(
            stage,
            tev_op::ADD,
            tev_bias::ZERO,
            tev_scale::SCALE_1,
            true,
            register::PREV,
        );
    }

    /// `GXSetTevColor`: GX_TEVPREV, GX_TEVREG0..2.
    pub fn set_tev_color(&mut self, reg: u32, c: [u8; 4]) {
        self.params.colors[color_kind::CPREV + (reg as usize & 3)] = rgba8(c);
    }

    /// `GXSetTevColorS10`: signed 10-bit components (-1024..1023).
    pub fn set_tev_color_s10(&mut self, reg: u32, c: [i16; 4]) {
        self.params.colors[color_kind::CPREV + (reg as usize & 3)] =
            c.map(|v| f32::from(v) / 255.0);
    }

    /// `GXSetTevKColor`: GX_KCOLOR0..3.
    pub fn set_tev_k_color(&mut self, id: u32, c: [u8; 4]) {
        self.params.colors[color_kind::K0 + (id as usize & 3)] = rgba8(c);
    }
    pub fn set_tev_k_color_sel(&mut self, stage: u32, sel: u32) {
        self.mat.set_tev_k_color_sel(stage, sel);
    }
    pub fn set_tev_k_alpha_sel(&mut self, stage: u32, sel: u32) {
        self.mat.set_tev_k_alpha_sel(stage, sel);
    }

    /// `GXSetTevSwapMode`: which swap table the rasterized color and the
    /// texture use.
    pub fn set_tev_swap_mode(&mut self, stage: u32, ras_sel: u32, tex_sel: u32) {
        if let Some(s) = self.swap_sel.get_mut(stage as usize) {
            *s = (ras_sel & 3, tex_sel & 3);
        }
    }

    /// `GXSetTevSwapModeTable`.
    pub fn set_tev_swap_mode_table(&mut self, id: u32, r: u32, g: u32, b: u32, a: u32) {
        self.swap_tables[id as usize & 3] = [r, g, b, a];
    }

    pub fn set_tev_direct(&mut self, stage: u32) {
        self.mat.set_tev_direct(stage);
    }

    /// `GXSetTevIndirect`.
    #[allow(clippy::too_many_arguments)]
    pub fn set_tev_indirect(
        &mut self,
        stage: u32,
        ind_stage: u32,
        format: u32,
        bias_sel: u32,
        matrix: u32,
        wrap_s: u32,
        wrap_t: u32,
        add_prev: bool,
        utc_lod: bool,
        alpha_sel: u32,
    ) {
        self.mat.set_tev_indirect(
            stage, ind_stage, format, bias_sel, matrix, wrap_s, wrap_t, add_prev, utc_lod,
            alpha_sel,
        );
    }

    /// `GXSetTevIndWarp`.
    pub fn set_tev_ind_warp(
        &mut self,
        stage: u32,
        ind_stage: u32,
        signed_offsets: bool,
        replace: bool,
        matrix: u32,
    ) {
        self.mat
            .set_tev_ind_warp(stage, ind_stage, signed_offsets, replace, matrix);
    }

    pub fn set_num_ind_stages(&mut self, n: u8) {
        self.num_ind_stages = usize::from(n);
    }
    pub fn set_ind_tex_order(&mut self, ind_stage: u32, coord: u32, map: u32) {
        self.mat.set_ind_tex_order(ind_stage, coord, map);
    }
    pub fn set_ind_tex_coord_scale(&mut self, ind_stage: u32, s: u32, t: u32) {
        self.mat.set_ind_tex_scale(ind_stage, s, t);
    }

    /// `GXSetIndTexMtx`: `id` is GX_ITM_0..2 (1..3).
    pub fn set_ind_tex_mtx(&mut self, id: u32, m: [[f32; 3]; 2], scale_exp: i8) {
        if let Some(s) = self
            .params
            .ind_tex_mtx
            .get_mut((id as usize).wrapping_sub(1))
        {
            *s = params::ind_tex_mtx(m, scale_exp);
        }
    }

    // ---- pixel engine ------------------------------------------------------

    pub fn set_blend_mode(&mut self, mode: u32, src: u32, dst: u32, logic: u32) {
        self.mat.set_blend_mode(mode, src, dst, logic);
    }
    pub fn set_z_mode(&mut self, test: bool, func: u32, update: bool) {
        self.mat.set_z_mode(test, func, update);
    }
    pub fn set_alpha_compare(&mut self, comp0: u32, ref0: u8, op: u32, comp1: u32, ref1: u8) {
        self.mat.set_alpha_compare(comp0, ref0, op, comp1, ref1);
    }
    pub fn set_cull_mode(&mut self, mode: u32) {
        self.mat.set_cull_mode(mode);
    }
    pub fn set_color_update(&mut self, v: bool) {
        self.mat.set_color_update(v);
    }
    pub fn set_alpha_update(&mut self, v: bool) {
        self.mat.set_alpha_update(v);
    }
    pub fn set_dst_alpha(&mut self, enable: bool, alpha: u8) {
        self.dst_alpha = enable.then_some(alpha);
    }
    pub fn set_z_comp_loc(&mut self, _before_tex: bool) {}

    /// `GXSetFog`.
    pub fn set_fog(
        &mut self,
        ty: u32,
        start_z: f32,
        end_z: f32,
        near_z: f32,
        far_z: f32,
        color: [u8; 4],
    ) {
        self.mat.set_fog(ty, false);
        self.params.fog.set(ty, start_z, end_z, near_z, far_z);
        self.params.fog.color = rgba8(color);
    }

    /// `GXSetCopyClear`.
    pub fn set_copy_clear(&mut self, color: [u8; 4], _z: u32) {
        self.frame.clear_color = rgba8(color);
    }

    /// The material the current state draws with (for tests and tools).
    pub fn current_material(&mut self) -> GxMaterial {
        (*self.snapshot(&Ram(&[])).0).clone()
    }
}

type LoaderKey = (Vcd, [VatFormat; 8]);

/// The state besides the material builder that a draw's material is built from.
#[derive(PartialEq)]
struct MatKey {
    counts: [usize; 4],
    swap_sel: [(u32, u32); 16],
    swap_tables: [crate::material::SwapTable; 4],
    dst_alpha: Option<u8>,
    tex_mtx_idx: [u32; 8],
    pn: u32,
    tex_mtx: [u32; 8],
}

/// Hash of texture bytes, eight at a time.
fn hash_bytes(b: &[u8]) -> u64 {
    let mut h = FastHasher::default();
    let (words, rest) = b.as_chunks::<8>();
    for w in words {
        h.write_u64(u64::from_le_bytes(*w));
    }
    h.write(rest);
    h.write_usize(b.len());
    h.finish()
}
