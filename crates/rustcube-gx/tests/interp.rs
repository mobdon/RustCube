//! Frames between game frames blend matched draws' position matrices.
use rustcube_gx::api::{Frame, Gx, Ram};
use rustcube_gx::enums::*;
use rustcube_gx::interp::Tween;

/// One frame: a triangle drawn with matrix 0 translated by `x` per draw in
/// `xs` (a second, larger triangle when `extra` is set).
fn frame(xs: &[f32], extra: bool) -> Frame {
    let mut gx = Gx::new();
    gx.clear_vtx_desc();
    gx.set_vtx_desc(attr::POS, attr_type::DIRECT);
    gx.set_vtx_attr_fmt(0, attr::POS, comp_cnt::POS_XYZ, comp_type::F32, 0);
    let mem = Ram(&[]);
    let tri = |gx: &mut Gx, n: u16| {
        gx.begin(command::DRAW_TRIANGLES, 0, n);
        for i in 0..n {
            gx.position3f32(i as f32, 0.0, 0.0);
        }
        gx.end(&mem);
    };
    if extra {
        tri(&mut gx, 6);
    }
    for &x in xs {
        gx.load_pos_mtx_imm(
            &[
                [1.0, 0.0, 0.0, x],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
            ],
            0,
        );
        tri(&mut gx, 3);
    }
    gx.take_frame()
}

fn tx(f: &Frame, i: usize) -> f32 {
    f.draws[i].draw_params.pos_mtx[0][0][3]
}

#[test]
fn blends_matched_draws_and_snaps_jumps() {
    let prev = frame(&[0.0, 10.0, 0.0], false);
    // A new draw first (no match), then the same three, the last one jumped.
    let mut cur = frame(&[4.0, 20.0, 5000.0], true);
    let tw = Tween::new(&prev, &cur);
    assert_eq!(tw.matched(), 2);
    tw.apply(&mut cur, 0.25);
    assert_eq!(tx(&cur, 1), 1.0, "a quarter of the way from 0 to 4");
    assert_eq!(tx(&cur, 2), 12.5);
    assert_eq!(tx(&cur, 3), 5000.0, "a jump shows where it landed");
    tw.apply(&mut cur, 1.0);
    assert_eq!(
        (tx(&cur, 1), tx(&cur, 2)),
        (4.0, 20.0),
        "t = 1 is the game frame"
    );
    tw.apply(&mut cur, 0.0);
    assert_eq!(tx(&cur, 1), 0.0);
}

#[test]
fn a_vanished_copy_does_not_shift_the_others() {
    // Three copies; the first is gone in the next frame (a ring collected).
    let prev = frame(&[0.0, 100.0, 200.0], false);
    let mut cur = frame(&[104.0, 204.0], false);
    let tw = Tween::new(&prev, &cur);
    tw.apply(&mut cur, 0.5);
    assert_eq!(
        (tx(&cur, 0), tx(&cur, 1)),
        (102.0, 202.0),
        "each from its own copy"
    );
}

#[test]
fn a_camera_turn_cut_snaps() {
    let turned = |a: f32| {
        let mut f = frame(&[0.0], false);
        let (s, c) = a.sin_cos();
        f.draws[0].draw_params.pos_mtx[0] =
            [[c, 0.0, s, 0.0], [0.0, 1.0, 0.0, 0.0], [-s, 0.0, c, 0.0]];
        f
    };
    let prev = turned(0.0);
    // A small turn blends; a cut to look the other way doesn't.
    assert_eq!(Tween::new(&prev, &turned(0.1)).matched(), 1);
    assert_eq!(Tween::new(&prev, &turned(2.5)).matched(), 0);
}

#[test]
fn a_far_copy_is_not_the_same_object() {
    // One copy leaves and another appears across the room: nothing to blend from.
    let prev = frame(&[0.0], false);
    let mut cur = frame(&[60.0], false);
    let tw = Tween::new(&prev, &cur);
    assert_eq!(tw.matched(), 0);
    tw.apply(&mut cur, 0.5);
    assert_eq!(tx(&cur, 0), 60.0);
    // Far from the camera a turn moves a draw further, and still blends.
    let far = |x: f32| {
        let mut f = frame(&[x], false);
        f.draws[0].draw_params.pos_mtx[0][2][3] = -400.0;
        f
    };
    assert_eq!(Tween::new(&far(0.0), &far(60.0)).matched(), 1);
}

#[test]
fn an_effect_on_another_texture_frame_blends_from_itself() {
    // Two copies of an animated effect, 30 apart: each steps to the texture the other had.
    let with = |xs: &[f32], ids: [u32; 2]| {
        let mut f = frame(xs, false);
        for (d, id) in f.draws.iter_mut().zip(ids) {
            d.material_params.texture_mappings[0].texture = Some(id);
        }
        f
    };
    let prev = with(&[0.0, 30.0], [1, 2]);
    let mut cur = with(&[1.0, 31.0], [2, 1]);
    let tw = Tween::new(&prev, &cur);
    tw.apply(&mut cur, 0.5);
    assert_eq!((tx(&cur, 0), tx(&cur, 1)), (0.5, 30.5), "each from itself, not from the copy with its texture");
}
