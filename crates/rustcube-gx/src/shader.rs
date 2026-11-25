// Derived from noclip.website (https://github.com/magcius/noclip.website),
// src/gx/gx_material.ts (GX_Program) at commit
// 2cebac2a5d1d7c44535397a14f51e764ca823767.
// Copyright (c) 2018 Jasper St. Pierre. MIT License: see THIRD-PARTY.md.
// Translated from TypeScript (GLSL output) to Rust (WGSL output) for rustcube-gx.
//! WGSL generation for a [`GxMaterial`]: light channels and lighting,
//! texgens (matrix, bump, SRTG, post matrices), indirect stages, every TEV
//! stage with swap tables, konst selects, compare ops and overflow, alpha
//! test, fog and destination alpha.
//!
//! WGSL has no swizzle assignment or overloading, so stage outputs are
//! written as whole vectors and helpers carry a size suffix. Other changes
//! from noclip: the uniform blocks always have their optional parts (fixed
//! layout, see `params.rs`); vertex inputs the vertex layout lacks become
//! constants; reverse-exponential fog types are matched on their low bits;
//! specular attenuation uses the computed N·L term.
use crate::enums::*;
use crate::material::{ColorChannelControl, GxMaterial, LightChannelControl, SwapTable, TevStage};
use crate::vtx::VertexInput;
use std::fmt::Write;

/// Uniform and texture bindings: group 0 = scene (0), material (1), draw
/// (2); group 1 = texture `i` at binding `2i`, its sampler at `2i + 1`.
pub const BINDINGS: &str = r#"
struct Mat3x4 { mx: vec4f, my: vec4f, mz: vec4f, };
struct Mat2x4 { mx: vec4f, my: vec4f, };
struct Light { Color: vec4f, Position: vec4f, Direction: vec4f, DistAtten: vec4f, CosAtten: vec4f, };
struct FogBlock { Param: vec4f, AdjTable: array<vec4f, 3>, Color: vec4f, };
struct SceneParams { Projection: mat4x4f, Misc0: vec4f, };
struct MaterialParams {
    ColorMatReg: array<vec4f, 2>,
    ColorAmbReg: array<vec4f, 2>,
    KonstColor: array<vec4f, 4>,
    Color: array<vec4f, 4>,
    TexMtx: array<Mat3x4, 10>,
    TextureSizes: array<vec4f, 4>,
    TextureBiases: array<vec4f, 2>,
    IndTexMtx: array<Mat2x4, 3>,
    PostTexMtx: array<Mat3x4, 20>,
    LightParams: array<Light, 8>,
    Fog: FogBlock,
    DynamicAlphaParams: vec4f,
};
struct DrawParams { PosMtx: array<Mat3x4, 10>, };
@group(0) @binding(0) var<uniform> u_Scene: SceneParams;
@group(0) @binding(1) var<uniform> u_Material: MaterialParams;
@group(0) @binding(2) var<uniform> u_Draw: DrawParams;
@group(1) @binding(0) var u_Texture0: texture_2d<f32>;
@group(1) @binding(1) var u_Sampler0: sampler;
@group(1) @binding(2) var u_Texture1: texture_2d<f32>;
@group(1) @binding(3) var u_Sampler1: sampler;
@group(1) @binding(4) var u_Texture2: texture_2d<f32>;
@group(1) @binding(5) var u_Sampler2: sampler;
@group(1) @binding(6) var u_Texture3: texture_2d<f32>;
@group(1) @binding(7) var u_Sampler3: sampler;
@group(1) @binding(8) var u_Texture4: texture_2d<f32>;
@group(1) @binding(9) var u_Sampler4: sampler;
@group(1) @binding(10) var u_Texture5: texture_2d<f32>;
@group(1) @binding(11) var u_Sampler5: sampler;
@group(1) @binding(12) var u_Texture6: texture_2d<f32>;
@group(1) @binding(13) var u_Sampler6: sampler;
@group(1) @binding(14) var u_Texture7: texture_2d<f32>;
@group(1) @binding(15) var u_Sampler7: sampler;

fn Mul34(m: Mat3x4, v: vec4f) -> vec3f { return vec3f(dot(m.mx, v), dot(m.my, v), dot(m.mz, v)); }
fn Mul24(m: Mat2x4, v: vec4f) -> vec2f { return vec2f(dot(m.mx, v), dot(m.my, v)); }
fn MulNormalMatrix(m: Mat3x4, v: vec4f) -> vec3f {
    // Undo non-uniform scale: divide by each column's squared length.
    let c0 = vec3f(m.mx.x, m.my.x, m.mz.x);
    let c1 = vec3f(m.mx.y, m.my.y, m.mz.y);
    let c2 = vec3f(m.mx.z, m.my.z, m.mz.z);
    let s = vec4f(dot(c0, c0), dot(c1, c1), dot(c2, c2), 1.0);
    return Mul34(m, v / s);
}
fn TevOverflow1(a: f32) -> f32 { return f32(i32(a * 255.0) & 255) / 255.0; }
fn TevOverflow4(a: vec4f) -> vec4f { return vec4f(vec4i(a * 255.0) & vec4i(255)) / 255.0; }
"#;

const VERTEX_LIB: &str = r#"
fn IdentityMtx() -> Mat3x4 { return Mat3x4(vec4f(1.0, 0.0, 0.0, 0.0), vec4f(0.0, 1.0, 0.0, 0.0), vec4f(0.0, 0.0, 1.0, 0.0)); }
fn GetPosTexMatrix(f: f32) -> Mat3x4 {
    let i = u32(f + 0.5);
    if (i == 20u) { return IdentityMtx(); }
    if (i >= 10u) { return u_Material.TexMtx[min(i - 10u, 9u)]; }
    return u_Draw.PosMtx[i];
}
fn ApplyAttenuation(c: vec3f, v: f32) -> f32 { return dot(c, vec3f(1.0, v, v * v)); }
fn ApplyLight(L: Light, attnFn: i32, diffFn: i32, P: vec3f, N: vec3f) -> vec4f {
    let delta = L.Position.xyz - P;
    let dist2 = dot(delta, delta);
    let dist = sqrt(dist2);
    let dir = delta / dist;
    var atten = 1.0;
    if (attnFn == 1) {
        // Spot / distance.
        let cosV = max(0.0, dot(dir, L.Direction.xyz));
        let cosA = max(0.0, ApplyAttenuation(L.CosAtten.xyz, cosV));
        let distA = dot(L.DistAtten.xyz, vec3f(1.0, dist, dist2));
        atten = max(0.0, cosA / distA);
    } else if (attnFn == 0) {
        // Specular.
        let attn = select(0.0, max(0.0, dot(N, L.Direction.xyz)), dot(N, dir) >= 0.0);
        let cosA = ApplyAttenuation(L.CosAtten.xyz, attn);
        var distV = L.DistAtten.xyz;
        if (diffFn != 0) { distV = normalize(distV); }
        let distA = max(0.0, ApplyAttenuation(distV, attn));
        atten = max(0.0, cosA / distA);
    }
    var diffuse = 1.0;
    if (attnFn != 0) {
        if (diffFn == 1) { diffuse = dot(N, dir); }
        else if (diffFn == 2) { diffuse = max(dot(N, dir), 0.0); }
    }
    return diffuse * atten * L.Color;
}
fn ApplyLightChannel(mask: i32, attnFn: i32, diffFn: i32, P: vec3f, N: vec3f) -> vec4f {
    var acc = vec4f(0.0);
    for (var i = 0; i < 8; i++) {
        if ((mask & (1 << u32(i))) != 0) {
            acc += ApplyLight(u_Material.LightParams[i], attnFn, diffFn, P, N);
        }
    }
    return acc;
}
"#;

const FRAGMENT_LIB: &str = r#"
fn TevPack16(a: vec2f) -> f32 { return dot(a, vec2f(1.0, 256.0)); }
fn TevPack24(a: vec3f) -> f32 { return dot(a, vec3f(1.0, 256.0, 256.0 * 256.0)); }
fn TevPerCompGT1(a: f32, b: f32) -> f32 { return select(0.0, 1.0, a > b); }
fn TevPerCompEQ1(a: f32, b: f32) -> f32 { return select(0.0, 1.0, a == b); }
fn TevPerCompGT3(a: vec3f, b: vec3f) -> vec3f { return select(vec3f(0.0), vec3f(1.0), a > b); }
fn TevPerCompEQ3(a: vec3f, b: vec3f) -> vec3f { return select(vec3f(0.0), vec3f(1.0), a == b); }
fn TevMask(n: f32, mask: i32) -> f32 { return f32(i32(n * 255.0) & mask) / 255.0; }
fn GxMod(a: f32, b: f32) -> f32 { return a - b * floor(a / b); }
"#;

/// Generated vertex and fragment WGSL (one module, entry points `vs` and
/// `fs`).
pub struct GxProgram {
    pub source: String,
}

/// The material tweaks noclip exposes as "hacks".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct MaterialHacks {
    pub disable_textures: bool,
    pub disable_vertex_colors: bool,
    pub disable_lighting: bool,
}

fn float(v: f32) -> String {
    let s = format!("{v:?}");
    if s.contains('.') || s.contains('e') {
        s
    } else {
        format!("{s}.0")
    }
}

fn channels_equal(a: &ColorChannelControl, b: &ColorChannelControl) -> bool {
    a == b
}

fn tex_gen_is_2d(ty: u32) -> bool {
    ty == tex_gen_type::MTX2x4 || ty == tex_gen_type::SRTG
}

struct Gen<'a> {
    m: &'a GxMaterial,
    hacks: MaterialHacks,
    /// Inputs the vertex buffer provides.
    has: &'a dyn Fn(VertexInput) -> bool,
}

impl Gen<'_> {
    // ---- vertex inputs -------------------------------------------------

    fn uses_color_channel(c: &ColorChannelControl) -> bool {
        c.mat_color_source == color_src::VTX || c.amb_color_source == color_src::VTX
    }
    fn uses_light_channel(c: Option<&LightChannelControl>) -> bool {
        c.is_some_and(|c| Self::uses_color_channel(&c.color) || Self::uses_color_channel(&c.alpha))
    }
    fn uses_normal_channel(c: &ColorChannelControl) -> bool {
        if !c.lighting_enabled || c.lit_mask == 0 {
            return false;
        }
        c.diffuse_function != diffuse_function::NONE
            || c.attenuation_function == attenuation_function::SPEC
    }
    fn uses_normal(&self) -> bool {
        self.m
            .light_channels
            .iter()
            .any(|c| Self::uses_normal_channel(&c.color) || Self::uses_normal_channel(&c.alpha))
    }
    fn uses_tex_gen_input(&self, s: u32) -> bool {
        self.m.tex_gens.iter().any(|g| g.source == s)
    }
    fn uses_bump(&self) -> bool {
        self.m
            .tex_gens
            .iter()
            .any(|g| (tex_gen_type::BUMP0..=tex_gen_type::BUMP7).contains(&g.ty))
    }
    fn uses_attrib(&self, a: VertexInput) -> bool {
        let tmi = |r: std::ops::Range<usize>| r.into_iter().any(|i| self.m.use_tex_mtx_idx[i]);
        match a {
            VertexInput::Pos => true,
            VertexInput::Tex0123MtxIdx => tmi(0..4),
            VertexInput::Tex4567MtxIdx => tmi(4..8),
            VertexInput::Nrm => self.uses_normal() || self.uses_tex_gen_input(tex_gen_src::NRM),
            VertexInput::Tangent => {
                self.uses_tex_gen_input(tex_gen_src::TANGENT) || self.uses_bump()
            }
            VertexInput::Binrm => self.uses_tex_gen_input(tex_gen_src::BINRM) || self.uses_bump(),
            VertexInput::Clr0 => Self::uses_light_channel(self.m.light_channels.first()),
            VertexInput::Clr1 => Self::uses_light_channel(self.m.light_channels.get(1)),
            VertexInput::Tex01 => {
                self.uses_tex_gen_input(tex_gen_src::TEX0)
                    || self.uses_tex_gen_input(tex_gen_src::TEX1)
            }
            VertexInput::Tex23 => {
                self.uses_tex_gen_input(tex_gen_src::TEX2)
                    || self.uses_tex_gen_input(tex_gen_src::TEX3)
            }
            VertexInput::Tex45 => {
                self.uses_tex_gen_input(tex_gen_src::TEX4)
                    || self.uses_tex_gen_input(tex_gen_src::TEX5)
            }
            VertexInput::Tex67 => {
                self.uses_tex_gen_input(tex_gen_src::TEX6)
                    || self.uses_tex_gen_input(tex_gen_src::TEX7)
            }
        }
    }

    // ---- matrices --------------------------------------------------------

    /// vec3 result; `src` is a vec4.
    fn mul_pnt_static(&self, pnt: u32, src: &str, nrm: bool) -> String {
        let (arr, idx) = if pnt == tex_gen_matrix::IDENTITY {
            return format!("({src}).xyz");
        } else if pnt >= tex_gen_matrix::TEXMTX0 {
            ("u_Material.TexMtx", (pnt - tex_gen_matrix::TEXMTX0) / 3)
        } else {
            ("u_Draw.PosMtx", (pnt - tex_gen_matrix::PNMTX0) / 3)
        };
        if nrm {
            format!("MulNormalMatrix({arr}[{idx}], {src})")
        } else {
            format!("Mul34({arr}[{idx}], {src})")
        }
    }
    fn mul_pnt_dynamic(&self, attr: &str, src: &str, nrm: bool) -> String {
        if nrm {
            format!("MulNormalMatrix(GetPosTexMatrix({attr}), {src})")
        } else {
            format!("Mul34(GetPosTexMatrix({attr}), {src})")
        }
    }
    fn mul(&self, attr: &str, pos: bool, nrm: bool) -> String {
        let src = format!("vec4f(({attr}).xyz, {})", if pos { "1.0" } else { "0.0" });
        let g = if self.m.use_pn_mtx_idx {
            self.mul_pnt_dynamic("a_Position.w", &src, nrm)
        } else {
            self.mul_pnt_static(tex_gen_matrix::PNMTX0, &src, nrm)
        };
        if pos {
            g
        } else {
            format!("normalize({g})")
        }
    }

    // ---- color channels --------------------------------------------------

    fn material_source(&self, c: &ColorChannelControl, i: usize) -> String {
        if c.mat_color_source == color_src::VTX {
            if self.hacks.disable_vertex_colors {
                return "vec4f(1.0)".into();
            }
            format!("a_Color{i}")
        } else {
            format!("u_Material.ColorMatReg[{i}]")
        }
    }
    fn ambient_source(&self, c: &ColorChannelControl, i: usize) -> String {
        if c.amb_color_source == color_src::VTX {
            if self.hacks.disable_vertex_colors {
                return "vec4f(1.0)".into();
            }
            format!("a_Color{i}")
        } else {
            format!("u_Material.ColorAmbReg[{i}]")
        }
    }
    fn color_channel(&self, c: &ColorChannelControl, i: usize) -> String {
        let mat = self.material_source(c, i);
        if c.lighting_enabled && !self.hacks.disable_lighting {
            let amb = self.ambient_source(c, i);
            format!(
                "({mat} * clamp({amb} + ApplyLightChannel({}, {}, {}, t_Position, t_Normal), vec4f(0.0), vec4f(1.0)))",
                c.lit_mask, c.attenuation_function, c.diffuse_function
            )
        } else {
            mat
        }
    }
    fn light_channels(&self) -> String {
        let mut s = String::new();
        for (i, ch) in self.m.light_channels.iter().enumerate().take(2) {
            if channels_equal(&ch.color, &ch.alpha) {
                let _ = writeln!(s, "    v_Color{i} = {};", self.color_channel(&ch.color, i));
            } else {
                let _ = writeln!(
                    s,
                    "    v_Color{i} = vec4f(({}).rgb, ({}).a);",
                    self.color_channel(&ch.color, i),
                    self.color_channel(&ch.alpha, i)
                );
            }
        }
        s
    }

    // ---- texgen ------------------------------------------------------------

    fn tex_gen_source(&self, src: u32) -> String {
        match src {
            tex_gen_src::POS => "vec4f(a_Position.xyz, 1.0)".into(),
            tex_gen_src::NRM => "vec4f(a_Normal.xyz, 1.0)".into(),
            tex_gen_src::BINRM => "vec4f(a_Binormal.xyz, 1.0)".into(),
            tex_gen_src::TANGENT => "vec4f(a_Tangent.xyz, 1.0)".into(),
            tex_gen_src::COLOR0 => "v_Color0".into(),
            tex_gen_src::COLOR1 => "v_Color1".into(),
            tex_gen_src::TEX0..=tex_gen_src::TEX7 => {
                let t = src - tex_gen_src::TEX0;
                let names = ["Tex01", "Tex23", "Tex45", "Tex67"];
                let sw = if t & 1 == 0 { "xy" } else { "zw" };
                format!("vec4f(a_{}.{sw}, 1.0, 1.0)", names[(t >> 1) as usize])
            }
            tex_gen_src::TEXCOORD0..=tex_gen_src::TEXCOORD6 => {
                format!("vec4f(t_TexCoord{}, 1.0)", src - tex_gen_src::TEXCOORD0)
            }
            _ => "vec4f(0.0, 0.0, 1.0, 1.0)".into(),
        }
    }
    fn tex_mtx_idx_attr(&self, i: usize) -> String {
        let (v, c) = if i < 4 {
            ("a_TexMtx0123Idx", i)
        } else {
            ("a_TexMtx4567Idx", i - 4)
        };
        format!("round({v}.{} * 255.0)", ["x", "y", "z", "w"][c])
    }
    fn tex_gen_matrix_mult(&self, i: usize, src: &str) -> String {
        if self.m.use_tex_mtx_idx[i] {
            self.mul_pnt_dynamic(&self.tex_mtx_idx_attr(i), src, false)
        } else {
            self.mul_pnt_static(self.m.tex_gens[i].matrix, src, false)
        }
    }
    fn tex_gen_bump(&self, i: usize, src: &str) -> String {
        let light = self.m.tex_gens[i].ty - tex_gen_type::BUMP0;
        let dir = format!("normalize(u_Material.LightParams[{light}].Position.xyz - t_Position)");
        let t = self.mul("a_Tangent", false, false);
        let b = self.mul("a_Binormal", false, false);
        format!("(({src}).xyz + vec3f(dot({dir}, {t}), dot({dir}, {b}), 0.0))")
    }
    fn tex_gen_type(&self, i: usize) -> String {
        let g = &self.m.tex_gens[i];
        let src = self.tex_gen_source(g.source);
        match g.ty {
            tex_gen_type::SRTG => format!("vec3f(({src}).xy, 1.0)"),
            tex_gen_type::MTX2x4 => {
                format!("vec3f(({}).xy, 1.0)", self.tex_gen_matrix_mult(i, &src))
            }
            tex_gen_type::MTX3x4 => self.tex_gen_matrix_mult(i, &src),
            _ => self.tex_gen_bump(i, &src),
        }
    }
    fn tex_gen_post(&self, i: usize) -> String {
        let g = &self.m.tex_gens[i];
        let mut v = self.tex_gen_type(i);
        if g.normalize {
            v = format!("normalize({v})");
        }
        if g.post_matrix == post_tex_gen_matrix::PTIDENTITY {
            v
        } else {
            let idx = (g.post_matrix - post_tex_gen_matrix::PTTEXMTX0) / 3;
            format!("Mul34(u_Material.PostTexMtx[{idx}], vec4f({v}, 1.0))")
        }
    }
    fn tex_gens(&self) -> String {
        let mut s = String::new();
        for (i, g) in self.m.tex_gens.iter().enumerate() {
            let _ = writeln!(
                s,
                "    // TexGen {i} type {} source {} matrix {}",
                g.ty, g.source, g.matrix
            );
            let _ = writeln!(s, "    let t_TexCoord{i} = {};", self.tex_gen_post(i));
            if tex_gen_is_2d(g.ty) {
                let _ = writeln!(s, "    o.v_TexCoord{i} = vec3f(t_TexCoord{i}.xy, 1.0);");
            } else {
                let _ = writeln!(s, "    o.v_TexCoord{i} = t_TexCoord{i};");
            }
        }
        s
    }

    // ---- fragment: textures and indirect ----------------------------------

    fn read_tex_coord(&self, i: u32) -> String {
        match self.m.tex_gens.get(i as usize) {
            Some(g) if tex_gen_is_2d(g.ty) => format!("i.v_TexCoord{i}.xy"),
            Some(_) => format!("(i.v_TexCoord{i}.xy / i.v_TexCoord{i}.z)"),
            None => "vec2f(0.0)".into(),
        }
    }
    fn lod_bias(&self, i: u32) -> String {
        format!(
            "(u_Scene.Misc0.x + u_Material.TextureBiases[{}].{})",
            i / 4,
            ["x", "y", "z", "w"][(i % 4) as usize]
        )
    }
    fn texture_scale(&self, i: u32) -> String {
        format!(
            "u_Material.TextureSizes[{}].{}",
            i / 2,
            if i.is_multiple_of(2) { "xy" } else { "zw" }
        )
    }
    fn texture_sample(&self, i: u32, coord: &str) -> String {
        let i = i.min(7);
        format!(
            "textureSampleBias(u_Texture{i}, u_Sampler{i}, {coord}, {})",
            self.lod_bias(i)
        )
    }
    fn ind_scale(s: u32) -> &'static str {
        [
            "1.0",
            "0.5",
            "0.25",
            "0.125",
            "0.0625",
            "0.03125",
            "0.015625",
            "0.0078125",
            "0.00390625",
        ]
        .get(s as usize)
        .copied()
        .unwrap_or("1.0")
    }
    fn ind_tex_stages(&self) -> String {
        let mut s = String::new();
        for (i, st) in self.m.ind_tex_stages.iter().enumerate() {
            if st.tex_coord_id as usize >= self.m.tex_gens.len() {
                // Keep the name defined for stages that reference it.
                let _ = writeln!(s, "    let t_IndTexCoord{i} = vec3f(0.0);");
                continue;
            }
            let mut coord = self.read_tex_coord(st.tex_coord_id);
            if st.scale_s != ind_tex_scale::_1 || st.scale_t != ind_tex_scale::_1 {
                coord = format!(
                    "({coord} * vec2f({}, {}))",
                    Self::ind_scale(st.scale_s),
                    Self::ind_scale(st.scale_t)
                );
            }
            let _ = writeln!(
                s,
                "    let t_IndTexCoord{i} = 255.0 * {}.abg;",
                self.texture_sample(st.texture, &coord)
            );
        }
        s
    }

    fn simple_coords(st: &TevStage) -> bool {
        st.ind_tex_matrix == ind_tex_mtx_id::OFF && !st.ind_tex_add_prev
    }

    fn ind_coord_base(&self, st: &TevStage) -> String {
        if (st.ind_tex_stage as usize) < self.m.ind_tex_stages.len() {
            format!("t_IndTexCoord{}", st.ind_tex_stage)
        } else {
            "vec3f(0.0)".into()
        }
    }

    fn alpha_bump_sel(&self, st: &TevStage) -> String {
        let base = self.ind_coord_base(st);
        let ch = match st.ind_tex_alpha_sel {
            ind_tex_alpha_sel::S => "x",
            ind_tex_alpha_sel::T => "y",
            ind_tex_alpha_sel::U => "z",
            _ => return "0.0".into(),
        };
        let mask = match st.ind_tex_format {
            ind_tex_format::_5 => "0xE0",
            ind_tex_format::_4 => "0xF0",
            _ => "0xF8",
        };
        format!("TevMask({base}.{ch}, {mask})")
    }

    fn ras(&self, st: &TevStage) -> String {
        match st.channel_id {
            ras_color_channel_id::COLOR0A0 => "i.v_Color0".into(),
            ras_color_channel_id::COLOR1A1 => "i.v_Color1".into(),
            ras_color_channel_id::ALPHA_BUMP => format!("vec4f({})", self.alpha_bump_sel(st)),
            ras_color_channel_id::ALPHA_BUMP_N => {
                format!("vec4f({} * (255.0 / 248.0))", self.alpha_bump_sel(st))
            }
            _ => "vec4f(0.0)".into(),
        }
    }

    fn tex_access(&self, st: &TevStage) -> String {
        if st.tex_map == tex_map_id::TEXMAP_NULL || self.hacks.disable_textures {
            return "vec4f(1.0)".into();
        }
        let coord = if Self::simple_coords(st) {
            "t_TexCoord".to_string()
        } else {
            format!("(t_TexCoord / {})", self.texture_scale(st.tex_map))
        };
        self.texture_sample(st.tex_map, &coord)
    }

    fn swz(t: Option<SwapTable>, ch: u32) -> &'static str {
        let ch = t.map_or(ch, |t| t[ch as usize & 3]);
        ["r", "g", "b", "a"][ch as usize & 3]
    }
    fn color_swizzle(t: Option<SwapTable>, alpha: bool) -> String {
        if alpha {
            Self::swz(t, 3).repeat(3)
        } else {
            format!("{}{}{}", Self::swz(t, 0), Self::swz(t, 1), Self::swz(t, 2))
        }
    }

    fn konst_color(sel: u32) -> String {
        if sel <= konst_color_sel::KCSEL_1_8 {
            return format!("vec3f({})", float((8 - sel) as f32 / 8.0));
        }
        let (k, sw) = match sel {
            0x0C..=0x0F => (sel - 0x0C, "rgb"),
            0x10..=0x13 => (sel - 0x10, "rrr"),
            0x14..=0x17 => (sel - 0x14, "ggg"),
            0x18..=0x1B => (sel - 0x18, "bbb"),
            0x1C..=0x1F => (sel - 0x1C, "aaa"),
            _ => return "vec3f(1.0)".into(),
        };
        format!("s_kColor{k}.{sw}")
    }
    fn konst_alpha(sel: u32) -> String {
        if sel <= konst_alpha_sel::KASEL_1_8 {
            return float((8 - sel) as f32 / 8.0);
        }
        let (k, sw) = match sel {
            0x10..=0x13 => (sel - 0x10, "r"),
            0x14..=0x17 => (sel - 0x14, "g"),
            0x18..=0x1B => (sel - 0x18, "b"),
            0x1C..=0x1F => (sel - 0x1C, "a"),
            _ => return "1.0".into(),
        };
        format!("s_kColor{k}.{sw}")
    }

    fn color_in(&self, st: &TevStage, c: u32) -> String {
        match c {
            cc::CPREV => "t_ColorPrev.rgb".into(),
            cc::APREV => "t_ColorPrev.aaa".into(),
            cc::C0 => "t_Color0.rgb".into(),
            cc::A0 => "t_Color0.aaa".into(),
            cc::C1 => "t_Color1.rgb".into(),
            cc::A1 => "t_Color1.aaa".into(),
            cc::C2 => "t_Color2.rgb".into(),
            cc::A2 => "t_Color2.aaa".into(),
            cc::TEXC => format!("t_TexSample.{}", Self::color_swizzle(st.tex_swap, false)),
            cc::TEXA => format!("t_TexSample.{}", Self::color_swizzle(st.tex_swap, true)),
            cc::RASC => format!(
                "saturate({}.{})",
                self.ras(st),
                Self::color_swizzle(st.ras_swap, false)
            ),
            cc::RASA => format!(
                "saturate({}.{})",
                self.ras(st),
                Self::color_swizzle(st.ras_swap, true)
            ),
            cc::ONE => "vec3f(1.0)".into(),
            cc::HALF => "vec3f(0.5)".into(),
            cc::KONST => Self::konst_color(st.konst_color_sel),
            _ => "vec3f(0.0)".into(),
        }
    }
    fn alpha_in(&self, st: &TevStage, a: u32) -> String {
        match a {
            ca::APREV => "t_ColorPrev.a".into(),
            ca::A0 => "t_Color0.a".into(),
            ca::A1 => "t_Color1.a".into(),
            ca::A2 => "t_Color2.a".into(),
            ca::TEXA => format!("t_TexSample.{}", Self::swz(st.tex_swap, 3)),
            ca::RASA => format!("saturate({}.{})", self.ras(st), Self::swz(st.ras_swap, 3)),
            ca::KONST => Self::konst_alpha(st.konst_alpha_sel),
            _ => "0.0".into(),
        }
    }

    fn tev_inputs(&self, st: &TevStage) -> String {
        let v = |k: usize| {
            format!(
                "vec4f({}, {})",
                self.color_in(st, st.color_in[k]),
                self.alpha_in(st, st.alpha_in[k])
            )
        };
        format!(
            "    t_TevA = TevOverflow4({});\n    t_TevB = TevOverflow4({});\n    t_TevC = TevOverflow4({});\n    t_TevD = {};\n",
            v(0),
            v(1),
            v(2),
            v(3)
        )
    }

    fn register(r: u32) -> &'static str {
        match r {
            register::REG0 => "t_Color0",
            register::REG1 => "t_Color1",
            register::REG2 => "t_Color2",
            _ => "t_ColorPrev",
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn tev_op(
        op: u32,
        bias: u32,
        scale: u32,
        a: &str,
        b: &str,
        c: &str,
        d: &str,
        zero: &str,
        alpha: bool,
    ) -> String {
        let n = if alpha { "1" } else { "3" };
        match op {
            tev_op::ADD | tev_op::SUB => {
                let neg = if op == tev_op::SUB { "-" } else { "" };
                let mut v = format!("({neg}mix({a}, {b}, {c}) + {d})");
                match bias {
                    tev_bias::ADDHALF => v = format!("({v} + 0.5)"),
                    tev_bias::SUBHALF => v = format!("({v} - 0.5)"),
                    _ => {}
                }
                match scale {
                    tev_scale::SCALE_2 => format!("({v} * 2.0)"),
                    tev_scale::SCALE_4 => format!("({v} * 4.0)"),
                    tev_scale::DIVIDE_2 => format!("({v} * 0.5)"),
                    _ => v,
                }
            }
            tev_op::COMP_R8_GT => format!("(select({zero}, {c}, t_TevA.r > t_TevB.r) + {d})"),
            tev_op::COMP_R8_EQ => format!("(select({zero}, {c}, t_TevA.r == t_TevB.r) + {d})"),
            tev_op::COMP_GR16_GT => {
                format!("(select({zero}, {c}, TevPack16(t_TevA.rg) > TevPack16(t_TevB.rg)) + {d})")
            }
            tev_op::COMP_GR16_EQ => {
                format!("(select({zero}, {c}, TevPack16(t_TevA.rg) == TevPack16(t_TevB.rg)) + {d})")
            }
            tev_op::COMP_BGR24_GT => format!(
                "(select({zero}, {c}, TevPack24(t_TevA.rgb) > TevPack24(t_TevB.rgb)) + {d})"
            ),
            tev_op::COMP_BGR24_EQ => format!(
                "(select({zero}, {c}, TevPack24(t_TevA.rgb) == TevPack24(t_TevB.rgb)) + {d})"
            ),
            tev_op::COMP_RGB8_GT => format!("((TevPerCompGT{n}({a}, {b}) * {c}) + {d})"),
            tev_op::COMP_RGB8_EQ => format!("((TevPerCompEQ{n}({a}, {b}) * {c}) + {d})"),
            _ => d.to_string(),
        }
    }

    fn color_op(&self, st: &TevStage) -> String {
        let e = Self::tev_op(
            st.color_op,
            st.color_bias,
            st.color_scale,
            "t_TevA.rgb",
            "t_TevB.rgb",
            "t_TevC.rgb",
            "t_TevD.rgb",
            "vec3f(0.0)",
            false,
        );
        let v = if st.color_clamp {
            format!("saturate({e})")
        } else {
            format!("clamp({e}, vec3f(-4.0), vec3f(4.0))")
        };
        let r = Self::register(st.color_reg);
        format!("    {r} = vec4f({v}, {r}.a);\n")
    }
    fn alpha_op(&self, st: &TevStage) -> String {
        let e = Self::tev_op(
            st.alpha_op,
            st.alpha_bias,
            st.alpha_scale,
            "t_TevA.a",
            "t_TevB.a",
            "t_TevC.a",
            "t_TevD.a",
            "0.0",
            true,
        );
        let v = if st.alpha_clamp {
            format!("saturate({e})")
        } else {
            format!("clamp({e}, -4.0, 4.0)")
        };
        format!("    {}.a = {v};\n", Self::register(st.alpha_reg))
    }

    fn wrap_n(coord: &str, wrap: u32) -> String {
        match wrap {
            ind_tex_wrap::_0 => "0.0".into(),
            ind_tex_wrap::_256 => format!("GxMod({coord}, 256.0)"),
            ind_tex_wrap::_128 => format!("GxMod({coord}, 128.0)"),
            ind_tex_wrap::_64 => format!("GxMod({coord}, 64.0)"),
            ind_tex_wrap::_32 => format!("GxMod({coord}, 32.0)"),
            ind_tex_wrap::_16 => format!("GxMod({coord}, 16.0)"),
            _ => coord.into(),
        }
    }

    fn tev_tex_coord_wrap(&self, st: &TevStage) -> Option<String> {
        if st.tex_coord_id == tex_coord_id::TEXCOORD_NULL || st.tex_map == tex_map_id::TEXMAP_NULL {
            return None;
        }
        if self.m.tex_gens.is_empty() {
            return Some("vec2f(0.0)".into());
        }
        let id = st.tex_coord_id.min(self.m.tex_gens.len() as u32 - 1);
        let mut base = self.read_tex_coord(id);
        if !Self::simple_coords(st) {
            base = format!("({base} * {})", self.texture_scale(st.tex_map));
        }
        if st.ind_tex_wrap_s == ind_tex_wrap::OFF && st.ind_tex_wrap_t == ind_tex_wrap::OFF {
            Some(base)
        } else {
            Some(format!(
                "vec2f({}, {})",
                Self::wrap_n(&format!("{base}.x"), st.ind_tex_wrap_s),
                Self::wrap_n(&format!("{base}.y"), st.ind_tex_wrap_t)
            ))
        }
    }

    fn ind_coord_bias(st: &TevStage) -> String {
        let b = if st.ind_tex_format == ind_tex_format::_8 {
            "-128.0"
        } else {
            "1.0"
        };
        match st.ind_tex_bias_sel {
            ind_tex_bias_sel::S => format!(" + vec3f({b}, 0.0, 0.0)"),
            ind_tex_bias_sel::T => format!(" + vec3f(0.0, {b}, 0.0)"),
            ind_tex_bias_sel::ST => format!(" + vec3f({b}, {b}, 0.0)"),
            ind_tex_bias_sel::U => format!(" + vec3f(0.0, 0.0, {b})"),
            ind_tex_bias_sel::SU => format!(" + vec3f({b}, 0.0, {b})"),
            ind_tex_bias_sel::TU => format!(" + vec3f(0.0, {b}, {b})"),
            ind_tex_bias_sel::STU => format!(" + vec3f({b})"),
            _ => String::new(),
        }
    }

    fn tev_indirect_mtx(&self, st: &TevStage) -> String {
        // Formats other than 8-bit read the same coordinates (noclip throws).
        let ind = format!("({}{})", self.ind_coord_base(st), Self::ind_coord_bias(st));
        let idx = (st.ind_tex_matrix & 3).saturating_sub(1);
        match st.ind_tex_matrix {
            ind_tex_mtx_id::_0 | ind_tex_mtx_id::_1 | ind_tex_mtx_id::_2 => {
                format!("Mul24(u_Material.IndTexMtx[{idx}], vec4f({ind}, 0.0))")
            }
            ind_tex_mtx_id::S0 | ind_tex_mtx_id::S1 | ind_tex_mtx_id::S2 => format!(
                "(u_Material.IndTexMtx[{idx}].mx.w * {} * {ind}.xx)",
                self.read_tex_coord(st.tex_coord_id)
            ),
            ind_tex_mtx_id::T0 | ind_tex_mtx_id::T1 | ind_tex_mtx_id::T2 => format!(
                "(u_Material.IndTexMtx[{idx}].mx.w * {} * {ind}.yy)",
                self.read_tex_coord(st.tex_coord_id)
            ),
            _ => "vec2f(0.0)".into(),
        }
    }

    fn tev_tex_coord(&self, st: &TevStage) -> String {
        let base = self.tev_tex_coord_wrap(st);
        let ind = if st.ind_tex_matrix != ind_tex_mtx_id::OFF
            && (st.ind_tex_stage as usize) < self.m.ind_tex_stages.len()
        {
            Some(self.tev_indirect_mtx(st))
        } else {
            None
        };
        let fin = match (base, ind) {
            (Some(b), Some(i)) => format!("{b} + {i}"),
            (Some(b), None) => b,
            (None, Some(i)) => i,
            (None, None) => return String::new(),
        };
        if st.ind_tex_add_prev {
            format!("    t_TexCoord += {fin};\n")
        } else {
            format!("    t_TexCoord = {fin};\n")
        }
    }

    fn tev_stages(&self) -> String {
        let mut s = String::new();
        for (i, st) in self.m.tev_stages.iter().enumerate() {
            let _ = writeln!(
                s,
                "    // TEV stage {i}: color in {:?} op {} alpha in {:?} op {} texcoord {} texmap {} channel {}",
                st.color_in, st.color_op, st.alpha_in, st.alpha_op, st.tex_coord_id, st.tex_map, st.channel_id
            );
            s += &self.tev_tex_coord(st);
            let _ = writeln!(s, "    t_TexSample = {};", self.tex_access(st));
            s += &self.tev_inputs(st);
            s += &self.color_op(st);
            s += &self.alpha_op(st);
        }
        s
    }

    fn tev_output(&self) -> String {
        let last = self.m.tev_stages.last();
        let c = Self::register(last.map_or(register::PREV, |s| s.color_reg));
        let a = Self::register(last.map_or(register::PREV, |s| s.alpha_reg));
        if c == a {
            format!("    let t_TevOutput = {c};\n")
        } else {
            format!("    let t_TevOutput = vec4f({c}.rgb, {a}.a);\n")
        }
    }

    fn alpha_compare(cmp: u32, r: &str) -> String {
        match cmp {
            compare_type::NEVER => "false".into(),
            compare_type::LESS => format!("(t_PixelOut.a < {r})"),
            compare_type::EQUAL => format!("(t_PixelOut.a == {r})"),
            compare_type::LEQUAL => format!("(t_PixelOut.a <= {r})"),
            compare_type::GREATER => format!("(t_PixelOut.a > {r})"),
            compare_type::NEQUAL => format!("(t_PixelOut.a != {r})"),
            compare_type::GEQUAL => format!("(t_PixelOut.a >= {r})"),
            _ => "true".into(),
        }
    }

    fn alpha_test(&self) -> String {
        let at = &self.m.alpha_test;
        if at.op == alpha_op::OR
            && (at.compare_a == compare_type::ALWAYS || at.compare_b == compare_type::ALWAYS)
        {
            return String::new();
        }
        if at.op == alpha_op::AND
            && at.compare_a == compare_type::ALWAYS
            && at.compare_b == compare_type::ALWAYS
        {
            return String::new();
        }
        let (ra, rb) = if self.m.has_dynamic_alpha_test {
            (
                "u_Material.DynamicAlphaParams.x".to_string(),
                "u_Material.DynamicAlphaParams.y".to_string(),
            )
        } else {
            (
                float(f32::from(at.reference_a) / 255.0),
                float(f32::from(at.reference_b) / 255.0),
            )
        };
        let a = Self::alpha_compare(at.compare_a, &ra);
        let b = Self::alpha_compare(at.compare_b, &rb);
        let op = match at.op {
            alpha_op::AND => "(t_AlphaTestA && t_AlphaTestB)",
            alpha_op::OR => "(t_AlphaTestA || t_AlphaTestB)",
            alpha_op::XOR => "(t_AlphaTestA != t_AlphaTestB)",
            _ => "(t_AlphaTestA == t_AlphaTestB)",
        };
        format!("    let t_AlphaTestA = {a};\n    let t_AlphaTestB = {b};\n    if (!{op}) {{ discard; }}\n")
    }

    fn fog(&self) -> String {
        let ty = self.m.rop.fog_type;
        if ty == fog_type::NONE {
            return String::new();
        }
        let base = "t_FogZ";
        let f = match ty & 7 {
            4 => format!("1.0 - exp2(-8.0 * {base})"),
            5 => format!("1.0 - exp2(-8.0 * {base} * {base})"),
            6 => format!("1.0 - exp2(-8.0 * (1.0 - {base}))"),
            7 => format!("1.0 - exp2(-8.0 * (1.0 - {base}) * (1.0 - {base}))"),
            _ => base.to_string(),
        };
        format!(
            "    let t_Z = i.pos.z;\n    let t_FogBase = select(u_Material.Fog.Param.x * t_Z, u_Material.Fog.Param.x / (u_Material.Fog.Param.y - t_Z), u_Material.Fog.Param.y != 0.0);\n    let t_FogZ = saturate(t_FogBase - u_Material.Fog.Param.z);\n    let t_Fog = saturate({f});\n    t_PixelOut = vec4f(mix(t_PixelOut.rgb, u_Material.Fog.Color.rgb, t_Fog), t_PixelOut.a);\n"
        )
    }

    fn dst_alpha(&self) -> String {
        match self.m.rop.dst_alpha {
            Some(v) => format!("    t_PixelOut.a = {};\n", float(f32::from(v) / 255.0)),
            None => String::new(),
        }
    }

    fn generate(&self) -> String {
        let mut s = String::new();
        s += BINDINGS;
        s += VERTEX_LIB;
        s += FRAGMENT_LIB;

        // Vertex input and output structs.
        s += "\nstruct VertexInput {\n";
        let mut used = Vec::new();
        for a in VertexInput::ALL {
            if self.uses_attrib(a) && (self.has)(a) {
                let ty = if a.components() == 3 {
                    "vec3f"
                } else {
                    "vec4f"
                };
                let _ = writeln!(s, "    @location({}) {}: {ty},", a.location(), a.name());
                used.push(a);
            }
        }
        if used.is_empty() {
            s += "    @builtin(vertex_index) unused_vertex_index: u32,\n";
        }
        s += "};\n";
        s += "struct VertexOutput {\n    @builtin(position) pos: vec4f,\n    @location(0) v_Color0: vec4f,\n    @location(1) v_Color1: vec4f,\n";
        for i in 0..self.m.tex_gens.len() {
            let _ = writeln!(s, "    @location({}) v_TexCoord{i}: vec3f,", i + 2);
        }
        s += "};\n";

        // Vertex shader.
        s += "\n@vertex\nfn vs(input: VertexInput) -> VertexOutput {\n    var o: VertexOutput;\n";
        for a in VertexInput::ALL {
            let ty = if a.components() == 3 {
                "vec3f"
            } else {
                "vec4f"
            };
            if used.contains(&a) {
                let _ = writeln!(s, "    let a_{} = input.{};", a.name(), a.name());
            } else if self.uses_attrib(a) {
                // Missing from the vertex data: WebGL's default attribute.
                let v = if a.components() == 3 {
                    "vec3f(0.0)"
                } else {
                    "vec4f(0.0, 0.0, 0.0, 1.0)"
                };
                let _ = writeln!(s, "    let a_{}: {ty} = {v};", a.name());
            }
        }
        let _ = writeln!(
            s,
            "    let t_Position = {};",
            self.mul("a_Position", true, false)
        );
        if self.uses_normal() {
            let _ = writeln!(
                s,
                "    let t_Normal = {};",
                self.mul("a_Normal", false, true)
            );
        } else {
            s += "    let t_Normal = vec3f(0.0);\n";
        }
        s += "    var v_Color0 = vec4f(1.0);\n    var v_Color1 = vec4f(1.0);\n";
        s += &self.light_channels();
        s += &self.tex_gens();
        s += "    o.v_Color0 = v_Color0;\n    o.v_Color1 = v_Color1;\n";
        s += "    o.pos = u_Scene.Projection * vec4f(t_Position, 1.0);\n    return o;\n}\n";

        // Fragment shader.
        s += "\n@fragment\nfn fs(i: VertexOutput) -> @location(0) vec4f {\n";
        s += "    let s_kColor0 = u_Material.KonstColor[0];\n    let s_kColor1 = u_Material.KonstColor[1];\n";
        s += "    let s_kColor2 = u_Material.KonstColor[2];\n    let s_kColor3 = u_Material.KonstColor[3];\n";
        s +=
            "    var t_ColorPrev = u_Material.Color[0];\n    var t_Color0 = u_Material.Color[1];\n";
        s += "    var t_Color1 = u_Material.Color[2];\n    var t_Color2 = u_Material.Color[3];\n";
        s += &self.ind_tex_stages();
        s += "    var t_TexCoord = vec2f(0.0);\n    var t_TexSample = vec4f(0.0);\n";
        s += "    var t_TevA: vec4f;\n    var t_TevB: vec4f;\n    var t_TevC: vec4f;\n    var t_TevD: vec4f;\n";
        s += &self.tev_stages();
        s += &self.tev_output();
        s += "    var t_PixelOut = TevOverflow4(t_TevOutput);\n";
        s += &self.alpha_test();
        s += &self.fog();
        s += &self.dst_alpha();
        s += "    return t_PixelOut;\n}\n";
        s
    }
}

/// Generate the WGSL for `m`. `has_input` says which inputs the vertex
/// buffer layout provides; used inputs it lacks become constants.
pub fn generate(
    m: &GxMaterial,
    hacks: MaterialHacks,
    has_input: &dyn Fn(VertexInput) -> bool,
) -> GxProgram {
    GxProgram {
        source: Gen {
            m,
            hacks,
            has: has_input,
        }
        .generate(),
    }
}
