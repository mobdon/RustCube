// Derived from noclip.website (https://github.com/magcius/noclip.website),
// src/gx/gx_render.ts at commit 2cebac2a5d1d7c44535397a14f51e764ca823767.
// Copyright (c) 2018 Jasper St. Pierre. MIT License: see THIRD-PARTY.md.
// Translated from TypeScript to Rust for rustcube-gx.
//! The per-scene, per-material and per-draw parameters and their packing
//! into the uniform blocks `shader.rs` declares. The layout is fixed (the
//! optional blocks are always present), so one bind group layout serves
//! every material.
use crate::material::{FogBlock, Light};

/// Indices into [`MaterialParams::colors`] (noclip's `ColorKind`).
pub mod color_kind {
    pub const MAT0: usize = 0;
    pub const MAT1: usize = 1;
    pub const AMB0: usize = 2;
    pub const AMB1: usize = 3;
    pub const K0: usize = 4;
    pub const K1: usize = 5;
    pub const K2: usize = 6;
    pub const K3: usize = 7;
    pub const CPREV: usize = 8;
    pub const C0: usize = 9;
    pub const C1: usize = 10;
    pub const C2: usize = 11;
    pub const COUNT: usize = 12;
}

/// A 3x4 row-major matrix (GX's `Mtx`).
pub type Mtx34 = [[f32; 4]; 3];

pub const IDENTITY34: Mtx34 = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
];

/// Handle of a texture held by the renderer.
pub type TextureId = u32;

/// Sampler state from `GXInitTexObj` / `GXInitTexObjLOD`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SamplerDesc {
    pub wrap_s: u32,
    pub wrap_t: u32,
    pub min_filter: u32,
    pub mag_filter: u32,
    /// LOD clamp, in 1/16 steps of a level (bit-exact for hashing).
    pub min_lod_x16: u16,
    pub max_lod_x16: u16,
    pub max_aniso: u32,
}

impl Default for SamplerDesc {
    fn default() -> Self {
        Self {
            wrap_s: crate::enums::wrap_mode::REPEAT,
            wrap_t: crate::enums::wrap_mode::REPEAT,
            min_filter: crate::enums::tex_filter::LINEAR,
            mag_filter: crate::enums::tex_filter::LINEAR,
            min_lod_x16: 0,
            max_lod_x16: 10 * 16,
            max_aniso: crate::enums::anisotropy::_1,
        }
    }
}

/// One texture map slot of a draw.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TextureMapping {
    pub texture: Option<TextureId>,
    pub sampler: SamplerDesc,
    pub width: f32,
    pub height: f32,
    pub lod_bias: f32,
    pub flip_y: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MaterialParams {
    pub texture_mappings: [TextureMapping; 8],
    /// RGBA 0..1, by [`color_kind`]. TEV registers may hold -4..4 (S10).
    pub colors: [[f32; 4]; color_kind::COUNT],
    pub tex_mtx: [Mtx34; 10],
    pub post_tex_mtx: [Mtx34; 20],
    /// Indirect matrices: rows (m00 m01 m02 scale), (m10 m11 m12 0) with
    /// the scale already applied (see [`ind_tex_mtx`]).
    pub ind_tex_mtx: [[[f32; 4]; 2]; 3],
    pub lights: [Light; 8],
    pub fog: FogBlock,
    /// Alpha references, 0..1, for materials with a dynamic alpha test.
    pub dynamic_alpha_ref: [f32; 2],
}

impl Default for MaterialParams {
    fn default() -> Self {
        let mut colors = [[0.0; 4]; color_kind::COUNT];
        colors[color_kind::MAT0] = [1.0; 4];
        colors[color_kind::MAT1] = [1.0; 4];
        Self {
            texture_mappings: [TextureMapping::default(); 8],
            colors,
            tex_mtx: [IDENTITY34; 10],
            post_tex_mtx: [IDENTITY34; 20],
            ind_tex_mtx: [[[0.0; 4]; 2]; 3],
            lights: [Light::default(); 8],
            fog: FogBlock::default(),
            dynamic_alpha_ref: [0.0; 2],
        }
    }
}

/// `GXSetIndTexMtx`: a 2x3 matrix and a power-of-two scale exponent.
pub fn ind_tex_mtx(m: [[f32; 3]; 2], scale_exp: i8) -> [[f32; 4]; 2] {
    let s = 2f32.powi(i32::from(scale_exp));
    [
        [m[0][0] * s, m[0][1] * s, m[0][2] * s, s],
        [m[1][0] * s, m[1][1] * s, m[1][2] * s, 0.0],
    ]
}

#[derive(Debug, Clone, PartialEq)]
pub struct DrawParams {
    pub pos_mtx: [Mtx34; 10],
}

impl Default for DrawParams {
    fn default() -> Self {
        Self {
            pos_mtx: [IDENTITY34; 10],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneParams {
    /// Column-major clip-from-view matrix with wgpu depth (0..1).
    pub projection: [[f32; 4]; 4],
    pub lod_bias: f32,
}

impl Default for SceneParams {
    fn default() -> Self {
        Self {
            projection: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
            lod_bias: 0.0,
        }
    }
}

pub const EFB_WIDTH: f32 = 640.0;
pub const EFB_HEIGHT: f32 = 528.0;

pub const SCENE_PARAMS_SIZE: usize = 80;
pub const MATERIAL_PARAMS_SIZE: usize = 2560;
pub const DRAW_PARAMS_SIZE: usize = 480;

/// Writes uniform data straight into a byte buffer (no vector per block:
/// a frame packs ~2,500 material blocks).
struct Fill<'a>(&'a mut Vec<u8>);

impl Fill<'_> {
    fn v4(&mut self, v: [f32; 4]) {
        self.0.extend_from_slice(bytemuck::cast_slice(&v));
    }
    fn m34(&mut self, m: &Mtx34) {
        m.iter().for_each(|r| self.v4(*r));
    }
    fn v3(&mut self, v: [f32; 3]) {
        self.v4([v[0], v[1], v[2], 0.0]);
    }
}

pub fn fill_scene_params(p: &SceneParams) -> Vec<u8> {
    let mut out = Vec::with_capacity(80);
    let mut f = Fill(&mut out);
    p.projection.iter().for_each(|c| f.v4(*c));
    f.v4([p.lod_bias, 0.0, 0.0, 0.0]);
    out
}

pub fn fill_material_params(p: &MaterialParams) -> Vec<u8> {
    let mut out = Vec::with_capacity(MATERIAL_PARAMS_SIZE);
    fill_material_params_into(p, &mut out);
    out
}

/// [`fill_material_params`] appended to `out`.
pub fn fill_material_params_into(p: &MaterialParams, out: &mut Vec<u8>) {
    let at = out.len();
    let mut f = Fill(out);
    p.colors.iter().for_each(|c| f.v4(*c));
    p.tex_mtx.iter().for_each(|m| f.m34(m));
    for pair in p.texture_mappings.chunks(2) {
        let h = |m: &TextureMapping| if m.flip_y { -m.height } else { m.height };
        f.v4([pair[0].width, h(&pair[0]), pair[1].width, h(&pair[1])]);
    }
    for quad in p.texture_mappings.chunks(4) {
        f.v4([
            quad[0].lod_bias,
            quad[1].lod_bias,
            quad[2].lod_bias,
            quad[3].lod_bias,
        ]);
    }
    for m in &p.ind_tex_mtx {
        f.v4(m[0]);
        f.v4(m[1]);
    }
    p.post_tex_mtx.iter().for_each(|m| f.m34(m));
    for l in &p.lights {
        f.v4(l.color);
        f.v3(l.position);
        f.v3(l.direction);
        f.v3(l.dist_atten);
        f.v3(l.cos_atten);
    }
    let fog = &p.fog;
    f.v4([fog.a, fog.b, fog.c, fog.adj_center]);
    let t = fog.adj_table.map(f32::from);
    f.v4([t[0], t[1], t[2], t[3]]);
    f.v4([t[4], t[5], t[6], t[7]]);
    f.v4([t[8], t[9], 0.0, 0.0]);
    f.v4(fog.color);
    f.v4([p.dynamic_alpha_ref[0], p.dynamic_alpha_ref[1], 0.0, 0.0]);
    debug_assert_eq!(out.len() - at, MATERIAL_PARAMS_SIZE);
}

pub fn fill_draw_params(p: &DrawParams) -> Vec<u8> {
    let mut out = Vec::with_capacity(DRAW_PARAMS_SIZE);
    fill_draw_params_into(p, &mut out);
    out
}

/// [`fill_draw_params`] appended to `out`.
pub fn fill_draw_params_into(p: &DrawParams, out: &mut Vec<u8>) {
    let mut f = Fill(out);
    p.pos_mtx.iter().for_each(|m| f.m34(m));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_sizes_match_the_wgsl() {
        assert_eq!(
            fill_scene_params(&SceneParams::default()).len(),
            SCENE_PARAMS_SIZE
        );
        assert_eq!(
            fill_material_params(&MaterialParams::default()).len(),
            MATERIAL_PARAMS_SIZE
        );
        assert_eq!(
            fill_draw_params(&DrawParams::default()).len(),
            DRAW_PARAMS_SIZE
        );
    }
}
