// Derived from noclip.website (https://github.com/magcius/noclip.website),
// src/gx/gx_enum.ts at commit 2cebac2a5d1d7c44535397a14f51e764ca823767.
// Copyright (c) 2018 Jasper St. Pierre. MIT License: see THIRD-PARTY.md.
// Translated from TypeScript to Rust for rustcube-gx.
//! GX constants (noclip took most of them from libogc), as raw register
//! values so recompiled code can pass its integers straight through.
#![allow(non_upper_case_globals)]

/// GX `TexFormat`.
pub mod tex_format {
    pub const I4: u32 = 0;
    pub const I8: u32 = 1;
    pub const IA4: u32 = 2;
    pub const IA8: u32 = 3;
    pub const RGB565: u32 = 4;
    pub const RGB5A3: u32 = 5;
    pub const RGBA8: u32 = 6;
    pub const C4: u32 = 8;
    pub const C8: u32 = 9;
    pub const C14X2: u32 = 0xa;
    pub const CMPR: u32 = 0xe;
}

/// GX `TexPalette`.
pub mod tex_palette {
    pub const IA8: u32 = 0;
    pub const RGB565: u32 = 1;
    pub const RGB5A3: u32 = 2;
}

/// GX `TexFilter`.
pub mod tex_filter {
    pub const NEAR: u32 = 0;
    pub const LINEAR: u32 = 1;
    pub const NEAR_MIP_NEAR: u32 = 2;
    pub const LIN_MIP_NEAR: u32 = 3;
    pub const NEAR_MIP_LIN: u32 = 4;
    pub const LIN_MIP_LIN: u32 = 5;
}

/// GX `Anisotropy`.
pub mod anisotropy {
    pub const _1: u32 = 0;
    pub const _2: u32 = 1;
    pub const _4: u32 = 2;
}

/// GX `Command`.
pub mod command {
    pub const NOOP: u32 = 0;
    pub const DRAW_QUADS: u32 = 0x80;
    pub const DRAW_QUAD_STRIP: u32 = 0x88;
    pub const DRAW_TRIANGLES: u32 = 0x90;
    pub const DRAW_TRIANGLE_STRIP: u32 = 0x98;
    pub const DRAW_TRIANGLE_FAN: u32 = 0xa0;
    pub const DRAW_LINES: u32 = 0xa8;
    pub const DRAW_LINE_STRIP: u32 = 0xb0;
    pub const DRAW_POINTS: u32 = 0xb8;
    pub const LOAD_INDX_A: u32 = 0x20;
    pub const LOAD_INDX_B: u32 = 0x28;
    pub const LOAD_INDX_C: u32 = 0x30;
    pub const LOAD_INDX_D: u32 = 0x38;
    pub const LOAD_BP_REG: u32 = 0x61;
    pub const LOAD_CP_REG: u32 = 8;
    pub const LOAD_XF_REG: u32 = 0x10;
    pub const CALL_DL: u32 = 0x40;
    pub const INVAL_VTX_CACHE: u32 = 0x48;
}

/// GX `Attr`.
pub mod attr {
    pub const PNMTXIDX: u32 = 0;
    pub const TEX0MTXIDX: u32 = 1;
    pub const TEX1MTXIDX: u32 = 2;
    pub const TEX2MTXIDX: u32 = 3;
    pub const TEX3MTXIDX: u32 = 4;
    pub const TEX4MTXIDX: u32 = 5;
    pub const TEX5MTXIDX: u32 = 6;
    pub const TEX6MTXIDX: u32 = 7;
    pub const TEX7MTXIDX: u32 = 8;
    pub const POS: u32 = 9;
    pub const NRM: u32 = 0xa;
    pub const CLR0: u32 = 0xb;
    pub const CLR1: u32 = 0xc;
    pub const TEX0: u32 = 0xd;
    pub const TEX1: u32 = 0xe;
    pub const TEX2: u32 = 0xf;
    pub const TEX3: u32 = 0x10;
    pub const TEX4: u32 = 0x11;
    pub const TEX5: u32 = 0x12;
    pub const TEX6: u32 = 0x13;
    pub const TEX7: u32 = 0x14;
    pub const MAX: u32 = 0x14;
    pub const _NBT: u32 = 0x19;
    pub const NULL: u32 = 0xff;
}

/// GX `CompCnt`.
pub mod comp_cnt {
    pub const POS_XY: u32 = 0;
    pub const POS_XYZ: u32 = 1;
    pub const NRM_XYZ: u32 = 0;
    pub const NRM_NBT: u32 = 1;
    pub const NRM_NBT3: u32 = 2;
    pub const CLR_RGB: u32 = 0;
    pub const CLR_RGBA: u32 = 1;
    pub const TEX_S: u32 = 0;
    pub const TEX_ST: u32 = 1;
}

/// GX `CompType`.
pub mod comp_type {
    pub const U8: u32 = 0;
    pub const S8: u32 = 1;
    pub const U16: u32 = 2;
    pub const S16: u32 = 3;
    pub const F32: u32 = 4;
    pub const RGB565: u32 = 0;
    pub const RGB8: u32 = 1;
    pub const RGBX8: u32 = 2;
    pub const RGBA4: u32 = 3;
    pub const RGBA6: u32 = 4;
    pub const RGBA8: u32 = 5;
}

/// GX `CompareType`.
pub mod compare_type {
    pub const NEVER: u32 = 0;
    pub const LESS: u32 = 1;
    pub const EQUAL: u32 = 2;
    pub const LEQUAL: u32 = 3;
    pub const GREATER: u32 = 4;
    pub const NEQUAL: u32 = 5;
    pub const GEQUAL: u32 = 6;
    pub const ALWAYS: u32 = 7;
}

/// GX `AlphaOp`.
pub mod alpha_op {
    pub const AND: u32 = 0;
    pub const OR: u32 = 1;
    pub const XOR: u32 = 2;
    pub const XNOR: u32 = 3;
}

/// GX `CullMode`.
pub mod cull_mode {
    pub const NONE: u32 = 0;
    pub const FRONT: u32 = 1;
    pub const BACK: u32 = 2;
    pub const ALL: u32 = 3;
}

/// GX `BlendMode`.
pub mod blend_mode {
    pub const NONE: u32 = 0;
    pub const BLEND: u32 = 1;
    pub const LOGIC: u32 = 2;
    pub const SUBTRACT: u32 = 3;
}

/// GX `BlendFactor`.
pub mod blend_factor {
    pub const ZERO: u32 = 0;
    pub const ONE: u32 = 1;
    pub const SRCCLR: u32 = 2;
    pub const INVSRCCLR: u32 = 3;
    pub const SRCALPHA: u32 = 4;
    pub const INVSRCALPHA: u32 = 5;
    pub const DSTALPHA: u32 = 6;
    pub const INVDSTALPHA: u32 = 7;
}

/// GX `LogicOp`.
pub mod logic_op {
    pub const CLEAR: u32 = 0;
    pub const AND: u32 = 1;
    pub const REVAND: u32 = 2;
    pub const COPY: u32 = 3;
    pub const INVAND: u32 = 4;
    pub const NOOP: u32 = 5;
    pub const XOR: u32 = 6;
    pub const OR: u32 = 7;
    pub const NOR: u32 = 8;
    pub const EQUIV: u32 = 9;
    pub const INV: u32 = 0xa;
    pub const REVOR: u32 = 0xb;
    pub const INVCOPY: u32 = 0xc;
    pub const INVOR: u32 = 0xd;
    pub const NAND: u32 = 0xe;
    pub const SET: u32 = 0xf;
}

/// GX `TevOp`.
pub mod tev_op {
    pub const ADD: u32 = 0;
    pub const SUB: u32 = 1;
    pub const COMP_R8_GT: u32 = 8;
    pub const COMP_R8_EQ: u32 = 9;
    pub const COMP_GR16_GT: u32 = 0xa;
    pub const COMP_GR16_EQ: u32 = 0xb;
    pub const COMP_BGR24_GT: u32 = 0xc;
    pub const COMP_BGR24_EQ: u32 = 0xd;
    pub const COMP_RGB8_GT: u32 = 0xe;
    pub const COMP_RGB8_EQ: u32 = 0xf;
    pub const COMP_A8_GT: u32 = 0xe;
    pub const COMP_A8_EQ: u32 = 0xf;
}

/// GX `TevBias`.
pub mod tev_bias {
    pub const ZERO: u32 = 0;
    pub const ADDHALF: u32 = 1;
    pub const SUBHALF: u32 = 2;
    pub const HWB_COMPARE: u32 = 3;
}

/// GX `TevScale`.
pub mod tev_scale {
    pub const SCALE_1: u32 = 0;
    pub const SCALE_2: u32 = 1;
    pub const SCALE_4: u32 = 2;
    pub const DIVIDE_2: u32 = 3;
    pub const HWB_R8: u32 = 0;
    pub const HWB_GR16: u32 = 1;
    pub const HWB_BGR24: u32 = 2;
    pub const HWB_RGB8: u32 = 3;
}

/// GX `CC`.
pub mod cc {
    pub const CPREV: u32 = 0;
    pub const APREV: u32 = 1;
    pub const C0: u32 = 2;
    pub const A0: u32 = 3;
    pub const C1: u32 = 4;
    pub const A1: u32 = 5;
    pub const C2: u32 = 6;
    pub const A2: u32 = 7;
    pub const TEXC: u32 = 8;
    pub const TEXA: u32 = 9;
    pub const RASC: u32 = 0xa;
    pub const RASA: u32 = 0xb;
    pub const ONE: u32 = 0xc;
    pub const HALF: u32 = 0xd;
    pub const KONST: u32 = 0xe;
    pub const ZERO: u32 = 0xf;
}

/// GX `CA`.
pub mod ca {
    pub const APREV: u32 = 0;
    pub const A0: u32 = 1;
    pub const A1: u32 = 2;
    pub const A2: u32 = 3;
    pub const TEXA: u32 = 4;
    pub const RASA: u32 = 5;
    pub const KONST: u32 = 6;
    pub const ZERO: u32 = 7;
}

/// GX `KonstColorSel`.
pub mod konst_color_sel {
    pub const KCSEL_1: u32 = 0;
    pub const KCSEL_7_8: u32 = 1;
    pub const KCSEL_6_8: u32 = 2;
    pub const KCSEL_5_8: u32 = 3;
    pub const KCSEL_4_8: u32 = 4;
    pub const KCSEL_3_8: u32 = 5;
    pub const KCSEL_2_8: u32 = 6;
    pub const KCSEL_1_8: u32 = 7;
    pub const KCSEL_K0: u32 = 0xc;
    pub const KCSEL_K1: u32 = 0xd;
    pub const KCSEL_K2: u32 = 0xe;
    pub const KCSEL_K3: u32 = 0xf;
    pub const KCSEL_K0_R: u32 = 0x10;
    pub const KCSEL_K1_R: u32 = 0x11;
    pub const KCSEL_K2_R: u32 = 0x12;
    pub const KCSEL_K3_R: u32 = 0x13;
    pub const KCSEL_K0_G: u32 = 0x14;
    pub const KCSEL_K1_G: u32 = 0x15;
    pub const KCSEL_K2_G: u32 = 0x16;
    pub const KCSEL_K3_G: u32 = 0x17;
    pub const KCSEL_K0_B: u32 = 0x18;
    pub const KCSEL_K1_B: u32 = 0x19;
    pub const KCSEL_K2_B: u32 = 0x1a;
    pub const KCSEL_K3_B: u32 = 0x1b;
    pub const KCSEL_K0_A: u32 = 0x1c;
    pub const KCSEL_K1_A: u32 = 0x1d;
    pub const KCSEL_K2_A: u32 = 0x1e;
    pub const KCSEL_K3_A: u32 = 0x1f;
}

/// GX `KonstAlphaSel`.
pub mod konst_alpha_sel {
    pub const KASEL_1: u32 = 0;
    pub const KASEL_7_8: u32 = 1;
    pub const KASEL_6_8: u32 = 2;
    pub const KASEL_5_8: u32 = 3;
    pub const KASEL_4_8: u32 = 4;
    pub const KASEL_3_8: u32 = 5;
    pub const KASEL_2_8: u32 = 6;
    pub const KASEL_1_8: u32 = 7;
    pub const KASEL_K0_R: u32 = 0x10;
    pub const KASEL_K1_R: u32 = 0x11;
    pub const KASEL_K2_R: u32 = 0x12;
    pub const KASEL_K3_R: u32 = 0x13;
    pub const KASEL_K0_G: u32 = 0x14;
    pub const KASEL_K1_G: u32 = 0x15;
    pub const KASEL_K2_G: u32 = 0x16;
    pub const KASEL_K3_G: u32 = 0x17;
    pub const KASEL_K0_B: u32 = 0x18;
    pub const KASEL_K1_B: u32 = 0x19;
    pub const KASEL_K2_B: u32 = 0x1a;
    pub const KASEL_K3_B: u32 = 0x1b;
    pub const KASEL_K0_A: u32 = 0x1c;
    pub const KASEL_K1_A: u32 = 0x1d;
    pub const KASEL_K2_A: u32 = 0x1e;
    pub const KASEL_K3_A: u32 = 0x1f;
}

/// GX `TevColorChan`.
pub mod tev_color_chan {
    pub const R: u32 = 0;
    pub const G: u32 = 1;
    pub const B: u32 = 2;
    pub const A: u32 = 3;
}

/// GX `WrapMode`.
pub mod wrap_mode {
    pub const CLAMP: u32 = 0;
    pub const REPEAT: u32 = 1;
    pub const MIRROR: u32 = 2;
}

/// GX `ColorSrc`.
pub mod color_src {
    pub const REG: u32 = 0;
    pub const VTX: u32 = 1;
}

/// GX `TexGenSrc`.
pub mod tex_gen_src {
    pub const POS: u32 = 0;
    pub const NRM: u32 = 1;
    pub const BINRM: u32 = 2;
    pub const TANGENT: u32 = 3;
    pub const TEX0: u32 = 4;
    pub const TEX1: u32 = 5;
    pub const TEX2: u32 = 6;
    pub const TEX3: u32 = 7;
    pub const TEX4: u32 = 8;
    pub const TEX5: u32 = 9;
    pub const TEX6: u32 = 0xa;
    pub const TEX7: u32 = 0xb;
    pub const TEXCOORD0: u32 = 0xc;
    pub const TEXCOORD1: u32 = 0xd;
    pub const TEXCOORD2: u32 = 0xe;
    pub const TEXCOORD3: u32 = 0xf;
    pub const TEXCOORD4: u32 = 0x10;
    pub const TEXCOORD5: u32 = 0x11;
    pub const TEXCOORD6: u32 = 0x12;
    pub const COLOR0: u32 = 0x13;
    pub const COLOR1: u32 = 0x14;
}

/// GX `TexGenType`.
pub mod tex_gen_type {
    pub const MTX3x4: u32 = 0;
    pub const MTX2x4: u32 = 1;
    pub const BUMP0: u32 = 2;
    pub const BUMP1: u32 = 3;
    pub const BUMP2: u32 = 4;
    pub const BUMP3: u32 = 5;
    pub const BUMP4: u32 = 6;
    pub const BUMP5: u32 = 7;
    pub const BUMP6: u32 = 8;
    pub const BUMP7: u32 = 9;
    pub const SRTG: u32 = 0xa;
}

/// GX `PosNrmMatrix`.
pub mod pos_nrm_matrix {
    pub const PNMTX0: u32 = 0;
    pub const PNMTX1: u32 = 3;
    pub const PNMTX2: u32 = 6;
    pub const PNMTX3: u32 = 9;
    pub const PNMTX4: u32 = 0xc;
    pub const PNMTX5: u32 = 0xf;
    pub const PNMTX6: u32 = 0x12;
    pub const PNMTX7: u32 = 0x15;
    pub const PNMTX8: u32 = 0x18;
    pub const PNMTX9: u32 = 0x1b;
}

/// GX `TexGenMatrix`.
pub mod tex_gen_matrix {
    pub const IDENTITY: u32 = 0x3c;
    pub const TEXMTX0: u32 = 0x1e;
    pub const TEXMTX1: u32 = 0x21;
    pub const TEXMTX2: u32 = 0x24;
    pub const TEXMTX3: u32 = 0x27;
    pub const TEXMTX4: u32 = 0x2a;
    pub const TEXMTX5: u32 = 0x2d;
    pub const TEXMTX6: u32 = 0x30;
    pub const TEXMTX7: u32 = 0x33;
    pub const TEXMTX8: u32 = 0x36;
    pub const TEXMTX9: u32 = 0x39;
    pub const PNMTX0: u32 = 0;
    pub const PNMTX1: u32 = 3;
    pub const PNMTX2: u32 = 6;
    pub const PNMTX3: u32 = 9;
    pub const PNMTX4: u32 = 0xc;
    pub const PNMTX5: u32 = 0xf;
    pub const PNMTX6: u32 = 0x12;
    pub const PNMTX7: u32 = 0x15;
    pub const PNMTX8: u32 = 0x18;
    pub const PNMTX9: u32 = 0x1b;
}

/// GX `PostTexGenMatrix`.
pub mod post_tex_gen_matrix {
    pub const PTTEXMTX0: u32 = 0x40;
    pub const PTTEXMTX1: u32 = 0x43;
    pub const PTTEXMTX2: u32 = 0x46;
    pub const PTTEXMTX3: u32 = 0x49;
    pub const PTTEXMTX4: u32 = 0x4c;
    pub const PTTEXMTX5: u32 = 0x4f;
    pub const PTTEXMTX6: u32 = 0x52;
    pub const PTTEXMTX7: u32 = 0x55;
    pub const PTTEXMTX8: u32 = 0x58;
    pub const PTTEXMTX9: u32 = 0x5b;
    pub const PTTEXMTX10: u32 = 0x5e;
    pub const PTTEXMTX11: u32 = 0x61;
    pub const PTTEXMTX12: u32 = 0x64;
    pub const PTTEXMTX13: u32 = 0x67;
    pub const PTTEXMTX14: u32 = 0x6a;
    pub const PTTEXMTX15: u32 = 0x6d;
    pub const PTTEXMTX16: u32 = 0x70;
    pub const PTTEXMTX17: u32 = 0x73;
    pub const PTTEXMTX18: u32 = 0x76;
    pub const PTTEXMTX19: u32 = 0x79;
    pub const PTIDENTITY: u32 = 0x7d;
}

/// GX `Register`.
pub mod register {
    pub const PREV: u32 = 0;
    pub const REG0: u32 = 1;
    pub const REG1: u32 = 2;
    pub const REG2: u32 = 3;
}

/// GX `TexCoordID`.
pub mod tex_coord_id {
    pub const TEXCOORD0: u32 = 0;
    pub const TEXCOORD1: u32 = 1;
    pub const TEXCOORD2: u32 = 2;
    pub const TEXCOORD3: u32 = 3;
    pub const TEXCOORD4: u32 = 4;
    pub const TEXCOORD5: u32 = 5;
    pub const TEXCOORD6: u32 = 6;
    pub const TEXCOORD7: u32 = 7;
    pub const TEXCOORD_NULL: u32 = 0xff;
}

/// GX `ColorChannelID`.
pub mod color_channel_id {
    pub const COLOR0: u32 = 0;
    pub const COLOR1: u32 = 1;
    pub const ALPHA0: u32 = 2;
    pub const ALPHA1: u32 = 3;
    pub const COLOR0A0: u32 = 4;
    pub const COLOR1A1: u32 = 5;
    pub const COLOR_ZERO: u32 = 6;
    pub const ALPHA_BUMP: u32 = 7;
    pub const ALPHA_BUMP_N: u32 = 8;
    pub const COLOR_NULL: u32 = 0xff;
}

/// GX `RasColorChannelID`.
pub mod ras_color_channel_id {
    pub const COLOR0A0: u32 = 0;
    pub const COLOR1A1: u32 = 1;
    pub const ALPHA_BUMP: u32 = 5;
    pub const ALPHA_BUMP_N: u32 = 6;
    pub const COLOR_ZERO: u32 = 7;
}

/// GX `VtxFmt`.
pub mod vtx_fmt {
    pub const VTXFMT0: u32 = 0;
    pub const VTXFMT1: u32 = 1;
    pub const VTXFMT2: u32 = 2;
    pub const VTXFMT3: u32 = 3;
    pub const VTXFMT4: u32 = 4;
    pub const VTXFMT5: u32 = 5;
    pub const VTXFMT6: u32 = 6;
    pub const VTXFMT7: u32 = 7;
}

/// GX `AttrType`.
pub mod attr_type {
    pub const NONE: u32 = 0;
    pub const DIRECT: u32 = 1;
    pub const INDEX8: u32 = 2;
    pub const INDEX16: u32 = 3;
    pub const ZERO: u32 = 4;
}

/// GX `TexMapID`.
pub mod tex_map_id {
    pub const TEXMAP0: u32 = 0;
    pub const TEXMAP1: u32 = 1;
    pub const TEXMAP2: u32 = 2;
    pub const TEXMAP3: u32 = 3;
    pub const TEXMAP4: u32 = 4;
    pub const TEXMAP5: u32 = 5;
    pub const TEXMAP6: u32 = 6;
    pub const TEXMAP7: u32 = 7;
    pub const TEXMAP_NULL: u32 = 0xff;
}

/// GX `IndTexScale`.
pub mod ind_tex_scale {
    pub const _1: u32 = 0;
    pub const _2: u32 = 1;
    pub const _4: u32 = 2;
    pub const _8: u32 = 3;
    pub const _16: u32 = 4;
    pub const _32: u32 = 5;
    pub const _64: u32 = 6;
    pub const _128: u32 = 7;
    pub const _256: u32 = 8;
}

/// GX `IndTexBiasSel`.
pub mod ind_tex_bias_sel {
    pub const NONE: u32 = 0;
    pub const S: u32 = 1;
    pub const T: u32 = 2;
    pub const ST: u32 = 3;
    pub const U: u32 = 4;
    pub const SU: u32 = 5;
    pub const TU: u32 = 6;
    pub const STU: u32 = 7;
}

/// GX `IndTexAlphaSel`.
pub mod ind_tex_alpha_sel {
    pub const OFF: u32 = 0;
    pub const S: u32 = 1;
    pub const T: u32 = 2;
    pub const U: u32 = 3;
}

/// GX `IndTexFormat`.
pub mod ind_tex_format {
    pub const _8: u32 = 0;
    pub const _5: u32 = 1;
    pub const _4: u32 = 2;
    pub const _3: u32 = 3;
}

/// GX `IndTexWrap`.
pub mod ind_tex_wrap {
    pub const OFF: u32 = 0;
    pub const _256: u32 = 1;
    pub const _128: u32 = 2;
    pub const _64: u32 = 3;
    pub const _32: u32 = 4;
    pub const _16: u32 = 5;
    pub const _0: u32 = 6;
}

/// GX `IndTexStageID`.
pub mod ind_tex_stage_id {
    pub const STAGE0: u32 = 0;
    pub const STAGE1: u32 = 1;
    pub const STAGE2: u32 = 2;
    pub const STAGE3: u32 = 3;
}

/// GX `IndTexMtxID`.
pub mod ind_tex_mtx_id {
    pub const OFF: u32 = 0;
    pub const _0: u32 = 1;
    pub const _1: u32 = 2;
    pub const _2: u32 = 3;
    pub const S0: u32 = 5;
    pub const S1: u32 = 6;
    pub const S2: u32 = 7;
    pub const T0: u32 = 9;
    pub const T1: u32 = 0xa;
    pub const T2: u32 = 0xb;
}

/// GX `XFRegister`.
pub mod xf_register {
    pub const XF_INVTXSPEC_ID: u32 = 0x1008;
    pub const XF_NUMCOLORS_ID: u32 = 0x1009;
    pub const XF_AMBIENT0_ID: u32 = 0x100a;
    pub const XF_AMBIENT1_ID: u32 = 0x100b;
    pub const XF_MATERIAL0_ID: u32 = 0x100c;
    pub const XF_MATERIAL1_ID: u32 = 0x100d;
    pub const XF_COLOR0CNTRL_ID: u32 = 0x100e;
    pub const XF_COLOR1CNTRL_ID: u32 = 0x100f;
    pub const XF_ALPHA0CNTRL_ID: u32 = 0x1010;
    pub const XF_ALPHA1CNTRL_ID: u32 = 0x1011;
    pub const XF_DUALTEXTRANS_ID: u32 = 0x1012;
    pub const XF_MATRIXINDEX0_ID: u32 = 0x1018;
    pub const XF_MATRIXINDEX1_ID: u32 = 0x1019;
    pub const XF_VPSCALEX_ID: u32 = 0x101a;
    pub const XF_VPSCALEY_ID: u32 = 0x101b;
    pub const XF_VPSCALEZ_ID: u32 = 0x101c;
    pub const XF_VPOFFSETX_ID: u32 = 0x101d;
    pub const XF_VPOFFSETY_ID: u32 = 0x101e;
    pub const XF_VPOFFSETZ_ID: u32 = 0x101f;
    pub const XF_PROJECTIONA_ID: u32 = 0x1020;
    pub const XF_PROJECTIONB_ID: u32 = 0x1021;
    pub const XF_PROJECTIONC_ID: u32 = 0x1022;
    pub const XF_PROJECTIOND_ID: u32 = 0x1023;
    pub const XF_PROJECTIONE_ID: u32 = 0x1024;
    pub const XF_PROJECTIONF_ID: u32 = 0x1025;
    pub const XF_PROJECTORTHO_ID: u32 = 0x1026;
    pub const XF_NUMTEX_ID: u32 = 0x103f;
    pub const XF_TEX0_ID: u32 = 0x1040;
    pub const XF_DUALTEX0_ID: u32 = 0x1050;
}

/// GX `BPRegister`.
pub mod bp_register {
    pub const GEN_MODE_ID: u32 = 0;
    pub const IND_MTXA0_ID: u32 = 6;
    pub const IND_MTXB0_ID: u32 = 7;
    pub const IND_MTXC0_ID: u32 = 8;
    pub const IND_CMD0_ID: u32 = 0x10;
    pub const RAS1_SS0_ID: u32 = 0x25;
    pub const RAS1_IREF_ID: u32 = 0x27;
    pub const RAS1_TREF_0_ID: u32 = 0x28;
    pub const SU_SSIZE_I0_ID: u32 = 0x30;
    pub const SU_SSIZE_I1_ID: u32 = 0x32;
    pub const SU_SSIZE_I2_ID: u32 = 0x34;
    pub const SU_SSIZE_I3_ID: u32 = 0x36;
    pub const SU_SSIZE_I4_ID: u32 = 0x38;
    pub const SU_SSIZE_I5_ID: u32 = 0x3a;
    pub const SU_SSIZE_I6_ID: u32 = 0x3c;
    pub const SU_SSIZE_I7_ID: u32 = 0x3e;
    pub const SU_TSIZE_I0_ID: u32 = 0x31;
    pub const SU_TSIZE_I1_ID: u32 = 0x33;
    pub const SU_TSIZE_I2_ID: u32 = 0x35;
    pub const SU_TSIZE_I3_ID: u32 = 0x37;
    pub const SU_TSIZE_I4_ID: u32 = 0x39;
    pub const SU_TSIZE_I5_ID: u32 = 0x3b;
    pub const SU_TSIZE_I6_ID: u32 = 0x3d;
    pub const SU_TSIZE_I7_ID: u32 = 0x3f;
    pub const PE_ZMODE_ID: u32 = 0x40;
    pub const PE_CMODE0_ID: u32 = 0x41;
    pub const TX_LOADTLUT_I0_ID: u32 = 0x64;
    pub const TX_SETMODE0_I0_ID: u32 = 0x80;
    pub const TX_SETMODE0_I4_ID: u32 = 0xa0;
    pub const TX_SETMODE1_I0_ID: u32 = 0x84;
    pub const TX_SETMODE1_I4_ID: u32 = 0xa4;
    pub const TX_SETIMAGE0_I0_ID: u32 = 0x88;
    pub const TX_SETIMAGE0_I4_ID: u32 = 0xa8;
    pub const TX_SETIMAGE1_I0_ID: u32 = 0x8c;
    pub const TX_SETIMAGE1_I4_ID: u32 = 0xac;
    pub const TX_SETIMAGE2_I0_ID: u32 = 0x90;
    pub const TX_SETIMAGE2_I4_ID: u32 = 0xb0;
    pub const TX_SETIMAGE3_I0_ID: u32 = 0x94;
    pub const TX_SETIMAGE3_I4_ID: u32 = 0xb4;
    pub const TX_SETTLUT_I0_ID: u32 = 0x98;
    pub const TX_SETTLUT_I4_ID: u32 = 0xb8;
    pub const TEV_COLOR_ENV_0_ID: u32 = 0xc0;
    pub const TEV_ALPHA_ENV_0_ID: u32 = 0xc1;
    pub const TEV_REGISTERL_0_ID: u32 = 0xe0;
    pub const TEV_REGISTERH_0_ID: u32 = 0xe1;
    pub const TEV_FOG_PARAM_0_ID: u32 = 0xee;
    pub const TEV_FOG_PARAM_1_ID: u32 = 0xef;
    pub const TEV_FOG_PARAM_2_ID: u32 = 0xf0;
    pub const TEV_FOG_PARAM_3_ID: u32 = 0xf1;
    pub const TEV_FOG_COLOR_ID: u32 = 0xf2;
    pub const TEV_ALPHAFUNC_ID: u32 = 0xf3;
    pub const TEV_KSEL_0_ID: u32 = 0xf6;
    pub const SS_MASK: u32 = 0xfe;
}

/// GX `CPRegister`.
pub mod cp_register {
    pub const MATINDEX_A_ID: u32 = 0x30;
    pub const MATINDEX_B_ID: u32 = 0x40;
    pub const VCD_LO_ID: u32 = 0x50;
    pub const VCD_HI_ID: u32 = 0x60;
    pub const VAT_A_ID: u32 = 0x70;
    pub const VAT_B_ID: u32 = 0x80;
    pub const VAT_C_ID: u32 = 0x90;
}

/// GX `DiffuseFunction`.
pub mod diffuse_function {
    pub const NONE: u32 = 0;
    pub const SIGN: u32 = 1;
    pub const CLAMP: u32 = 2;
}

/// GX `AttenuationFunction`.
pub mod attenuation_function {
    pub const SPEC: u32 = 0;
    pub const SPOT: u32 = 1;
    pub const NONE: u32 = 2;
}

/// GX `DistAttnFunction`.
pub mod dist_attn_function {
    pub const OFF: u32 = 0;
    pub const GENTLE: u32 = 1;
    pub const MEDIUM: u32 = 2;
    pub const STEEP: u32 = 3;
}

/// GX `SpotFunction`.
pub mod spot_function {
    pub const OFF: u32 = 0;
    pub const FLAT: u32 = 1;
    pub const COS: u32 = 2;
    pub const COS2: u32 = 3;
    pub const SHARP: u32 = 4;
    pub const RING1: u32 = 5;
    pub const RING2: u32 = 6;
}

/// GX `ProjectionType`.
pub mod projection_type {
    pub const PERSPECTIVE: u32 = 0;
    pub const ORTHOGRAPHIC: u32 = 1;
}

/// GX `FogType`.
pub mod fog_type {
    pub const NONE: u32 = 0;
    pub const PERSP_LIN: u32 = 2;
    pub const PERSP_EXP: u32 = 4;
    pub const PERSP_EXP2: u32 = 5;
    pub const PERSP_REVEXP: u32 = 6;
    pub const PERSP_REVEXP2: u32 = 7;
    pub const ORTHO_LIN: u32 = 0xa;
    pub const ORTHO_EXP: u32 = 0xc;
    pub const ORTHO_EXP2: u32 = 0xd;
    pub const ORTHO_REVEXP: u32 = 0xe;
    pub const ORTHO_REVEXP2: u32 = 0xf;
}
