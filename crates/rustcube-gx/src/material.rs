// Derived from noclip.website (https://github.com/magcius/noclip.website),
// src/gx/gx_material.ts, src/gx/GXMaterialBuilder.ts and src/gx/gx_render.ts
// at commit 2cebac2a5d1d7c44535397a14f51e764ca823767.
// Copyright (c) 2018 Jasper St. Pierre. MIT License: see THIRD-PARTY.md.
// Translated from TypeScript to Rust for rustcube-gx.
//! The GX material: every piece of GX state that changes the shader or the
//! pipeline (light channels, texgens, TEV and indirect stages, alpha test,
//! fog, blend, z mode, cull), a builder in GX call order, decoding from
//! BP/XF registers, and the light and fog parameter helpers.
use crate::enums::*;
use crate::vtx::DisplayListRegisters;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColorChannelControl {
    pub lighting_enabled: bool,
    pub mat_color_source: u32,
    pub amb_color_source: u32,
    pub lit_mask: u32,
    pub diffuse_function: u32,
    pub attenuation_function: u32,
}

impl Default for ColorChannelControl {
    fn default() -> Self {
        Self {
            lighting_enabled: false,
            mat_color_source: color_src::REG,
            amb_color_source: color_src::REG,
            lit_mask: 0,
            diffuse_function: diffuse_function::NONE,
            attenuation_function: attenuation_function::NONE,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct LightChannelControl {
    pub color: ColorChannelControl,
    pub alpha: ColorChannelControl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TexGen {
    pub ty: u32,
    pub source: u32,
    pub matrix: u32,
    pub normalize: bool,
    pub post_matrix: u32,
}

impl Default for TexGen {
    fn default() -> Self {
        Self {
            ty: tex_gen_type::MTX2x4,
            source: tex_gen_src::TEX0,
            matrix: tex_gen_matrix::IDENTITY,
            normalize: false,
            post_matrix: post_tex_gen_matrix::PTIDENTITY,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct IndTexStage {
    pub tex_coord_id: u32,
    pub texture: u32,
    pub scale_s: u32,
    pub scale_t: u32,
}

/// Channel selection for each of R, G, B, A.
pub type SwapTable = [u32; 4];

pub const DEFAULT_SWAP_TABLES: [SwapTable; 4] =
    [[0, 1, 2, 3], [0, 0, 0, 3], [1, 1, 1, 3], [2, 2, 2, 3]];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TevStage {
    pub color_in: [u32; 4],
    pub color_op: u32,
    pub color_bias: u32,
    pub color_scale: u32,
    pub color_clamp: bool,
    pub color_reg: u32,
    pub alpha_in: [u32; 4],
    pub alpha_op: u32,
    pub alpha_bias: u32,
    pub alpha_scale: u32,
    pub alpha_clamp: bool,
    pub alpha_reg: u32,
    pub tex_coord_id: u32,
    pub tex_map: u32,
    pub channel_id: u32,
    pub konst_color_sel: u32,
    pub konst_alpha_sel: u32,
    pub ras_swap: Option<SwapTable>,
    pub tex_swap: Option<SwapTable>,
    pub ind_tex_stage: u32,
    pub ind_tex_format: u32,
    pub ind_tex_bias_sel: u32,
    pub ind_tex_alpha_sel: u32,
    pub ind_tex_matrix: u32,
    pub ind_tex_wrap_s: u32,
    pub ind_tex_wrap_t: u32,
    pub ind_tex_add_prev: bool,
    pub ind_tex_use_orig_lod: bool,
}

impl TevStage {
    /// GXMaterialBuilder's defaults for stage `idx`.
    pub fn new(idx: u32) -> Self {
        let (tc, tm) = if idx <= 7 {
            (idx, idx)
        } else {
            (tex_coord_id::TEXCOORD_NULL, tex_map_id::TEXMAP_NULL)
        };
        Self {
            color_in: [cc::ZERO, cc::ZERO, cc::ZERO, cc::TEXC],
            color_op: tev_op::ADD,
            color_bias: tev_bias::ZERO,
            color_scale: tev_scale::SCALE_1,
            color_clamp: true,
            color_reg: register::PREV,
            alpha_in: [ca::ZERO, ca::ZERO, ca::ZERO, ca::TEXA],
            alpha_op: tev_op::ADD,
            alpha_bias: tev_bias::ZERO,
            alpha_scale: tev_scale::SCALE_1,
            alpha_clamp: true,
            alpha_reg: register::PREV,
            tex_coord_id: tc,
            tex_map: tm,
            channel_id: ras_color_channel_id::COLOR0A0,
            konst_color_sel: konst_color_sel::KCSEL_2_8,
            konst_alpha_sel: konst_alpha_sel::KASEL_1,
            ras_swap: None,
            tex_swap: None,
            ind_tex_stage: 0,
            ind_tex_format: ind_tex_format::_8,
            ind_tex_bias_sel: ind_tex_bias_sel::NONE,
            ind_tex_alpha_sel: ind_tex_alpha_sel::OFF,
            ind_tex_matrix: ind_tex_mtx_id::OFF,
            ind_tex_wrap_s: ind_tex_wrap::OFF,
            ind_tex_wrap_t: ind_tex_wrap::OFF,
            ind_tex_add_prev: false,
            ind_tex_use_orig_lod: false,
        }
    }
}

/// Alpha compare; references are 0..255 as GX takes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AlphaTest {
    pub op: u32,
    pub compare_a: u32,
    pub reference_a: u8,
    pub compare_b: u32,
    pub reference_b: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RopInfo {
    pub fog_type: u32,
    pub fog_adj_enabled: bool,
    pub depth_test: bool,
    pub depth_func: u32,
    pub depth_write: bool,
    pub blend_mode: u32,
    pub blend_src: u32,
    pub blend_dst: u32,
    pub blend_logic_op: u32,
    /// `GXSetDstAlpha`: constant alpha written to the target, 0..255.
    pub dst_alpha: Option<u8>,
    pub color_update: bool,
    pub alpha_update: bool,
}

/// One GX material. Hashable: the pipeline cache keys on it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GxMaterial {
    pub cull_mode: u32,
    pub light_channels: Vec<LightChannelControl>,
    pub tex_gens: Vec<TexGen>,
    pub tev_stages: Vec<TevStage>,
    pub ind_tex_stages: Vec<IndTexStage>,
    pub alpha_test: AlphaTest,
    pub rop: RopInfo,
    /// Vertices pick their position matrix (PNMTXIDX); else matrix 0.
    pub use_pn_mtx_idx: bool,
    /// Per texgen: vertices pick their texture matrix (TEXnMTXIDX).
    pub use_tex_mtx_idx: [bool; 8],
    /// Alpha references come from the material parameters, not the shader.
    pub has_dynamic_alpha_test: bool,
}

impl GxMaterial {
    pub fn has_post_tex_mtx_block(&self) -> bool {
        self.tex_gens
            .iter()
            .any(|t| t.post_matrix != post_tex_gen_matrix::PTIDENTITY)
    }
    pub fn has_lights_block(&self) -> bool {
        self.light_channels
            .iter()
            .take(2)
            .any(|c| uses_lights(&c.color) || uses_lights(&c.alpha))
    }
    pub fn has_fog_block(&self) -> bool {
        self.rop.fog_type != fog_type::NONE
    }
    /// CULL_ALL draws nothing.
    pub fn is_valid(&self) -> bool {
        self.cull_mode != cull_mode::ALL && !self.tev_stages.is_empty()
    }
}

fn uses_lights(c: &ColorChannelControl) -> bool {
    c.lighting_enabled && c.lit_mask != 0
}

/// `GXColorChannelID` to the TEV's rasterized color selector.
pub fn ras_color_channel_id(v: u32) -> u32 {
    match v {
        color_channel_id::COLOR0 | color_channel_id::ALPHA0 | color_channel_id::COLOR0A0 => {
            ras_color_channel_id::COLOR0A0
        }
        color_channel_id::COLOR1 | color_channel_id::ALPHA1 | color_channel_id::COLOR1A1 => {
            ras_color_channel_id::COLOR1A1
        }
        color_channel_id::ALPHA_BUMP => ras_color_channel_id::ALPHA_BUMP,
        color_channel_id::ALPHA_BUMP_N => ras_color_channel_id::ALPHA_BUMP_N,
        _ => ras_color_channel_id::COLOR_ZERO,
    }
}

/// Builds a [`GxMaterial`] with GX's call shapes (GXMaterialBuilder).
#[derive(Debug, Clone, PartialEq)]
pub struct GxMaterialBuilder {
    cull_mode: u32,
    light_channels: Vec<LightChannelControl>,
    tex_gens: Vec<TexGen>,
    tev_stages: Vec<TevStage>,
    ind_tex_stages: Vec<IndTexStage>,
    alpha_test: AlphaTest,
    rop: RopInfo,
    use_pn_mtx_idx: bool,
    use_tex_mtx_idx: [bool; 8],
    has_dynamic_alpha_test: bool,
}

impl Default for GxMaterialBuilder {
    fn default() -> Self {
        Self::new()
    }
}

fn ensure<T: Clone>(v: &mut Vec<T>, idx: usize, make: impl Fn(usize) -> T) -> &mut T {
    while v.len() <= idx {
        let i = v.len();
        v.push(make(i));
    }
    &mut v[idx]
}

impl GxMaterialBuilder {
    pub fn new() -> Self {
        Self {
            cull_mode: cull_mode::NONE,
            light_channels: Vec::new(),
            tex_gens: Vec::new(),
            tev_stages: Vec::new(),
            ind_tex_stages: Vec::new(),
            alpha_test: AlphaTest {
                op: alpha_op::AND,
                compare_a: compare_type::ALWAYS,
                reference_a: 0,
                compare_b: compare_type::ALWAYS,
                reference_b: 0,
            },
            rop: RopInfo {
                fog_type: fog_type::NONE,
                fog_adj_enabled: false,
                depth_test: true,
                depth_func: compare_type::LEQUAL,
                depth_write: true,
                blend_mode: blend_mode::NONE,
                blend_src: blend_factor::SRCALPHA,
                blend_dst: blend_factor::INVSRCALPHA,
                blend_logic_op: logic_op::CLEAR,
                dst_alpha: None,
                color_update: true,
                alpha_update: false,
            },
            use_pn_mtx_idx: true,
            use_tex_mtx_idx: [false; 8],
            has_dynamic_alpha_test: false,
        }
    }

    pub fn set_cull_mode(&mut self, m: u32) {
        self.cull_mode = m;
    }

    /// Keep only the first `n` texgens / TEV stages / indirect stages /
    /// channels (GXSetNumTexGens etc.).
    pub fn set_num_tex_gens(&mut self, n: usize) {
        self.tex_gens.truncate(n);
        ensure(&mut self.tex_gens, n.saturating_sub(1), |_| {
            TexGen::default()
        });
        self.tex_gens.truncate(n);
    }
    pub fn set_num_tev_stages(&mut self, n: usize) {
        if n > 0 {
            self.tev_stage(n as u32 - 1);
        }
        self.tev_stages.truncate(n);
    }
    pub fn set_num_ind_stages(&mut self, n: usize) {
        if n > 0 {
            self.ind_stage(n as u32 - 1);
        }
        self.ind_tex_stages.truncate(n);
    }
    pub fn set_num_chans(&mut self, n: usize) {
        if n > 0 {
            ensure(&mut self.light_channels, n - 1, |_| {
                LightChannelControl::default()
            });
        }
        self.light_channels.truncate(n);
    }

    pub fn set_tex_coord_gen(
        &mut self,
        idx: u32,
        ty: u32,
        source: u32,
        matrix: u32,
        normalize: bool,
        post_matrix: u32,
    ) {
        *ensure(&mut self.tex_gens, idx as usize, |_| TexGen::default()) = TexGen {
            ty,
            source,
            matrix,
            normalize,
            post_matrix,
        };
    }

    /// The matrix texgen `i` multiplies by (from the matrix index registers;
    /// the texgen registers don't hold it).
    pub fn set_tex_gen_matrix(&mut self, i: usize, matrix: u32) {
        if let Some(g) = self.tex_gens.get_mut(i) {
            g.matrix = matrix;
        }
    }

    /// `GXSetChanCtrl`: `idx` is a GXChannelID (COLOR0, ALPHA0, COLOR0A0...).
    #[allow(clippy::too_many_arguments)]
    pub fn set_chan_ctrl(
        &mut self,
        idx: u32,
        enable: bool,
        amb_src: u32,
        mat_src: u32,
        light_mask: u32,
        diff_fn: u32,
        attn_fn: u32,
    ) {
        let c = ColorChannelControl {
            lighting_enabled: enable,
            amb_color_source: amb_src,
            mat_color_source: mat_src,
            lit_mask: light_mask,
            diffuse_function: diff_fn,
            attenuation_function: attn_fn,
        };
        let ch = ensure(&mut self.light_channels, (idx & 1) as usize, |_| {
            LightChannelControl::default()
        });
        let set = (idx >> 1) + 1;
        if set & 1 != 0 {
            ch.color = c;
        }
        if set & 2 != 0 {
            ch.alpha = c;
        }
    }

    fn tev_stage(&mut self, idx: u32) -> &mut TevStage {
        ensure(&mut self.tev_stages, idx as usize, |i| {
            TevStage::new(i as u32)
        })
    }
    fn ind_stage(&mut self, idx: u32) -> &mut IndTexStage {
        ensure(&mut self.ind_tex_stages, idx as usize, |_| IndTexStage {
            scale_s: ind_tex_scale::_1,
            scale_t: ind_tex_scale::_1,
            ..Default::default()
        })
    }

    pub fn set_tev_order(&mut self, idx: u32, tex_coord: u32, tex_map: u32, channel: u32) {
        let s = self.tev_stage(idx);
        s.tex_coord_id = tex_coord;
        s.tex_map = tex_map;
        s.channel_id = channel;
    }
    pub fn set_tev_color_in(&mut self, idx: u32, a: u32, b: u32, c: u32, d: u32) {
        self.tev_stage(idx).color_in = [a, b, c, d];
    }
    pub fn set_tev_alpha_in(&mut self, idx: u32, a: u32, b: u32, c: u32, d: u32) {
        self.tev_stage(idx).alpha_in = [a, b, c, d];
    }
    pub fn set_tev_color_op(
        &mut self,
        idx: u32,
        op: u32,
        bias: u32,
        scale: u32,
        clamp: bool,
        reg: u32,
    ) {
        let s = self.tev_stage(idx);
        (
            s.color_op,
            s.color_bias,
            s.color_scale,
            s.color_clamp,
            s.color_reg,
        ) = (op, bias, scale, clamp, reg);
    }
    pub fn set_tev_alpha_op(
        &mut self,
        idx: u32,
        op: u32,
        bias: u32,
        scale: u32,
        clamp: bool,
        reg: u32,
    ) {
        let s = self.tev_stage(idx);
        (
            s.alpha_op,
            s.alpha_bias,
            s.alpha_scale,
            s.alpha_clamp,
            s.alpha_reg,
        ) = (op, bias, scale, clamp, reg);
    }
    pub fn set_tev_k_color_sel(&mut self, idx: u32, sel: u32) {
        self.tev_stage(idx).konst_color_sel = sel;
    }
    pub fn set_tev_k_alpha_sel(&mut self, idx: u32, sel: u32) {
        self.tev_stage(idx).konst_alpha_sel = sel;
    }
    pub fn set_tev_swap_mode(&mut self, idx: u32, ras: Option<SwapTable>, tex: Option<SwapTable>) {
        let s = self.tev_stage(idx);
        s.ras_swap = ras;
        s.tex_swap = tex;
    }
    #[allow(clippy::too_many_arguments)]
    pub fn set_tev_indirect(
        &mut self,
        idx: u32,
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
        let s = self.tev_stage(idx);
        s.ind_tex_stage = ind_stage;
        s.ind_tex_format = format;
        s.ind_tex_bias_sel = bias_sel;
        s.ind_tex_alpha_sel = alpha_sel;
        s.ind_tex_matrix = matrix;
        s.ind_tex_wrap_s = wrap_s;
        s.ind_tex_wrap_t = wrap_t;
        s.ind_tex_add_prev = add_prev;
        s.ind_tex_use_orig_lod = utc_lod;
    }
    pub fn set_tev_ind_warp(
        &mut self,
        idx: u32,
        ind_stage: u32,
        signed_offsets: bool,
        replace: bool,
        matrix: u32,
    ) {
        let bias = if signed_offsets {
            ind_tex_bias_sel::ST
        } else {
            ind_tex_bias_sel::NONE
        };
        let wrap = if replace {
            ind_tex_wrap::_0
        } else {
            ind_tex_wrap::OFF
        };
        self.set_tev_indirect(
            idx,
            ind_stage,
            ind_tex_format::_8,
            bias,
            matrix,
            wrap,
            wrap,
            false,
            false,
            ind_tex_alpha_sel::OFF,
        );
    }
    pub fn set_tev_direct(&mut self, idx: u32) {
        self.set_tev_indirect(
            idx,
            0,
            ind_tex_format::_8,
            ind_tex_bias_sel::NONE,
            ind_tex_mtx_id::OFF,
            ind_tex_wrap::OFF,
            ind_tex_wrap::OFF,
            false,
            false,
            ind_tex_alpha_sel::OFF,
        );
    }
    pub fn set_ind_tex_order(&mut self, idx: u32, tex_coord: u32, tex_map: u32) {
        let s = self.ind_stage(idx);
        s.tex_coord_id = tex_coord;
        s.texture = tex_map;
    }
    pub fn set_ind_tex_scale(&mut self, idx: u32, s: u32, t: u32) {
        let st = self.ind_stage(idx);
        st.scale_s = s;
        st.scale_t = t;
    }
    pub fn set_alpha_compare(
        &mut self,
        compare_a: u32,
        ref_a: u8,
        op: u32,
        compare_b: u32,
        ref_b: u8,
    ) {
        self.alpha_test = AlphaTest {
            op,
            compare_a,
            reference_a: ref_a,
            compare_b,
            reference_b: ref_b,
        };
    }
    pub fn set_fog(&mut self, ty: u32, adj: bool) {
        self.rop.fog_type = ty;
        self.rop.fog_adj_enabled = adj;
    }
    pub fn set_blend_mode(&mut self, mode: u32, src: u32, dst: u32, logic: u32) {
        (
            self.rop.blend_mode,
            self.rop.blend_src,
            self.rop.blend_dst,
            self.rop.blend_logic_op,
        ) = (mode, src, dst, logic);
    }
    pub fn set_z_mode(&mut self, test: bool, func: u32, write: bool) {
        (
            self.rop.depth_test,
            self.rop.depth_func,
            self.rop.depth_write,
        ) = (test, func, write);
    }
    pub fn set_dst_alpha(&mut self, v: Option<u8>) {
        self.rop.dst_alpha = v;
    }
    pub fn set_color_update(&mut self, v: bool) {
        self.rop.color_update = v;
    }
    pub fn set_alpha_update(&mut self, v: bool) {
        self.rop.alpha_update = v;
    }
    pub fn set_use_pn_mtx_idx(&mut self, v: bool) {
        self.use_pn_mtx_idx = v;
    }
    pub fn set_use_tex_mtx_idx(&mut self, i: usize, v: bool) {
        if let Some(s) = self.use_tex_mtx_idx.get_mut(i) {
            *s = v;
        }
    }
    pub fn set_dynamic_alpha_test(&mut self, v: bool) {
        self.has_dynamic_alpha_test = v;
    }

    pub fn finish(&self) -> GxMaterial {
        GxMaterial {
            cull_mode: self.cull_mode,
            light_channels: self.light_channels.clone(),
            tex_gens: self.tex_gens.clone(),
            tev_stages: self.tev_stages.clone(),
            ind_tex_stages: self.ind_tex_stages.clone(),
            alpha_test: self.alpha_test,
            rop: self.rop,
            use_pn_mtx_idx: self.use_pn_mtx_idx,
            use_tex_mtx_idx: self.use_tex_mtx_idx,
            has_dynamic_alpha_test: self.has_dynamic_alpha_test,
        }
    }

    // Decoding from hardware registers (a material display list).

    pub fn set_tex_gen_from_registers(&mut self, r: &DisplayListRegisters, i: u32) {
        let v = r.xf_get(xf_register::XF_TEX0_ID + i, 0);
        let proj = (v >> 1) & 1;
        let tg_type = (v >> 4) & 7;
        let src = (v >> 7) & 0x0F;
        let emboss_src = (v >> 12) & 7;
        let emboss_lgt = (v >> 15) & 7;
        const SRC: [u32; 13] = [
            tex_gen_src::POS,
            tex_gen_src::NRM,
            tex_gen_src::COLOR0,
            tex_gen_src::BINRM,
            tex_gen_src::TANGENT,
            tex_gen_src::TEX0,
            tex_gen_src::TEX1,
            tex_gen_src::TEX2,
            tex_gen_src::TEX3,
            tex_gen_src::TEX4,
            tex_gen_src::TEX5,
            tex_gen_src::TEX6,
            tex_gen_src::TEX7,
        ];
        let (ty, source) = match tg_type {
            0 => (
                if proj == 0 {
                    tex_gen_type::MTX2x4
                } else {
                    tex_gen_type::MTX3x4
                },
                SRC.get(src as usize).copied().unwrap_or(tex_gen_src::TEX0),
            ),
            1 => (
                tex_gen_type::BUMP0 + emboss_lgt,
                tex_gen_src::TEXCOORD0 + emboss_src,
            ),
            2 => (tex_gen_type::SRTG, tex_gen_src::COLOR0),
            _ => (tex_gen_type::SRTG, tex_gen_src::COLOR1),
        };
        // The matrix comes from XF_MATRIXINDEX0/1 (`set_tex_gen_matrix`).
        let dv = r.xf_get(xf_register::XF_DUALTEX0_ID + i, 0);
        let post = (dv & 0xFF) + post_tex_gen_matrix::PTTEXMTX0;
        let normalize = (dv >> 8) & 1 != 0;
        self.set_tex_coord_gen(i, ty, source, tex_gen_matrix::IDENTITY, normalize, post);
    }

    pub fn set_color_channel_from_registers(&mut self, r: &DisplayListRegisters, i: u32) {
        let decode = |c: u32| {
            let attn_en = (c >> 9) & 1 != 0;
            let attn_sel = (c >> 10) & 1 != 0;
            ColorChannelControl {
                mat_color_source: c & 1,
                lighting_enabled: (c >> 1) & 1 != 0,
                lit_mask: (((c >> 11) & 0xF) << 4) | ((c >> 2) & 0xF),
                amb_color_source: (c >> 6) & 1,
                diffuse_function: (c >> 7) & 3,
                attenuation_function: if attn_en {
                    if attn_sel {
                        attenuation_function::SPOT
                    } else {
                        attenuation_function::SPEC
                    }
                } else {
                    attenuation_function::NONE
                },
            }
        };
        let ch = ensure(&mut self.light_channels, i as usize, |_| {
            LightChannelControl::default()
        });
        ch.color = decode(r.xf_get(xf_register::XF_COLOR0CNTRL_ID + i, 0));
        ch.alpha = decode(r.xf_get(xf_register::XF_ALPHA0CNTRL_ID + i, 0));
    }

    pub fn set_tev_stage_from_registers(&mut self, r: &DisplayListRegisters, i: u32) {
        let odd = i & 1 != 0;
        let v = r.bp[(bp_register::RAS1_TREF_0_ID + (i >> 1)) as usize];
        let ti = (v >> if odd { 12 } else { 0 }) & 7;
        let tc = (v >> if odd { 15 } else { 3 }) & 7;
        let te = (v >> if odd { 18 } else { 6 }) & 1 != 0;
        let ch = (v >> if odd { 19 } else { 7 }) & 7;
        self.set_tev_order(i, tc, if te { ti } else { tex_map_id::TEXMAP_NULL }, ch);

        fn find_op(bias: u32, scale: u32, sub: bool) -> u32 {
            if bias == tev_bias::HWB_COMPARE {
                match scale {
                    tev_scale::HWB_R8 => {
                        if sub {
                            tev_op::COMP_R8_EQ
                        } else {
                            tev_op::COMP_R8_GT
                        }
                    }
                    tev_scale::HWB_GR16 => {
                        if sub {
                            tev_op::COMP_GR16_EQ
                        } else {
                            tev_op::COMP_GR16_GT
                        }
                    }
                    tev_scale::HWB_BGR24 => {
                        if sub {
                            tev_op::COMP_BGR24_EQ
                        } else {
                            tev_op::COMP_BGR24_GT
                        }
                    }
                    _ => {
                        if sub {
                            tev_op::COMP_RGB8_EQ
                        } else {
                            tev_op::COMP_RGB8_GT
                        }
                    }
                }
            } else if sub {
                tev_op::SUB
            } else {
                tev_op::ADD
            }
        }
        let c = r.bp[(bp_register::TEV_COLOR_ENV_0_ID + i * 2) as usize];
        let (cb, cs) = ((c >> 16) & 3, (c >> 20) & 3);
        self.set_tev_color_in(i, (c >> 12) & 0xF, (c >> 8) & 0xF, (c >> 4) & 0xF, c & 0xF);
        self.set_tev_color_op(
            i,
            find_op(cb, cs, (c >> 18) & 1 != 0),
            cb,
            cs,
            (c >> 19) & 1 != 0,
            (c >> 22) & 3,
        );
        let a = r.bp[(bp_register::TEV_ALPHA_ENV_0_ID + i * 2) as usize];
        let (rswap, tswap) = (a & 3, (a >> 2) & 3);
        let (ab, as_) = ((a >> 16) & 3, (a >> 20) & 3);
        self.set_tev_alpha_in(i, (a >> 13) & 7, (a >> 10) & 7, (a >> 7) & 7, (a >> 4) & 7);
        self.set_tev_alpha_op(
            i,
            find_op(ab, as_, (a >> 18) & 1 != 0),
            ab,
            as_,
            (a >> 19) & 1 != 0,
            (a >> 22) & 3,
        );

        let ksel = r.bp[(bp_register::TEV_KSEL_0_ID + (i >> 1)) as usize];
        self.set_tev_k_color_sel(i, (if odd { ksel >> 14 } else { ksel >> 4 }) & 0x1F);
        self.set_tev_k_alpha_sel(i, (if odd { ksel >> 19 } else { ksel >> 9 }) & 0x1F);

        let ind = r.bp[(bp_register::IND_CMD0_ID + i) as usize];
        self.set_tev_indirect(
            i,
            ind & 3,
            (ind >> 2) & 3,
            (ind >> 4) & 7,
            (ind >> 9) & 0xF,
            (ind >> 13) & 7,
            (ind >> 16) & 7,
            (ind >> 20) & 1 != 0,
            (ind >> 19) & 1 != 0,
            (ind >> 7) & 3,
        );
        let table = |sel: u32| {
            let rg = r.bp[(bp_register::TEV_KSEL_0_ID + sel * 2) as usize];
            let ba = r.bp[(bp_register::TEV_KSEL_0_ID + sel * 2 + 1) as usize];
            [rg & 3, (rg >> 2) & 3, ba & 3, (ba >> 2) & 3]
        };
        self.set_tev_swap_mode(i, Some(table(rswap)), Some(table(tswap)));
    }

    pub fn set_ind_tex_stage_from_registers(&mut self, r: &DisplayListRegisters, i: u32) {
        let iref = r.bp[bp_register::RAS1_IREF_ID as usize];
        let ss = r.bp[(bp_register::RAS1_SS0_ID + (i >> 1)) as usize];
        let sh = 8 * (i & 1);
        // IREF: per stage, 3 bits texmap then 3 bits texcoord (noclip read
        // both from the same bits).
        self.set_ind_tex_order(i, (iref >> (6 * i + 3)) & 7, (iref >> (6 * i)) & 7);
        self.set_ind_tex_scale(i, (ss >> sh) & 0xF, (ss >> (sh + 4)) & 0xF);
    }

    pub fn set_rop_state_from_registers(&mut self, r: &DisplayListRegisters) {
        if r.bp_is_set(bp_register::PE_CMODE0_ID) {
            let cm = r.bp[bp_register::PE_CMODE0_ID as usize];
            self.set_color_update((cm >> 3) & 1 != 0);
            self.set_alpha_update((cm >> 4) & 1 != 0);
            let mode = if cm & 1 != 0 {
                if (cm >> 11) & 1 != 0 {
                    blend_mode::SUBTRACT
                } else {
                    blend_mode::BLEND
                }
            } else if (cm >> 1) & 1 != 0 {
                blend_mode::LOGIC
            } else {
                blend_mode::NONE
            };
            self.set_blend_mode(mode, (cm >> 8) & 7, (cm >> 5) & 7, (cm >> 12) & 0xF);
        }
        if r.bp_is_set(bp_register::PE_ZMODE_ID) {
            let zm = r.bp[bp_register::PE_ZMODE_ID as usize];
            self.set_z_mode(zm & 1 != 0, (zm >> 1) & 7, (zm >> 4) & 1 != 0);
        }
        if r.bp_is_set(bp_register::TEV_ALPHAFUNC_ID) {
            let ap = r.bp[bp_register::TEV_ALPHAFUNC_ID as usize];
            self.set_alpha_compare(
                (ap >> 16) & 7,
                (ap & 0xFF) as u8,
                (ap >> 22) & 3,
                (ap >> 19) & 7,
                ((ap >> 8) & 0xFF) as u8,
            );
        }
        if r.bp_is_set(bp_register::GEN_MODE_ID) {
            let g = r.bp[bp_register::GEN_MODE_ID as usize];
            let hw = [
                cull_mode::NONE,
                cull_mode::BACK,
                cull_mode::FRONT,
                cull_mode::ALL,
            ];
            self.set_cull_mode(hw[((g >> 14) & 3) as usize]);
        }
    }

    /// Everything a material display list sets.
    pub fn set_from_registers(&mut self, r: &DisplayListRegisters) {
        let g = r.bp[bp_register::GEN_MODE_ID as usize];
        for i in 0..(g & 0xF) {
            self.set_tex_gen_from_registers(r, i);
        }
        for i in 0..((g >> 10) & 0xF) + 1 {
            self.set_tev_stage_from_registers(r, i);
        }
        for i in 0..((g >> 16) & 7) {
            self.set_ind_tex_stage_from_registers(r, i);
        }
        for i in 0..r.xf_get(xf_register::XF_NUMCOLORS_ID, 0).min(2) {
            self.set_color_channel_from_registers(r, i);
        }
        self.set_rop_state_from_registers(r);
    }
}

/// A GX light, in view space (as GXLoadLightObjImm takes it).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Light {
    pub position: [f32; 3],
    pub direction: [f32; 3],
    pub dist_atten: [f32; 3],
    pub cos_atten: [f32; 3],
    /// RGBA, 0..1.
    pub color: [f32; 4],
}

impl Default for Light {
    fn default() -> Self {
        Self {
            position: [0.0; 3],
            direction: [0.0, 0.0, -1.0],
            dist_atten: [1.0, 0.0, 0.0],
            cos_atten: [1.0, 0.0, 0.0],
            color: [0.0, 0.0, 0.0, 1.0],
        }
    }
}

impl Light {
    /// `GXInitLightSpot`.
    pub fn set_spot(&mut self, cutoff_deg: f32, func: u32) {
        let func = if cutoff_deg <= 0.0 || cutoff_deg >= 90.0 {
            spot_function::OFF
        } else {
            func
        };
        let cr = cutoff_deg.to_radians().cos();
        let d = (1.0 - cr) * (1.0 - cr);
        self.cos_atten = match func {
            spot_function::FLAT => [-1000.0 * cr, 1000.0, 0.0],
            spot_function::COS => [-cr / (1.0 - cr), 1.0 / (1.0 - cr), 0.0],
            spot_function::COS2 => [0.0, -cr / (1.0 - cr), 1.0 / (1.0 - cr)],
            spot_function::SHARP => [cr * (cr - 2.0) / d, 2.0 / d, -1.0 / d],
            spot_function::RING1 => [-4.0 * cr / d, 4.0 * (1.0 + cr) / d, -4.0 / d],
            spot_function::RING2 => [1.0 - 2.0 * cr * cr / d, 4.0 * cr / d, -2.0 / d],
            _ => [1.0, 0.0, 0.0],
        };
    }

    /// `GXInitLightDistAttn`.
    pub fn set_dist_attn(&mut self, ref_dist: f32, ref_brightness: f32, func: u32) {
        let func = if ref_dist < 0.0 || ref_brightness <= 0.0 || ref_brightness >= 1.0 {
            dist_attn_function::OFF
        } else {
            func
        };
        let k = (1.0 - ref_brightness) / ref_brightness;
        self.dist_atten = match func {
            dist_attn_function::GENTLE => [1.0, k / ref_dist, 0.0],
            dist_attn_function::MEDIUM => {
                [1.0, 0.5 * k / ref_dist, 0.5 * k / (ref_dist * ref_dist)]
            }
            dist_attn_function::STEEP => [1.0, 0.0, k / (ref_dist * ref_dist)],
            _ => [1.0, 0.0, 0.0],
        };
    }
}

/// Fog parameters (`GXSetFog`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FogBlock {
    pub color: [f32; 4],
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub adj_table: [u16; 10],
    pub adj_center: f32,
}

impl FogBlock {
    /// Compute A, B, C from GX's start/end/near/far.
    pub fn set(&mut self, ty: u32, start_z: f32, end_z: f32, near_z: f32, far_z: f32) {
        if (ty >> 3) != 0 {
            // Orthographic.
            self.a = (far_z - near_z) / (end_z - start_z);
            self.b = 0.0;
            self.c = (start_z - near_z) / (end_z - start_z);
        } else {
            self.a = (far_z * near_z) / ((far_z - near_z) * (end_z - start_z));
            self.b = far_z / (far_z - near_z);
            self.c = start_z / (end_z - start_z);
        }
    }
}
