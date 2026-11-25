//! Generated WGSL must validate for every kind of GX state, and the GX API
//! must record draws headlessly.
use rustcube_gx::api::{tev_mode, Gx, Ram, TexObj};
use rustcube_gx::enums::*;
use rustcube_gx::material::GxMaterial;
use rustcube_gx::shader::{self, MaterialHacks};
use rustcube_gx::vtx::VertexInput;

fn validate(m: &GxMaterial) {
    for has_all in [true, false] {
        let has = move |i: VertexInput| has_all || i == VertexInput::Pos;
        let src = shader::generate(m, MaterialHacks::default(), &has).source;
        let module = naga::front::wgsl::parse_str(&src)
            .unwrap_or_else(|e| panic!("{}\n{src}", e.emit_to_string(&src)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{e:?}\n{src}"));
    }
}

fn base() -> Gx {
    let mut gx = Gx::new();
    gx.clear_vtx_desc();
    gx.set_vtx_desc(attr::POS, attr_type::DIRECT);
    gx.set_vtx_desc(attr::CLR0, attr_type::DIRECT);
    gx.set_vtx_desc(attr::TEX0, attr_type::DIRECT);
    gx.set_vtx_attr_fmt(0, attr::POS, comp_cnt::POS_XYZ, comp_type::F32, 0);
    gx.set_vtx_attr_fmt(0, attr::CLR0, comp_cnt::CLR_RGBA, comp_type::RGBA8, 0);
    gx.set_vtx_attr_fmt(0, attr::TEX0, comp_cnt::TEX_ST, comp_type::F32, 0);
    gx
}

#[test]
fn tev_presets_validate() {
    for mode in [
        tev_mode::MODULATE,
        tev_mode::DECAL,
        tev_mode::BLEND,
        tev_mode::REPLACE,
        tev_mode::PASSCLR,
    ] {
        let mut gx = base();
        gx.set_num_tev_stages(2);
        gx.set_tev_op(0, mode);
        gx.set_tev_op(1, mode);
        gx.set_tev_order(
            1,
            tex_coord_id::TEXCOORD0,
            tex_map_id::TEXMAP1,
            color_channel_id::COLOR0A0,
        );
        validate(&gx.current_material());
    }
}

#[test]
fn every_tev_input_op_and_konst_validates() {
    let mut gx = base();
    gx.set_num_tev_stages(16);
    let ops = [
        tev_op::ADD,
        tev_op::SUB,
        tev_op::COMP_R8_GT,
        tev_op::COMP_R8_EQ,
        tev_op::COMP_GR16_GT,
        tev_op::COMP_GR16_EQ,
        tev_op::COMP_BGR24_GT,
        tev_op::COMP_BGR24_EQ,
        tev_op::COMP_RGB8_GT,
        tev_op::COMP_RGB8_EQ,
    ];
    for s in 0..16u32 {
        gx.set_tev_color_in(s, s % 16, (s + 3) % 16, (s + 7) % 16, (s + 11) % 16);
        gx.set_tev_alpha_in(s, s % 8, (s + 3) % 8, (s + 5) % 8, (s + 7) % 8);
        let op = ops[s as usize % ops.len()];
        gx.set_tev_color_op(s, op, s % 3, s % 4, s % 2 == 0, s % 4);
        gx.set_tev_alpha_op(s, op, s % 3, (s + 1) % 4, s % 2 == 1, (s + 1) % 4);
        gx.set_tev_k_color_sel(s, [0, 3, 7, 0x0C, 0x11, 0x16, 0x1B, 0x1F][s as usize % 8]);
        gx.set_tev_k_alpha_sel(s, [0, 5, 0x10, 0x15, 0x1A, 0x1F][s as usize % 6]);
        gx.set_tev_swap_mode(s, s % 4, (s + 1) % 4);
        let chan = [
            color_channel_id::COLOR0A0,
            color_channel_id::COLOR1A1,
            color_channel_id::COLOR_ZERO,
            color_channel_id::ALPHA_BUMP,
            color_channel_id::ALPHA_BUMP_N,
        ][s as usize % 5];
        gx.set_tev_order(s, s % 8, if s == 5 { 0xFF } else { s % 8 }, chan);
    }
    gx.set_num_tex_gens(8);
    for t in 0..8 {
        gx.set_tex_coord_gen(
            t,
            tex_gen_type::MTX2x4,
            tex_gen_src::TEX0 + t,
            tex_gen_matrix::TEXMTX0 + 3 * t,
        );
    }
    gx.set_alpha_compare(
        compare_type::GREATER,
        0x40,
        alpha_op::XOR,
        compare_type::LESS,
        0xC0,
    );
    validate(&gx.current_material());
}

#[test]
fn indirect_texgen_lighting_fog_validate() {
    let mut gx = base();
    gx.set_vtx_desc(attr::PNMTXIDX, attr_type::DIRECT);
    gx.set_vtx_desc(attr::TEX1MTXIDX, attr_type::DIRECT);
    gx.set_vtx_desc(attr::NRM, attr_type::DIRECT);
    gx.set_num_chans(2);
    gx.set_chan_ctrl(
        color_channel_id::COLOR0A0,
        true,
        color_src::REG,
        color_src::VTX,
        0b101,
        diffuse_function::CLAMP,
        attenuation_function::SPOT,
    );
    gx.set_chan_ctrl(
        color_channel_id::COLOR1,
        true,
        color_src::VTX,
        color_src::REG,
        0b10,
        diffuse_function::SIGN,
        attenuation_function::SPEC,
    );
    gx.set_num_tex_gens(4);
    gx.set_tex_coord_gen2(
        0,
        tex_gen_type::MTX3x4,
        tex_gen_src::POS,
        tex_gen_matrix::TEXMTX1,
        true,
        post_tex_gen_matrix::PTTEXMTX2,
    );
    gx.set_tex_coord_gen(
        1,
        tex_gen_type::MTX2x4,
        tex_gen_src::NRM,
        tex_gen_matrix::PNMTX0,
    );
    gx.set_tex_coord_gen(
        2,
        tex_gen_type::BUMP0,
        tex_gen_src::TEXCOORD0,
        tex_gen_matrix::IDENTITY,
    );
    gx.set_tex_coord_gen(
        3,
        tex_gen_type::SRTG,
        tex_gen_src::COLOR0,
        tex_gen_matrix::IDENTITY,
    );
    gx.set_num_ind_stages(2);
    gx.set_ind_tex_order(0, tex_coord_id::TEXCOORD1, tex_map_id::TEXMAP2);
    gx.set_ind_tex_coord_scale(0, ind_tex_scale::_2, ind_tex_scale::_4);
    gx.set_ind_tex_order(1, tex_coord_id::TEXCOORD2, tex_map_id::TEXMAP3);
    gx.set_ind_tex_mtx(1, [[0.5, 0.0, 0.0], [0.0, 0.5, 0.0]], 1);
    gx.set_num_tev_stages(4);
    gx.set_tev_ind_warp(0, 0, true, false, ind_tex_mtx_id::_0);
    gx.set_tev_indirect(
        1,
        1,
        ind_tex_format::_8,
        ind_tex_bias_sel::STU,
        ind_tex_mtx_id::S1,
        ind_tex_wrap::_256,
        ind_tex_wrap::_0,
        true,
        false,
        ind_tex_alpha_sel::T,
    );
    gx.set_tev_indirect(
        2,
        0,
        ind_tex_format::_5,
        ind_tex_bias_sel::U,
        ind_tex_mtx_id::T2,
        ind_tex_wrap::_16,
        ind_tex_wrap::OFF,
        false,
        false,
        ind_tex_alpha_sel::U,
    );
    gx.set_tev_order(2, 2, 2, color_channel_id::ALPHA_BUMP);
    gx.set_tev_order(3, 3, 3, color_channel_id::COLOR1A1);
    for ty in [
        fog_type::PERSP_LIN,
        fog_type::PERSP_EXP,
        fog_type::PERSP_EXP2,
        fog_type::PERSP_REVEXP,
        fog_type::ORTHO_REVEXP2,
    ] {
        gx.set_fog(ty, 10.0, 100.0, 1.0, 1000.0, [128, 128, 255, 255]);
        gx.set_dst_alpha(true, 0x80);
        gx.set_blend_mode(
            blend_mode::BLEND,
            blend_factor::SRCALPHA,
            blend_factor::INVSRCALPHA,
            logic_op::COPY,
        );
        validate(&gx.current_material());
    }
}

#[test]
fn immediate_mode_records_a_textured_quad() {
    // RAM with one 4x4 RGB565 texture at 0x1000.
    let mut ram = vec![0u8; 0x2000];
    for i in 0..16 {
        ram[0x1000 + 2 * i..0x1002 + 2 * i].copy_from_slice(&0xF800u16.to_be_bytes());
    }
    let mem = Ram(&ram);
    let mut gx = base();
    gx.set_tev_op(0, tev_mode::MODULATE);
    gx.load_tex_obj(
        &TexObj::new(
            0x8000_1000,
            4,
            4,
            tex_format::RGB565,
            wrap_mode::REPEAT,
            wrap_mode::REPEAT,
            false,
        ),
        0,
    );
    gx.begin(command::DRAW_QUADS, 0, 4);
    for (x, y) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
        gx.position3f32(x, y, 0.0);
        gx.color4u8(255, 255, 255, 255);
        gx.tex_coord2f32(x, y);
    }
    gx.end(&mem);
    let f = gx.take_frame();
    assert_eq!(f.draws.len(), 1);
    assert_eq!(f.draws[0].indices, vec![0, 1, 2, 0, 2, 3]);
    assert_eq!(f.uploads.len(), 1);
    assert_eq!(&f.uploads[0].levels[0].2[0..4], &[255, 0, 0, 255]);
    validate(&f.draws[0].material);
}

#[test]
fn display_list_with_register_loads() {
    let mut gx = base();
    // CP: VCD_LO = POS direct (bit 9) only; VAT_A fmt 0: POS xyz f32.
    // BP: ZMODE (test, LEQUAL, update) then a 3-vertex triangle.
    let mut dl = vec![0x08, 0x50, 0, 0, 0x02, 0x00];
    dl.extend([0x08, 0x70, 0, 0, 0, (4 << 1) | 1]);
    dl.extend([0x61, 0x40, 0, 0, 0x17]);
    dl.extend([0x90, 0x00, 0x03]);
    for v in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
        for c in v {
            dl.extend(c.to_be_bytes());
        }
    }
    dl.push(0);
    gx.run_display_list(&dl, &Ram(&[]));
    let f = gx.take_frame();
    assert_eq!(f.draws.len(), 1);
    assert_eq!(f.draws[0].indices.len(), 3);
    assert!(f.draws[0].material.rop.depth_test);
    validate(&f.draws[0].material);
}

#[test]
fn material_cache_follows_state_changes() {
    let mut gx = base();
    gx.set_tev_op(0, tev_mode::MODULATE);
    let a = gx.current_material();
    gx.set_num_tev_stages(2);
    let b = gx.current_material();
    assert_ne!(
        a.tev_stages.len(),
        b.tev_stages.len(),
        "stage count change seen"
    );
    gx.set_tev_op(0, tev_mode::REPLACE);
    assert_ne!(b, gx.current_material(), "TEV change seen");
    gx.set_num_tev_stages(1);
    gx.set_tev_op(0, tev_mode::MODULATE);
    assert_eq!(a, gx.current_material(), "same state, same material");
}
