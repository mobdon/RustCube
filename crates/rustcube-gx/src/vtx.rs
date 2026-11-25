// Derived from noclip.website (https://github.com/magcius/noclip.website),
// src/gx/gx_displaylist.ts at commit 2cebac2a5d1d7c44535397a14f51e764ca823767.
// Copyright (c) 2018 Jasper St. Pierre. MIT License: see THIRD-PARTY.md.
// Translated from TypeScript to Rust for rustcube-gx.
//! GX display lists and the vertex loader.
//!
//! A GX vertex is described by the VCD (per attribute: absent, direct in
//! the command stream, or an 8/16-bit index into an array) and the VAT
//! (per vertex format and attribute: component type, count and fixed-point
//! shift). The loader turns a display list's draw commands into one flat,
//! little-endian vertex buffer plus a triangle index list, in the layout
//! the generated shaders read ([`LoadedVertexLayout`]).
//!
//! noclip compiles a JavaScript function per VAT/VCD pair; here the same
//! steps run as a small interpreter over a precomputed attribute list.
//! Changes from noclip: indices are u32, NBT3 writes N/B/T to their own
//! inputs (noclip wrote all three to the normal), an `Index` output is
//! stored as f32, and line/point primitives are skipped instead of failing.
use crate::enums::{attr, attr_type, bp_register, command, comp_cnt, comp_type};

/// Attributes 0..=20 (PNMTXIDX..TEX7).
pub const ATTR_COUNT: usize = attr::MAX as usize + 1;

/// `GX_SetVtxAttrFmt`: one attribute of one vertex format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct VtxAttrFmt {
    pub comp_type: u32,
    pub comp_cnt: u32,
    pub comp_shift: u32,
}

/// What the loader writes for an attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum OutputMode {
    #[default]
    VertexData,
    /// The raw index (for callers that fetch the data themselves).
    Index,
    None,
}

/// `GX_SetVtxDesc`: how an attribute is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct VtxDesc {
    /// [`attr_type`]: NONE, DIRECT, INDEX8, INDEX16 or (noclip's) ZERO.
    pub ty: u32,
    pub output: OutputMode,
}

pub type Vcd = [VtxDesc; ATTR_COUNT];
pub type VatFormat = [VtxAttrFmt; ATTR_COUNT];

/// `GX_SetArray`: an attribute array (data starts at the array base).
#[derive(Debug, Clone, Copy, Default)]
pub struct GxArray<'a> {
    pub data: &'a [u8],
    pub stride: usize,
    /// The bytes indexed so far (an end offset), when the caller tracks it.
    pub used: Option<&'a std::cell::Cell<usize>>,
}

/// Shader inputs. Texture matrix indices are packed four to a vec4, and
/// texture coordinates two to a vec4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum VertexInput {
    Tex0123MtxIdx,
    Tex4567MtxIdx,
    /// xyz position, w = PNMTXIDX / 3.
    Pos,
    Nrm,
    Binrm,
    Tangent,
    Clr0,
    Clr1,
    Tex01,
    Tex23,
    Tex45,
    Tex67,
}

impl VertexInput {
    pub const ALL: [VertexInput; 12] = [
        Self::Tex0123MtxIdx,
        Self::Tex4567MtxIdx,
        Self::Pos,
        Self::Nrm,
        Self::Binrm,
        Self::Tangent,
        Self::Clr0,
        Self::Clr1,
        Self::Tex01,
        Self::Tex23,
        Self::Tex45,
        Self::Tex67,
    ];

    /// Shader location (noclip's `vtxAttributeGenDefs` order).
    pub fn location(self) -> u32 {
        match self {
            Self::Pos => 0,
            Self::Tex0123MtxIdx => 1,
            Self::Tex4567MtxIdx => 2,
            Self::Nrm => 3,
            Self::Binrm => 4,
            Self::Tangent => 5,
            Self::Clr0 => 6,
            Self::Clr1 => 7,
            Self::Tex01 => 8,
            Self::Tex23 => 9,
            Self::Tex45 => 10,
            Self::Tex67 => 11,
        }
    }

    /// Name used in the generated WGSL (`a_<name>`).
    pub fn name(self) -> &'static str {
        match self {
            Self::Pos => "Position",
            Self::Tex0123MtxIdx => "TexMtx0123Idx",
            Self::Tex4567MtxIdx => "TexMtx4567Idx",
            Self::Nrm => "Normal",
            Self::Binrm => "Binormal",
            Self::Tangent => "Tangent",
            Self::Clr0 => "Color0",
            Self::Clr1 => "Color1",
            Self::Tex01 => "Tex01",
            Self::Tex23 => "Tex23",
            Self::Tex45 => "Tex45",
            Self::Tex67 => "Tex67",
        }
    }

    /// Component count the shader declares.
    pub fn components(self) -> u32 {
        match self {
            Self::Nrm | Self::Binrm | Self::Tangent => 3,
            _ => 4,
        }
    }
}

/// Storage of one input in the vertex buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputFormat {
    /// `n` little-endian f32.
    F32(u32),
    /// Four u8 read as 0..1.
    Unorm8x4,
}

impl InputFormat {
    pub fn size(self) -> usize {
        match self {
            Self::F32(n) => 4 * n as usize,
            Self::Unorm8x4 => 4,
        }
    }
    fn comp_size(self) -> usize {
        match self {
            Self::F32(_) => 4,
            Self::Unorm8x4 => 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SingleVertexInputLayout {
    pub input: VertexInput,
    pub offset: usize,
    pub format: InputFormat,
}

/// How one attribute's value is stored at its destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
enum FieldFormat {
    #[default]
    None,
    /// `n` f32 components.
    F32(u32),
    /// u8 components holding raw values (matrix indices).
    U8,
    /// u8 components holding 0..1 values.
    U8Norm,
}

/// The loaded (destination) vertex layout.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LoadedVertexLayout {
    pub stride: usize,
    pub inputs: Vec<SingleVertexInputLayout>,
    attr_offsets: [usize; ATTR_COUNT],
    attr_formats: [FieldFormat; ATTR_COUNT],
}

impl LoadedVertexLayout {
    pub fn input(&self, i: VertexInput) -> Option<&SingleVertexInputLayout> {
        self.inputs.iter().find(|l| l.input == i)
    }
}

fn is_mtx_idx(a: u32) -> bool {
    a <= attr::TEX7MTXIDX
}
fn is_tex_mtx_idx(a: u32) -> bool {
    (attr::TEX0MTXIDX..=attr::TEX7MTXIDX).contains(&a)
}
fn is_color(a: u32) -> bool {
    a == attr::CLR0 || a == attr::CLR1
}
fn is_tex(a: u32) -> bool {
    (attr::TEX0..=attr::TEX7).contains(&a)
}

fn comp_byte_size_raw(ct: u32) -> usize {
    match ct {
        comp_type::U8 | comp_type::S8 => 1,
        comp_type::U16 | comp_type::S16 => 2,
        comp_type::F32 => 4,
        // RGBA8 shares the value 5 with nothing else in the non-color types.
        _ => 1,
    }
}

fn comp_byte_size(a: u32, f: &VtxAttrFmt) -> usize {
    if is_mtx_idx(a) {
        1
    } else {
        comp_byte_size_raw(f.comp_type)
    }
}

fn comp_count(a: u32, f: &VtxAttrFmt) -> usize {
    if is_mtx_idx(a) {
        return 1;
    }
    match a {
        attr::POS => {
            if f.comp_cnt == comp_cnt::POS_XY {
                2
            } else {
                3
            }
        }
        attr::NRM => {
            if f.comp_cnt == comp_cnt::NRM_NBT {
                9
            } else {
                3
            }
        }
        attr::CLR0 | attr::CLR1 => {
            if f.comp_cnt == comp_cnt::CLR_RGB {
                3
            } else {
                4
            }
        }
        _ => {
            if f.comp_cnt == comp_cnt::TEX_S {
                1
            } else {
                2
            }
        }
    }
}

/// Bytes an attribute takes when sent directly.
pub fn attribute_byte_size(a: u32, f: &VtxAttrFmt) -> usize {
    if is_mtx_idx(a) {
        return 1;
    }
    if is_color(a) {
        return match f.comp_type {
            comp_type::RGB565 | comp_type::RGBA4 => 2,
            comp_type::RGB8 | comp_type::RGBA6 => 3,
            _ => 4,
        };
    }
    comp_byte_size(a, f) * comp_count(a, f)
}

fn comp_shift(a: u32, f: &VtxAttrFmt) -> u32 {
    if is_mtx_idx(a) {
        return 0;
    }
    // Normals always use 6 (8-bit) or 14 (16-bit); the VAT value is ignored.
    if a == attr::NRM {
        return match f.comp_type {
            comp_type::U8 | comp_type::S8 => 6,
            comp_type::U16 | comp_type::S16 => 14,
            _ => 0,
        };
    }
    match f.comp_type {
        comp_type::U8 | comp_type::S8 | comp_type::U16 | comp_type::S16 => f.comp_shift,
        _ => 0,
    }
}

fn index_components(a: u32, f: &VtxAttrFmt) -> usize {
    if a == attr::NRM && f.comp_cnt == comp_cnt::NRM_NBT3 {
        3
    } else {
        1
    }
}

/// Bytes one vertex of this format takes in the command stream.
pub fn source_vertex_size(vat: &VatFormat, vcd: &Vcd) -> usize {
    let mut size = 0;
    for a in 0..ATTR_COUNT as u32 {
        let d = &vcd[a as usize];
        let f = &vat[a as usize];
        size += match d.ty {
            attr_type::DIRECT => attribute_byte_size(a, f),
            attr_type::INDEX8 => index_components(a, f),
            attr_type::INDEX16 => 2 * index_components(a, f),
            _ => 0,
        };
    }
    size
}

/// The destination layout for a VCD. `use_nbt` adds binormal and tangent.
pub fn compile_loaded_vertex_layout(vcd: &Vcd, use_nbt: bool) -> LoadedVertexLayout {
    fn input_format(i: VertexInput) -> InputFormat {
        match i {
            VertexInput::Tex0123MtxIdx | VertexInput::Tex4567MtxIdx => InputFormat::Unorm8x4,
            VertexInput::Clr0 | VertexInput::Clr1 => InputFormat::Unorm8x4,
            VertexInput::Nrm | VertexInput::Binrm | VertexInput::Tangent => InputFormat::F32(3),
            _ => InputFormat::F32(4),
        }
    }
    let mut size = 0usize;
    let mut inputs: Vec<SingleVertexInputLayout> = Vec::new();
    let mut alloc =
        |inputs: &mut Vec<SingleVertexInputLayout>, i: VertexInput, fmt: InputFormat| {
            if let Some(l) = inputs.iter().find(|l| l.input == i) {
                return *l;
            }
            let cs = fmt.comp_size();
            size = size.div_ceil(cs) * cs;
            let l = SingleVertexInputLayout {
                input: i,
                offset: size,
                format: fmt,
            };
            size += fmt.size();
            inputs.push(l);
            l
        };
    let mut attr_offsets = [0usize; ATTR_COUNT];
    let mut attr_formats = [FieldFormat::None; ATTR_COUNT];
    for a in 0..ATTR_COUNT as u32 {
        let d = &vcd[a as usize];
        if d.ty == attr_type::NONE || d.output == OutputMode::None {
            continue;
        }
        let (l, field, comp) = if d.output == OutputMode::Index {
            let i = match a {
                attr::POS => VertexInput::Pos,
                attr::NRM => VertexInput::Nrm,
                attr::CLR0 => VertexInput::Clr0,
                attr::CLR1 => VertexInput::Clr1,
                _ => continue,
            };
            (
                alloc(&mut inputs, i, InputFormat::F32(i.components())),
                FieldFormat::F32(1),
                0,
            )
        } else if is_tex_mtx_idx(a) {
            let i = if a < attr::TEX4MTXIDX {
                VertexInput::Tex0123MtxIdx
            } else {
                VertexInput::Tex4567MtxIdx
            };
            let l = alloc(&mut inputs, i, input_format(i));
            (l, FieldFormat::U8, ((a - attr::TEX0MTXIDX) & 3) as usize)
        } else if a == attr::POS {
            let l = alloc(
                &mut inputs,
                VertexInput::Pos,
                input_format(VertexInput::Pos),
            );
            (l, FieldFormat::F32(3), 0)
        } else if a == attr::PNMTXIDX {
            let l = alloc(
                &mut inputs,
                VertexInput::Pos,
                input_format(VertexInput::Pos),
            );
            (l, FieldFormat::F32(1), 3)
        } else if a == attr::NRM {
            let l = alloc(
                &mut inputs,
                VertexInput::Nrm,
                input_format(VertexInput::Nrm),
            );
            if use_nbt {
                alloc(
                    &mut inputs,
                    VertexInput::Binrm,
                    input_format(VertexInput::Binrm),
                );
                alloc(
                    &mut inputs,
                    VertexInput::Tangent,
                    input_format(VertexInput::Tangent),
                );
            }
            (l, FieldFormat::F32(3), 0)
        } else if is_tex(a) {
            let t = a - attr::TEX0;
            let i = [
                VertexInput::Tex01,
                VertexInput::Tex23,
                VertexInput::Tex45,
                VertexInput::Tex67,
            ][(t >> 1) as usize];
            let l = alloc(&mut inputs, i, input_format(i));
            (l, FieldFormat::F32(2), ((t & 1) * 2) as usize)
        } else if is_color(a) {
            let i = if a == attr::CLR0 {
                VertexInput::Clr0
            } else {
                VertexInput::Clr1
            };
            (
                alloc(&mut inputs, i, input_format(i)),
                FieldFormat::U8Norm,
                0,
            )
        } else {
            continue;
        };
        attr_offsets[a as usize] = l.offset + l.format.comp_size() * comp;
        attr_formats[a as usize] = field;
    }
    LoadedVertexLayout {
        stride: size.div_ceil(4) * 4,
        inputs,
        attr_offsets,
        attr_formats,
    }
}

/// Matrix tables set by indexed loads (LOAD_INDX_A / _C) before a draw;
/// 0xFFFF where unset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedVertexDraw {
    pub index_offset: u32,
    pub index_count: u32,
    pub pos_matrix_table: [u16; 10],
    pub tex_matrix_table: [u16; 10],
}

impl LoadedVertexDraw {
    fn new(index_offset: u32) -> Self {
        Self {
            index_offset,
            index_count: 0,
            pos_matrix_table: [0xFFFF; 10],
            tex_matrix_table: [0xFFFF; 10],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrawCall {
    pub prim: u32,
    pub vtx_fmt: usize,
    pub src_offs: usize,
    pub vertex_count: usize,
}

/// A loaded display list.
#[derive(Debug, Clone, Default)]
pub struct LoadedVertexData {
    pub vertices: Vec<u8>,
    pub indices: Vec<u32>,
    pub vertex_count: u32,
    pub draws: Vec<LoadedVertexDraw>,
    /// Draw commands found (for reloading with other arrays).
    pub draw_calls: Vec<DrawCall>,
    /// Offset after the last command read.
    pub end_offs: usize,
}

/// Triangle indices for a GX primitive of `n` vertices starting at `base`.
/// Lines and points give none.
pub fn triangulate(prim: u32, base: u32, n: u32, out: &mut Vec<u32>) {
    match prim {
        command::DRAW_TRIANGLES => out.extend((0..n - n % 3).map(|i| base + i)),
        command::DRAW_QUADS | command::DRAW_QUAD_STRIP => {
            for i in (0..n.saturating_sub(3)).step_by(4) {
                let b = base + i;
                out.extend([b, b + 1, b + 2, b, b + 2, b + 3]);
            }
        }
        command::DRAW_TRIANGLE_STRIP => {
            for i in 2..n {
                out.extend([base + i - 2, base + i - (!i & 1), base + i - (i & 1)]);
            }
        }
        command::DRAW_TRIANGLE_FAN => {
            for i in 2..n {
                out.extend([base, base + i - 1, base + i]);
            }
        }
        _ => {}
    }
}

/// A vertex loader for one VCD and up to eight vertex formats.
#[derive(Debug, Clone)]
pub struct VtxLoader {
    pub vat: [Option<VatFormat>; 8],
    pub vcd: Vcd,
    pub layout: LoadedVertexLayout,
    src_sizes: [usize; 8],
}

impl VtxLoader {
    /// Loader for vertex format 0 only.
    pub fn new(vat: VatFormat, vcd: Vcd) -> Self {
        let mut v = [None; 8];
        v[0] = Some(vat);
        Self::new_multi(v, vcd)
    }

    pub fn new_multi(vat: [Option<VatFormat>; 8], vcd: Vcd) -> Self {
        let use_nbt = vat.iter().flatten().any(|f| {
            let c = f[attr::NRM as usize].comp_cnt;
            c == comp_cnt::NRM_NBT || c == comp_cnt::NRM_NBT3
        });
        let layout = compile_loaded_vertex_layout(&vcd, use_nbt);
        let src_sizes = vat.map(|f| f.map_or(0, |f| source_vertex_size(&f, &vcd)));
        Self {
            vat,
            vcd,
            layout,
            src_sizes,
        }
    }

    /// Parse the draw commands of `dl` (stops at the first 0x00 byte or at
    /// a command it does not know) and build the index list. Vertex data is
    /// allocated but not filled: see [`Self::load_vertex_data`].
    pub fn parse_display_list(&self, dl: &[u8], first_vertex: u32) -> LoadedVertexData {
        let mut out = LoadedVertexData::default();
        let mut at = 0usize;
        let mut total_vertices = 0u32;
        let mut cur: Option<usize> = None;
        let mut xf: Option<LoadedVertexDraw> = None;
        let be16 = |o: usize| {
            dl.get(o..o + 2)
                .map_or(0, |b| u16::from_be_bytes([b[0], b[1]]))
        };
        while at < dl.len() {
            let cmd = u32::from(dl[at]);
            if cmd == 0 {
                break;
            }
            match cmd {
                command::LOAD_INDX_A | command::LOAD_INDX_C => {
                    cur = None;
                    let d =
                        xf.get_or_insert_with(|| LoadedVertexDraw::new(out.indices.len() as u32));
                    let index = be16(at + 1);
                    let addr_len = be16(at + 3);
                    let addr = u32::from(addr_len & 0x0FFF);
                    // Position matrices from 0x000, texture matrices by
                    // convention from 0x078; 3x4 floats each.
                    let (base, table) = if cmd == command::LOAD_INDX_A {
                        (0, &mut d.pos_matrix_table)
                    } else {
                        (0x78, &mut d.tex_matrix_table)
                    };
                    let slot = addr.wrapping_sub(base) / 12;
                    if let Some(s) = table.get_mut(slot as usize) {
                        *s = index;
                    }
                    at += 5;
                    continue;
                }
                command::LOAD_INDX_B | command::LOAD_INDX_D => {
                    at += 5;
                    continue;
                }
                _ => {}
            }
            let prim = cmd & 0xF8;
            let vtx_fmt = (cmd & 7) as usize;
            if !(command::DRAW_QUADS..=command::DRAW_POINTS).contains(&prim)
                || self.vat[vtx_fmt].is_none()
            {
                break;
            }
            let n = u32::from(be16(at + 1));
            at += 3;
            let draw_index = match cur {
                Some(i) => i,
                None => {
                    let d = xf
                        .take()
                        .unwrap_or_else(|| LoadedVertexDraw::new(out.indices.len() as u32));
                    out.draws.push(d);
                    cur = Some(out.draws.len() - 1);
                    out.draws.len() - 1
                }
            };
            let before = out.indices.len();
            triangulate(prim, first_vertex + total_vertices, n, &mut out.indices);
            out.draws[draw_index].index_count += (out.indices.len() - before) as u32;
            out.draw_calls.push(DrawCall {
                prim,
                vtx_fmt,
                src_offs: at,
                vertex_count: n as usize,
            });
            total_vertices += n;
            at += self.src_sizes[vtx_fmt] * n as usize;
        }
        out.end_offs = at;
        out.vertex_count = total_vertices;
        out.vertices = vec![0; self.layout.stride * total_vertices as usize];
        out
    }

    /// Fill `data.vertices` from the display list it was parsed from.
    pub fn load_vertex_data(
        &self,
        data: &mut LoadedVertexData,
        dl: &[u8],
        arrays: &[GxArray<'_>; ATTR_COUNT],
    ) {
        let mut dst = std::mem::take(&mut data.vertices);
        let mut o = 0;
        for c in &data.draw_calls {
            let mut src = c.src_offs;
            for _ in 0..c.vertex_count {
                src = self.load_vertex(
                    c.vtx_fmt,
                    dl,
                    src,
                    arrays,
                    &mut dst[o..o + self.layout.stride],
                );
                o += self.layout.stride;
            }
        }
        data.vertices = dst;
    }

    /// Parse and load in one go.
    pub fn run_vertices(&self, dl: &[u8], arrays: &[GxArray<'_>; ATTR_COUNT]) -> LoadedVertexData {
        let mut d = self.parse_display_list(dl, 0);
        self.load_vertex_data(&mut d, dl, arrays);
        d
    }

    /// Load one vertex of format `fmt` from `dl` at `at` into `dst`;
    /// returns the offset after it.
    pub fn load_vertex(
        &self,
        fmt: usize,
        dl: &[u8],
        mut at: usize,
        arrays: &[GxArray<'_>; ATTR_COUNT],
        dst: &mut [u8],
    ) -> usize {
        let Some(vat) = &self.vat[fmt] else { return at };
        for a in 0..ATTR_COUNT as u32 {
            let d = &self.vcd[a as usize];
            let f = &vat[a as usize];
            match d.ty {
                attr_type::DIRECT => {
                    if d.output == OutputMode::VertexData {
                        self.write_attr(a, f, dl, at, dst, 0);
                    }
                    at += attribute_byte_size(a, f);
                }
                attr_type::INDEX8 | attr_type::INDEX16 => {
                    let wide = d.ty == attr_type::INDEX16;
                    for k in 0..index_components(a, f) {
                        let idx = if wide {
                            dl.get(at..at + 2)
                                .map_or(0, |b| usize::from(u16::from_be_bytes([b[0], b[1]])))
                        } else {
                            usize::from(dl.get(at).copied().unwrap_or(0))
                        };
                        at += if wide { 2 } else { 1 };
                        match d.output {
                            OutputMode::VertexData => {
                                let arr = &arrays[a as usize];
                                if let Some(u) = arr.used {
                                    // An attribute is at most 32 bytes past its index's start.
                                    u.set(u.get().max(idx * arr.stride + arr.stride.max(32)));
                                }
                                self.write_attr(a, f, arr.data, idx * arr.stride, dst, k);
                            }
                            OutputMode::Index => {
                                let off = self.layout.attr_offsets[a as usize];
                                put_f32(dst, off, idx as f32);
                            }
                            OutputMode::None => {}
                        }
                    }
                }
                attr_type::ZERO => {
                    let off = self.layout.attr_offsets[a as usize];
                    match self.layout.attr_formats[a as usize] {
                        FieldFormat::F32(n) => {
                            (0..n as usize).for_each(|i| put_f32(dst, off + 4 * i, 0.0))
                        }
                        FieldFormat::U8 => put_u8(dst, off, 0),
                        FieldFormat::U8Norm => (0..4).for_each(|i| put_u8(dst, off + i, 0)),
                        FieldFormat::None => {}
                    }
                }
                _ => {}
            }
        }
        at
    }

    /// Write attribute `a` read from `src` at `so`. `nbt3` selects which of
    /// N/B/T an NBT3 index addresses.
    fn write_attr(
        &self,
        a: u32,
        f: &VtxAttrFmt,
        src: &[u8],
        so: usize,
        dst: &mut [u8],
        nbt3: usize,
    ) {
        let off = self.layout.attr_offsets[a as usize];
        if is_mtx_idx(a) {
            let v = src.get(so).copied().unwrap_or(0) / 3;
            match self.layout.attr_formats[a as usize] {
                FieldFormat::U8 => put_u8(dst, off, v),
                _ => put_f32(dst, off, f32::from(v)),
            }
            return;
        }
        if is_color(a) {
            let c = read_color(f.comp_type, src, so);
            for (i, v) in c.iter().enumerate() {
                put_u8(dst, off + i, (v * 255.0).round() as u8);
            }
            return;
        }
        let shift = comp_shift(a, f);
        let cs = comp_byte_size(a, f);
        let n = comp_count(a, f);
        let read = |i: usize| read_comp(f.comp_type, src, so + i * cs, shift);
        if a == attr::NRM {
            // NRM, or NBT (9 components), or one of NBT3's three vectors.
            let inputs = [VertexInput::Nrm, VertexInput::Binrm, VertexInput::Tangent];
            let vectors = if n == 9 { 3 } else { 1 };
            for v in 0..vectors {
                let target = if n == 9 { v } else { nbt3 };
                let Some(l) = self.layout.input(inputs[target]) else {
                    continue;
                };
                for c in 0..3 {
                    put_f32(dst, l.offset + 4 * c, read(v * 3 + c));
                }
            }
            return;
        }
        let FieldFormat::F32(dn) = self.layout.attr_formats[a as usize] else {
            return;
        };
        for i in 0..dn as usize {
            put_f32(dst, off + 4 * i, if i < n { read(i) } else { 0.0 });
        }
    }
}

fn put_f32(dst: &mut [u8], at: usize, v: f32) {
    if let Some(d) = dst.get_mut(at..at + 4) {
        d.copy_from_slice(&v.to_le_bytes());
    }
}

fn put_u8(dst: &mut [u8], at: usize, v: u8) {
    if let Some(d) = dst.get_mut(at) {
        *d = v;
    }
}

fn read_comp(ct: u32, src: &[u8], at: usize, shift: u32) -> f32 {
    let b = |n: usize| src.get(at..at + n);
    let v = match ct {
        comp_type::F32 => {
            return b(4).map_or(0.0, |b| f32::from_be_bytes([b[0], b[1], b[2], b[3]]))
        }
        comp_type::U8 => f32::from(b(1).map_or(0, |b| b[0])),
        comp_type::S8 => f32::from(b(1).map_or(0, |b| b[0] as i8)),
        comp_type::U16 => f32::from(b(2).map_or(0, |b| u16::from_be_bytes([b[0], b[1]]))),
        comp_type::S16 => f32::from(b(2).map_or(0, |b| i16::from_be_bytes([b[0], b[1]]))),
        _ => 0.0,
    };
    v / (1u32 << shift) as f32
}

fn read_color(ct: u32, src: &[u8], at: usize) -> [f32; 4] {
    let g = |i: usize| f32::from(src.get(at + i).copied().unwrap_or(0));
    match ct {
        comp_type::RGB565 => {
            let t = (g(0) as u32) << 8 | g(1) as u32;
            [
                ((t >> 11) & 0x1F) as f32 / 31.0,
                ((t >> 5) & 0x3F) as f32 / 63.0,
                (t & 0x1F) as f32 / 31.0,
                1.0,
            ]
        }
        comp_type::RGB8 | comp_type::RGBX8 => [g(0) / 255.0, g(1) / 255.0, g(2) / 255.0, 1.0],
        comp_type::RGBA4 => {
            let t = (g(0) as u32) << 8 | g(1) as u32;
            [12, 8, 4, 0].map(|s| ((t >> s) & 0xF) as f32 / 15.0)
        }
        comp_type::RGBA6 => {
            let t = (g(0) as u32) << 16 | (g(1) as u32) << 8 | g(2) as u32;
            [18, 12, 6, 0].map(|s| ((t >> s) & 0x3F) as f32 / 63.0)
        }
        _ => [g(0) / 255.0, g(1) / 255.0, g(2) / 255.0, g(3) / 255.0],
    }
}

/// Join loaded display lists into one vertex and index buffer (indices of
/// later lists are rebased).
pub fn coalesce_loaded_datas(datas: &[LoadedVertexData]) -> LoadedVertexData {
    let mut out = LoadedVertexData::default();
    for d in datas {
        let base_index = out.indices.len() as u32;
        let base_vertex = out.vertex_count;
        for draw in &d.draws {
            let mut nd = draw.clone();
            nd.index_offset += base_index;
            match out.draws.last_mut() {
                Some(last)
                    if last.index_offset + last.index_count == nd.index_offset
                        && last.pos_matrix_table == nd.pos_matrix_table
                        && last.tex_matrix_table == nd.tex_matrix_table =>
                {
                    last.index_count += nd.index_count;
                }
                _ => out.draws.push(nd),
            }
        }
        out.indices
            .extend(d.indices.iter().map(|i| i + base_vertex));
        out.vertices.extend_from_slice(&d.vertices);
        out.vertex_count += d.vertex_count;
    }
    out
}

/// BP / CP / XF register state set by a display list's register loads.
#[derive(Debug, Clone)]
pub struct DisplayListRegisters {
    pub bp: [u32; 0x100],
    pub cp: [u32; 0x100],
    xf: Vec<u32>,
    /// TEV color registers: the same BP address holds a "color" and a
    /// "konst" bank.
    pub tev_kcolor: [u32; 16],
}

impl Default for DisplayListRegisters {
    fn default() -> Self {
        let mut r = Self {
            bp: [0; 0x100],
            cp: [0; 0x100],
            xf: vec![0; 0x1000],
            tev_kcolor: [0; 16],
        };
        r.bp[bp_register::SS_MASK as usize] = 0x00FF_FFFF;
        r
    }
}

impl DisplayListRegisters {
    pub fn bp_is_set(&self, addr: u32) -> bool {
        self.bp[addr as usize & 0xFF] & (1 << 31) != 0
    }

    /// One BP load: register address in the top byte, value below, through
    /// the write mask.
    pub fn bp_set(&mut self, bag: u32) {
        let addr = (bag >> 24) as usize;
        let mask = self.bp[bp_register::SS_MASK as usize];
        let value = (self.bp[addr] & !mask) | (bag & mask);
        if addr != bp_register::SS_MASK as usize {
            self.bp[bp_register::SS_MASK as usize] = 0x00FF_FFFF;
        }
        self.bp[addr] = (1 << 31) | value;
        let base = bp_register::TEV_REGISTERL_0_ID as usize;
        if (base..=base + 8).contains(&addr) {
            let kci = addr - base;
            let bank = ((value >> 23) & 1) as usize;
            if let Some(s) = self.tev_kcolor.get_mut(bank * 8 + kci) {
                *s = value;
            }
        }
    }

    pub fn xf_set(&mut self, reg: u32, sub: usize, v: u32) {
        if let Some(s) = reg
            .checked_sub(0x1000)
            .and_then(|i| self.xf.get_mut(i as usize * 0x10 + sub))
        {
            *s = v;
        }
    }

    pub fn xf_get(&self, reg: u32, sub: usize) -> u32 {
        reg.checked_sub(0x1000)
            .and_then(|i| self.xf.get(i as usize * 0x10 + sub))
            .copied()
            .unwrap_or(0)
    }

    /// GX's initial swap tables.
    pub fn init_gx(&mut self) {
        for i in (0..8).step_by(2) {
            self.bp_set(((bp_register::TEV_KSEL_0_ID + i) << 24) | 0b0100);
            self.bp_set(((bp_register::TEV_KSEL_0_ID + i + 1) << 24) | 0b1110);
        }
    }

    /// Run a register-only display list (NOP, BP, CP and XF loads). Returns
    /// the offset of the first other command, if any.
    pub fn run(&mut self, dl: &[u8]) -> Option<usize> {
        let mut i = 0usize;
        let be32 = |o: usize| {
            dl.get(o..o + 4)
                .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
        };
        let be16 = |o: usize| dl.get(o..o + 2).map(|b| u16::from_be_bytes([b[0], b[1]]));
        while i < dl.len() {
            let start = i;
            let cmd = u32::from(dl[i]);
            i += 1;
            match cmd {
                command::NOOP => {}
                command::LOAD_BP_REG => {
                    self.bp_set(be32(i)?);
                    i += 4;
                }
                command::LOAD_CP_REG => {
                    let addr = usize::from(*dl.get(i)?);
                    self.cp[addr] = be32(i + 1)?;
                    i += 5;
                }
                command::LOAD_XF_REG => {
                    let len = usize::from(be16(i)?) + 1;
                    let addr = u32::from(be16(i + 2)?);
                    i += 4;
                    for j in 0..16 {
                        let v = if j < len { be32(i + 4 * j)? } else { 0 };
                        self.xf_set(addr, j, v);
                    }
                    i += 4 * len;
                }
                _ => return Some(start),
            }
        }
        None
    }
}

/// A readable dump of a register display list.
pub fn display_list_to_string(dl: &[u8]) -> String {
    let mut s = String::new();
    let mut i = 0usize;
    let be32 = |o: usize| {
        dl.get(o..o + 4)
            .map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    };
    let be16 = |o: usize| {
        dl.get(o..o + 2)
            .map_or(0, |b| u16::from_be_bytes([b[0], b[1]]))
    };
    let mut ss_mask = 0x00FF_FFFFu32;
    while i < dl.len() {
        let cmd = u32::from(dl[i]);
        i += 1;
        match cmd {
            command::NOOP => {}
            command::LOAD_BP_REG => {
                let bag = be32(i);
                i += 4;
                let addr = bag >> 24;
                let v = bag & ss_mask;
                ss_mask = if addr == bp_register::SS_MASK {
                    v
                } else {
                    0x00FF_FFFF
                };
                s += &format!("Set BP {addr:#x} to {v:#x}\n");
            }
            command::LOAD_CP_REG => {
                let addr = dl.get(i).copied().unwrap_or(0);
                s += &format!("Set CP {addr:#x} to {:#x}\n", be32(i + 1));
                i += 5;
            }
            command::LOAD_XF_REG => {
                let len = usize::from(be16(i)) + 1;
                let addr = usize::from(be16(i + 2));
                i += 4;
                for j in 0..len {
                    s += &format!("Set XF {:#x} to {:#x}\n", addr + j, be32(i));
                    i += 4;
                }
            }
            _ => {
                s += &format!("Unknown command {cmd:#x} at {}\n", i - 1);
                break;
            }
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vcd_with(entries: &[(u32, u32)]) -> Vcd {
        let mut v = [VtxDesc::default(); ATTR_COUNT];
        for &(a, ty) in entries {
            v[a as usize].ty = ty;
        }
        v
    }

    #[test]
    fn indexed_triangle_strip() {
        let vcd = vcd_with(&[
            (attr::POS, attr_type::INDEX8),
            (attr::CLR0, attr_type::DIRECT),
        ]);
        let mut vat = [VtxAttrFmt::default(); ATTR_COUNT];
        vat[attr::POS as usize] = VtxAttrFmt {
            comp_type: comp_type::S16,
            comp_cnt: comp_cnt::POS_XYZ,
            comp_shift: 1,
        };
        vat[attr::CLR0 as usize] = VtxAttrFmt {
            comp_type: comp_type::RGBA8,
            comp_cnt: comp_cnt::CLR_RGBA,
            comp_shift: 0,
        };
        let loader = VtxLoader::new(vat, vcd);
        // Positions: index i -> (2i, -2, 4) before the shift of 1.
        let mut pos = Vec::new();
        for i in 0..4i16 {
            for c in [2 * i, -2, 4] {
                pos.extend(c.to_be_bytes());
            }
        }
        let mut dl = vec![0x98, 0x00, 0x04];
        for i in 0..4u8 {
            dl.extend([i, 0xFF, 0x80, 0x00, 0xFF]);
        }
        dl.push(0);
        let mut arrays = [GxArray::default(); ATTR_COUNT];
        arrays[attr::POS as usize] = GxArray {
            data: &pos,
            stride: 6,
            used: None,
        };
        let d = loader.run_vertices(&dl, &arrays);
        assert_eq!(d.vertex_count, 4);
        assert_eq!(d.indices, vec![0, 1, 2, 1, 3, 2]);
        let stride = loader.layout.stride;
        let p = loader.layout.input(VertexInput::Pos).unwrap().offset;
        let f = |o: usize| f32::from_le_bytes(d.vertices[o..o + 4].try_into().unwrap());
        assert_eq!(
            [
                f(stride * 3 + p),
                f(stride * 3 + p + 4),
                f(stride * 3 + p + 8)
            ],
            [3.0, -1.0, 2.0]
        );
        let c = loader.layout.input(VertexInput::Clr0).unwrap().offset;
        assert_eq!(&d.vertices[c..c + 4], &[0xFF, 0x80, 0x00, 0xFF]);
    }

    #[test]
    fn registers() {
        let mut r = DisplayListRegisters::default();
        // BP 0x41 = 0x123456, then XF 0x1009 (one value) = 2.
        let dl = [
            0x61, 0x41, 0x12, 0x34, 0x56, 0x10, 0x00, 0x00, 0x10, 0x09, 0, 0, 0, 2, 0x00,
        ];
        assert_eq!(r.run(&dl), None);
        assert!(r.bp_is_set(0x41));
        assert_eq!(r.bp[0x41] & 0xFFFFFF, 0x123456);
        assert_eq!(r.xf_get(0x1009, 0), 2);
    }
}
