//! A 4:3 frame fitted to 16:9: wider 3D view, HUD kept in proportion at the edges.
use rustcube_gx::api::{Frame, Gx, Ram};
use rustcube_gx::enums::*;
use rustcube_gx::widescreen::widen;

/// the first game's perspective (as its frames carry it) and its HUD sprite matrix.
const PERSP: [[f32; 4]; 4] = [
    [1.4284168, 0.0, 0.0, 0.0],
    [0.0, 1.9045559, 0.0, 0.0],
    [0.0, 0.0, -1.0, -1.0],
    [0.0, 0.0, -1.0, 0.0],
];
const HUD: [[f32; 4]; 3] = [
    [-0.0021877366, 0.0, 0.7000757, 0.0],
    [0.0, 0.0021877366, -0.5250568, 0.0],
    [0.0, 0.0, 1.0, 0.0],
];

fn frame() -> Frame {
    let mut gx = Gx::new();
    gx.clear_vtx_desc();
    gx.set_vtx_desc(attr::POS, attr_type::DIRECT);
    gx.set_vtx_attr_fmt(0, attr::POS, comp_cnt::POS_XYZ, comp_type::F32, 0);
    gx.set_viewport(0.0, 0.0, 640.0, 480.0, 0.0, 1.0);
    let mem = Ram(&[]);
    let quad = |gx: &mut Gx, pts: [(f32, f32, f32); 4]| {
        gx.begin(command::DRAW_QUADS, 0, 4);
        for (x, y, z) in pts {
            gx.position3f32(x, y, z);
        }
        gx.end(&mem);
    };
    // A 3D quad in front of the camera.
    gx.load_pos_mtx_imm(
        &[
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, -10.0],
        ],
        0,
    );
    quad(
        &mut gx,
        [
            (-1.0, -1.0, 0.0),
            (1.0, -1.0, 0.0),
            (1.0, 1.0, 0.0),
            (-1.0, 1.0, 0.0),
        ],
    );
    // HUD sprites in pixels (x runs negative in its sprite matrix, z = -1):
    // the lives icon at the left, the bar in the middle.
    gx.load_pos_mtx_imm(&HUD, 0);
    quad(
        &mut gx,
        [
            (-30.0, 420.0, -1.0),
            (-90.0, 420.0, -1.0),
            (-90.0, 460.0, -1.0),
            (-30.0, 460.0, -1.0),
        ],
    );
    quad(
        &mut gx,
        [
            (-250.0, 430.0, -1.0),
            (-390.0, 430.0, -1.0),
            (-390.0, 450.0, -1.0),
            (-250.0, 450.0, -1.0),
        ],
    );
    let mut f = gx.take_frame();
    for d in &mut f.draws {
        d.projection = PERSP;
    }
    f
}

/// Where view-space-transformed vertex `v` of draw `d` lands: normalised x.
fn ndc_x(d: &rustcube_gx::api::DrawCmd, v: [f32; 3]) -> f32 {
    let m = d.draw_params.pos_mtx[0];
    let p = d.projection;
    let view: [f32; 3] =
        std::array::from_fn(|r| m[r][0] * v[0] + m[r][1] * v[1] + m[r][2] * v[2] + m[r][3]);
    let clip =
        |row: usize| p[0][row] * view[0] + p[1][row] * view[1] + p[2][row] * view[2] + p[3][row];
    clip(0) / clip(3)
}

#[test]
fn wider_view_and_hud_at_the_edges() {
    let mut f = frame();
    let left = [-30.0, 440.0, -1.0];
    let middle = [-320.0, 440.0, -1.0];
    let corner = [1.0, 1.0, 0.0];
    let before = [
        ndc_x(&f.draws[0], corner),
        ndc_x(&f.draws[1], left),
        ndc_x(&f.draws[2], middle),
    ];
    widen(&mut f, 16.0 / 9.0);
    let k = 0.75; // (4/3) / (16/9)
    let close = |a: f32, b: f32| (a - b).abs() < 1e-4;
    // 3D: a wider view (the same point lands nearer the centre).
    assert!(close(ndc_x(&f.draws[0], corner), before[0] * k));
    // Left HUD: kept in proportion and moved toward the left edge, as the PC release has it.
    assert!(close(ndc_x(&f.draws[1], left), before[1] * k - (1.0 - k) * rustcube_gx::widescreen::HUD_EDGE));
    // Middle HUD: kept in proportion, still centred.
    assert!(close(ndc_x(&f.draws[2], middle), before[2] * k));
    // One projection for every screen draw (they share a render pass).
    assert!(f
        .draws
        .iter()
        .all(|d| d.projection == f.draws[0].projection));
    // 4:3 itself is left alone.
    let mut g = frame();
    let p = g.draws[0].projection;
    widen(&mut g, 4.0 / 3.0);
    assert_eq!(g.draws[0].projection, p);
}

/// Two HUD sprites with a repeating texture: one shows it once, one tiles it 4x.
fn sprites() -> Frame {
    use rustcube_gx::api::{tev_mode, TexObj};
    let ram = vec![0u8; 0x2000];
    let mem = Ram(&ram);
    let mut gx = Gx::new();
    gx.clear_vtx_desc();
    gx.set_vtx_desc(attr::POS, attr_type::DIRECT);
    gx.set_vtx_desc(attr::TEX0, attr_type::DIRECT);
    gx.set_vtx_attr_fmt(0, attr::POS, comp_cnt::POS_XYZ, comp_type::F32, 0);
    gx.set_vtx_attr_fmt(0, attr::TEX0, comp_cnt::TEX_ST, comp_type::F32, 0);
    gx.set_tev_op(0, tev_mode::REPLACE);
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
    gx.load_pos_mtx_imm(&HUD, 0);
    for tiles in [1.0f32, 4.0] {
        gx.begin(command::DRAW_QUADS, 0, 4);
        for (x, y) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
            gx.position3f32(-30.0 - 60.0 * x, 420.0 + 40.0 * y, -1.0);
            gx.tex_coord2f32(x * tiles, y * tiles);
        }
        gx.end(&mem);
    }
    gx.take_frame()
}

#[test]
fn hud_sprites_shown_once_sample_clamped() {
    let mut f = sprites();
    rustcube_gx::widescreen::clamp_hud_textures(&mut f);
    let wrap = |i: usize| {
        f.draws[i].material_params.texture_mappings[0]
            .sampler
            .wrap_s
    };
    assert_eq!(
        wrap(0),
        wrap_mode::CLAMP,
        "a sprite showing its texture once"
    );
    assert_eq!(wrap(1), wrap_mode::REPEAT, "a tiled backdrop keeps repeat");
}

#[test]
fn backdrop_tiles_stretch_together() {
    // A menu backdrop as three full-height tiles (pixel x runs negative, y too).
    let mut gx = Gx::new();
    gx.clear_vtx_desc();
    gx.set_vtx_desc(attr::POS, attr_type::DIRECT);
    gx.set_vtx_attr_fmt(0, attr::POS, comp_cnt::POS_XYZ, comp_type::F32, 0);
    gx.set_viewport(0.0, 0.0, 640.0, 480.0, 0.0, 1.0);
    gx.load_pos_mtx_imm(&HUD, 0);
    let mem = Ram(&[]);
    for t in 0..3 {
        let (x0, x1) = (-(t as f32) * 213.0, -(t as f32 + 1.0) * 213.0);
        gx.begin(command::DRAW_QUADS, 0, 4);
        for (x, y) in [(x0, 0.0), (x1, 0.0), (x1, -480.0), (x0, -480.0)] {
            gx.position3f32(x, y, -1.0);
        }
        gx.end(&mem);
    }
    let mut f = gx.take_frame();
    for d in &mut f.draws {
        d.projection = PERSP;
    }
    let edges: Vec<[f32; 2]> = (0..3)
        .map(|t| {
            let (x0, x1) = (-(t as f32) * 213.0, -(t as f32 + 1.0) * 213.0);
            [
                ndc_x(&f.draws[t], [x0, 0.0, -1.0]),
                ndc_x(&f.draws[t], [x1, 0.0, -1.0]),
            ]
        })
        .collect();
    widen(&mut f, 16.0 / 9.0);
    for t in 0..3 {
        let (x0, x1) = (-(t as f32) * 213.0, -(t as f32 + 1.0) * 213.0);
        let now = [
            ndc_x(&f.draws[t], [x0, 0.0, -1.0]),
            ndc_x(&f.draws[t], [x1, 0.0, -1.0]),
        ];
        for (a, b) in now.iter().zip(&edges[t]) {
            assert!(
                (a - b).abs() < 1e-4,
                "tile {t} stretched to fill, joined to its neighbours"
            );
        }
    }
}

#[test]
fn a_screen_of_2d_art_is_pillarboxed() {
    // Only sprites (no 3D scene): drawn unchanged into the middle 4:3.
    let mut f = frame();
    f.draws.remove(0); // the 3D quad
    let before: Vec<_> = f
        .draws
        .iter()
        .map(|d| (d.projection, d.draw_params.pos_mtx))
        .collect();
    widen(&mut f, 16.0 / 9.0);
    for (d, (p, m)) in f.draws.iter().zip(&before) {
        assert_eq!(
            (&d.projection, &d.draw_params.pos_mtx),
            (p, m),
            "art itself unchanged"
        );
        assert_eq!(d.viewport[0], 80.0, "starts an eighth in (640 x 0.25 / 2)");
        assert_eq!(d.viewport[2], 480.0, "4:3 of the 16:9 width");
    }
}

#[test]
fn a_flash_over_2d_art_stays_in_the_middle() {
    // An untextured full-screen quad (the title's white flash) over 2D art: the
    // side bars stay black instead of turning grey.
    let mut gx = Gx::new();
    gx.clear_vtx_desc();
    gx.set_vtx_desc(attr::POS, attr_type::DIRECT);
    gx.set_vtx_attr_fmt(0, attr::POS, comp_cnt::POS_XYZ, comp_type::F32, 0);
    gx.set_viewport(0.0, 0.0, 640.0, 480.0, 0.0, 1.0);
    gx.load_pos_mtx_imm(&HUD, 0);
    gx.begin(command::DRAW_QUADS, 0, 4);
    for (x, y) in [(0.0, 0.0), (-640.0, 0.0), (-640.0, 480.0), (0.0, 480.0)] {
        gx.position3f32(x, y, -1.0);
    }
    gx.end(&Ram(&[]));
    let mut f = gx.take_frame();
    f.draws[0].projection = PERSP;
    widen(&mut f, 16.0 / 9.0);
    assert_eq!(f.draws[0].viewport[0], 80.0);
    assert_eq!(f.draws[0].viewport[2], 480.0);
}

#[test]
fn a_row_of_glyphs_moves_as_one() {
    // A 3D scene, then two neighbouring HUD glyphs whose own centres fall on
    // either side of the anchoring line (normalised x 1/3): the results
    // screen's "05:21:02" was split between the middle and the right edge.
    let mut gx = Gx::new();
    gx.clear_vtx_desc();
    gx.set_vtx_desc(attr::POS, attr_type::DIRECT);
    gx.set_vtx_attr_fmt(0, attr::POS, comp_cnt::POS_XYZ, comp_type::F32, 0);
    gx.set_viewport(0.0, 0.0, 640.0, 480.0, 0.0, 1.0);
    let mem = Ram(&[]);
    let quad = |gx: &mut Gx, x0: f32, x1: f32, y0: f32, y1: f32, z: f32| {
        gx.begin(command::DRAW_QUADS, 0, 4);
        for (x, y) in [(x0, y0), (x1, y0), (x1, y1), (x0, y1)] {
            gx.position3f32(x, y, z);
        }
        gx.end(&mem);
    };
    gx.load_pos_mtx_imm(
        &[
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, -10.0],
        ],
        0,
    );
    quad(&mut gx, -1.0, 1.0, -1.0, 1.0, 0.0);
    gx.load_pos_mtx_imm(&HUD, 0);
    quad(&mut gx, -416.0, -426.0, 200.0, 220.0, -1.0);
    quad(&mut gx, -428.0, -438.0, 200.0, 220.0, -1.0);
    let mut f = gx.take_frame();
    for d in &mut f.draws {
        d.projection = PERSP;
    }
    let before: Vec<f32> = f.draws[1..]
        .iter()
        .map(|d| ndc_x(d, [-421.0, 210.0, -1.0]))
        .collect();
    let own: Vec<f32> = f.draws[1..]
        .iter()
        .zip([-421.0, -433.0])
        .map(|(d, x)| ndc_x(d, [x, 210.0, -1.0]))
        .collect();
    assert!(own[0] < 1.0 / 3.0 && own[1] > 1.0 / 3.0, "straddle: {own:?}");
    widen(&mut f, 16.0 / 9.0);
    let after: Vec<f32> = f.draws[1..]
        .iter()
        .map(|d| ndc_x(d, [-421.0, 210.0, -1.0]))
        .collect();
    let moved: Vec<f32> = after.iter().zip(&before).map(|(a, b)| a - b).collect();
    assert!((moved[0] - moved[1]).abs() < 1e-4, "same move: {moved:?}");
}

#[test]
fn a_lock_on_reticle_follows_the_3d_view() {
    // A reticle is a HUD sprite the game draws between 3D draws, at its
    // target's 4:3 screen spot; fixed HUD (score, lives) comes after the last
    // 3D draw. In the left third the reticle must not jump to the edge.
    let mut gx = Gx::new();
    gx.clear_vtx_desc();
    gx.set_vtx_desc(attr::POS, attr_type::DIRECT);
    gx.set_vtx_attr_fmt(0, attr::POS, comp_cnt::POS_XYZ, comp_type::F32, 0);
    gx.set_viewport(0.0, 0.0, 640.0, 480.0, 0.0, 1.0);
    let mem = Ram(&[]);
    let quad = |gx: &mut Gx, x0: f32, x1: f32, y0: f32, y1: f32, z: f32| {
        gx.begin(command::DRAW_QUADS, 0, 4);
        for (x, y) in [(x0, y0), (x1, y0), (x1, y1), (x0, y1)] {
            gx.position3f32(x, y, z);
        }
        gx.end(&mem);
    };
    let world = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, -10.0],
    ];
    gx.load_pos_mtx_imm(&world, 0);
    quad(&mut gx, -1.0, 1.0, -1.0, 1.0, 0.0);
    gx.load_pos_mtx_imm(&HUD, 0);
    quad(&mut gx, -60.0, -120.0, 200.0, 260.0, -1.0); // reticle, left third
    gx.load_pos_mtx_imm(&world, 0);
    quad(&mut gx, -1.0, 1.0, -1.0, 1.0, 0.0); // more 3D after it
    gx.load_pos_mtx_imm(&HUD, 0);
    quad(&mut gx, -30.0, -90.0, 420.0, 460.0, -1.0); // lives icon, left third
    let mut f = gx.take_frame();
    for d in &mut f.draws {
        d.projection = PERSP;
    }
    let reticle = [-90.0, 230.0, -1.0];
    let lives = [-60.0, 440.0, -1.0];
    let before = (ndc_x(&f.draws[1], reticle), ndc_x(&f.draws[3], lives));
    assert!(before.0 < -1.0 / 3.0 && before.1 < -1.0 / 3.0);
    widen(&mut f, 16.0 / 9.0);
    let k = 0.75;
    let close = |a: f32, b: f32| (a - b).abs() < 1e-4;
    // The reticle scales with the wider 3D view, like the point it marks.
    assert!(close(ndc_x(&f.draws[1], reticle), before.0 * k));
    // Fixed HUD still moves to its edge.
    assert!(close(ndc_x(&f.draws[3], lives), before.1 * k - (1.0 - k) * rustcube_gx::widescreen::HUD_EDGE));
}

#[test]
fn text_over_a_centred_panel_stays_with_it() {
    // The result screen: a panel half the screen wide in the middle, a label that starts just
    // left of it and a number that ends just right of it; and the lives icon far from it.
    // Moved to the screen edges, label and number hung out of the panel on both sides.
    let mut gx = Gx::new();
    gx.clear_vtx_desc();
    gx.set_vtx_desc(attr::POS, attr_type::DIRECT);
    gx.set_vtx_attr_fmt(0, attr::POS, comp_cnt::POS_XYZ, comp_type::F32, 0);
    gx.set_viewport(0.0, 0.0, 640.0, 480.0, 0.0, 1.0);
    let mem = Ram(&[]);
    let quad = |gx: &mut Gx, x0: f32, x1: f32, y0: f32, y1: f32, z: f32| {
        gx.begin(command::DRAW_QUADS, 0, 4);
        for (x, y) in [(x0, y0), (x1, y0), (x1, y1), (x0, y1)] {
            gx.position3f32(x, y, z);
        }
        gx.end(&mem);
    };
    gx.load_pos_mtx_imm(&[[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, -10.0]], 0);
    quad(&mut gx, -1.0, 1.0, -1.0, 1.0, 0.0);
    gx.load_pos_mtx_imm(&HUD, 0);
    quad(&mut gx, -160.0, -480.0, 100.0, 200.0, -1.0); // panel
    quad(&mut gx, -130.0, -190.0, 110.0, 130.0, -1.0); // label over its left edge
    quad(&mut gx, -470.0, -500.0, 110.0, 130.0, -1.0); // number over its right edge
    quad(&mut gx, -30.0, -90.0, 420.0, 460.0, -1.0); // lives icon
    let mut f = gx.take_frame();
    for d in &mut f.draws {
        d.projection = PERSP;
    }
    let at = [[-320.0, 150.0], [-160.0, 120.0], [-485.0, 120.0], [-60.0, 440.0]];
    let x = |f: &Frame| -> Vec<f32> { f.draws[1..].iter().zip(at).map(|(d, p)| ndc_x(d, [p[0], p[1], -1.0])).collect() };
    let before = x(&f);
    assert!(before[1] < -1.0 / 3.0 && before[2] > 1.0 / 3.0, "outside the middle third: {before:?}");
    widen(&mut f, 16.0 / 9.0);
    let after = x(&f);
    let k = 0.75;
    for i in 0..3 {
        assert!((after[i] - before[i] * k).abs() < 1e-4, "piece {i} keeps its place by the panel: {before:?} {after:?}");
    }
    assert!((after[3] - (before[3] * k - (1.0 - k) * rustcube_gx::widescreen::HUD_EDGE)).abs() < 1e-4, "the lives icon still goes to the edge");
}
