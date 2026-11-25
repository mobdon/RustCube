//! The FIFO sink must turn the raw command stream (as the game writes it to
//! 0xCC008000, in pieces of any size) into the same draws as the GX API.
use rustcube_gx::api::Ram;
use rustcube_gx::fifo::GxFifo;

fn bp(out: &mut Vec<u8>, v: u32) {
    out.push(0x61);
    out.extend(v.to_be_bytes());
}
fn cp(out: &mut Vec<u8>, a: u8, v: u32) {
    out.extend([0x08, a]);
    out.extend(v.to_be_bytes());
}
fn xf(out: &mut Vec<u8>, addr: u16, words: &[u32]) {
    out.push(0x10);
    out.extend(((words.len() - 1) as u16).to_be_bytes());
    out.extend(addr.to_be_bytes());
    for w in words {
        out.extend(w.to_be_bytes());
    }
}

/// Position-only, F32 XYZ in format 0, then one triangle.
fn triangle() -> Vec<u8> {
    let mut s = Vec::new();
    cp(&mut s, 0x50, 1 << 9); // VCD: POS direct
    cp(&mut s, 0x60, 0);
    cp(&mut s, 0x70, 1 | (4 << 1)); // VAT0: POS XYZ F32
    s.extend([0x90, 0, 3]);
    for v in [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
        for c in v {
            s.extend(c.to_be_bytes());
        }
    }
    s
}

#[test]
fn byte_at_a_time_draws_once() {
    let mut f = GxFifo::default();
    for b in triangle() {
        f.write(&[b], &Ram(&[]));
    }
    assert_eq!(f.pending(), 0);
    let frame = f.gx.take_frame();
    assert_eq!(frame.draws.len(), 1);
    assert_eq!(frame.draws[0].indices.len(), 3);
}

#[test]
fn partial_command_waits() {
    let mut f = GxFifo::default();
    let s = triangle();
    f.write(&s[..s.len() - 1], &Ram(&[]));
    assert_eq!(f.gx.take_frame().draws.len(), 0);
    f.write(&s[s.len() - 1..], &Ram(&[]));
    assert_eq!(f.gx.take_frame().draws.len(), 1);
}

#[test]
fn viewport_scissor_and_clear() {
    let mut f = GxFifo::default();
    let mut s = Vec::new();
    // GXSetViewport(10, 20, 320, 240, 0, 1).
    let z = 16_777_215.0f32;
    let vp = [
        160.0,
        -120.0,
        z,
        10.0 + 160.0 + 342.0,
        20.0 + 120.0 + 342.0,
        z,
    ];
    xf(&mut s, 0x101A, &vp.map(f32::to_bits));
    // GXSetScissor(4, 8, 100, 50).
    bp(&mut s, 0x20 << 24 | (4 + 342) << 12 | (8 + 342));
    bp(&mut s, 0x21 << 24 | (4 + 342 + 99) << 12 | (8 + 342 + 49));
    // GXSetCopyClear(r 1, g 2, b 3, a 4).
    bp(&mut s, 0x4F << 24 | 4 << 8 | 1);
    bp(&mut s, 0x50 << 24 | 2 << 8 | 3);
    s.extend(triangle());
    f.write(&s, &Ram(&[]));
    let frame = f.gx.take_frame();
    let d = &frame.draws[0];
    assert_eq!(d.viewport, [10.0, 20.0, 320.0, 240.0, 0.0, 1.0]);
    assert_eq!(d.scissor, [4, 8, 100, 50]);
    assert_eq!(frame.clear_color, [1.0, 2.0, 3.0, 4.0].map(|c| c / 255.0));
}

#[test]
fn perspective_projection_from_xf() {
    let mut f = GxFifo::default();
    let mut s = Vec::new();
    xf(
        &mut s,
        0x1020,
        &[2.0f32, 0.5, 3.0, 0.25, -1.0, -2.0].map(f32::to_bits),
    );
    xf(&mut s, 0x1026, &[0]);
    s.extend(triangle());
    f.write(&s, &Ram(&[]));
    // Row major [[2,0,.5,0],[0,3,.25,0],[0,0,-1,-2],[0,0,-1,0]]; column 2 of
    // the wgpu matrix is GX row 2 plus row 3 (z remap), transposed.
    let p = f.gx.take_frame().draws[0].projection;
    assert_eq!(p[0], [2.0, 0.0, 0.0, 0.0]);
    assert_eq!(p[2], [0.5, 0.25, -2.0, -1.0]);
    assert_eq!(p[3], [0.0, 0.0, -2.0, 0.0]);
}

#[test]
fn call_display_list_runs_and_cannot_recurse() {
    let tri = triangle();
    let mut ram = vec![0u8; 0x200];
    ram[0x100..0x100 + tri.len()].copy_from_slice(&tri);
    // A second list at 0x180 that calls itself.
    let me = [0x40u8, 0x80, 0, 1, 0x80, 0, 0, 0, 9];
    ram[0x180..0x189].copy_from_slice(&me);
    let mut s = vec![0x40];
    s.extend(0x8000_0100u32.to_be_bytes());
    s.extend((tri.len() as u32).to_be_bytes());
    s.extend(me);
    let mut f = GxFifo::default();
    f.write(&s, &Ram(&ram));
    assert_eq!(f.gx.take_frame().draws.len(), 1);
    assert_eq!(f.pending(), 0);
}

#[test]
fn texture_from_bp_registers() {
    let mut f = GxFifo::default();
    let mut s = Vec::new();
    // Map 0: 8x4 RGBA8 at 0x80000000, clamp, linear.
    bp(&mut s, 0x80 << 24 | 4 << 5 | 1 << 4);
    bp(&mut s, 0x88 << 24 | 6 << 20 | 3 << 10 | 7);
    bp(&mut s, 0x94 << 24);
    s.extend(triangle());
    let ram = vec![0xFFu8; 0x400];
    f.write(&s, &Ram(&ram));
    let frame = f.gx.take_frame();
    assert_eq!(frame.uploads.len(), 1);
    let (w, h, _) = &frame.uploads[0].levels[0];
    assert_eq!((*w, *h), (8, 4));
}

#[test]
fn invalidate_reuploads_under_the_same_id() {
    // The game invalidates textures every frame; new ids each time made the
    // renderer keep every frame's copy (gigabytes within a minute). Bytes
    // that didn't change aren't decoded again at all.
    let mut f = GxFifo::default();
    let mut s = Vec::new();
    bp(&mut s, 0x80 << 24 | 4 << 5 | 1 << 4);
    bp(&mut s, 0x88 << 24 | 6 << 20 | 3 << 10 | 7);
    bp(&mut s, 0x94 << 24);
    s.extend(triangle());
    let mut inv = Vec::new();
    bp(&mut inv, 0x66 << 24);
    let mut ram = vec![0xFFu8; 0x400];
    let mut ids = Vec::new();
    for (round, uploads) in [1, 0, 1].into_iter().enumerate() {
        if round == 2 {
            ram[0] = 0; // the game rewrote the texture
        }
        f.write(&s, &Ram(&ram));
        let frame = f.gx.take_frame();
        assert_eq!(frame.uploads.len(), uploads, "round {round}");
        ids.extend(frame.uploads.iter().map(|u| u.id));
        ids.extend(
            frame
                .draws
                .iter()
                .filter_map(|d| d.material_params.texture_mappings[0].texture),
        );
        f.write(&inv, &Ram(&ram));
    }
    assert!(ids.iter().all(|&i| i == ids[0]), "{ids:?}");
    // Without an invalidate the cached texture is used as is.
    f.write(&s, &Ram(&ram));
    f.write(&s, &Ram(&ram));
    assert!(f.gx.take_frame().uploads.is_empty());
}

#[test]
fn unknown_bytes_are_skipped() {
    let mut f = GxFifo::default();
    let mut s = vec![0x07, 0xFF];
    s.extend(triangle());
    f.write(&s, &Ram(&[]));
    assert_eq!(f.gx.take_frame().draws.len(), 1);
}

/// One more triangle in the format `triangle` set up.
fn another(s: &mut Vec<u8>) {
    s.extend([0x90, 0, 3]);
    for v in [[2.0f32, 0.0, 0.0], [3.0, 0.0, 0.0], [2.0, 1.0, 0.0]] {
        for c in v {
            s.extend(c.to_be_bytes());
        }
    }
}

/// A view matrix that is neither the identity nor the HUD's pixel scale.
fn view_3d(s: &mut Vec<u8>) {
    let m = [
        1.0f32, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, -10.0,
    ];
    xf(s, 0, &m.map(f32::to_bits));
}

#[test]
fn draws_with_no_state_change_between_them_join() {
    let mut f = GxFifo::default();
    let mut s = Vec::new();
    view_3d(&mut s);
    s.extend(triangle());
    another(&mut s);
    another(&mut s);
    f.write(&s, &Ram(&[]));
    let frame = f.gx.take_frame();
    assert_eq!(frame.draws.len(), 1);
    let d = &frame.draws[0];
    assert_eq!(d.indices, [0, 1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(d.vertices.len(), 9 * d.layout.stride);
    // The second triangle's first vertex is its own (x = 2), after the first's three.
    let x =
        |v: usize| f32::from_le_bytes(d.vertices[v * d.layout.stride..][..4].try_into().unwrap());
    assert_eq!([x(0), x(3), x(6)], [0.0, 2.0, 2.0]);
}

#[test]
fn a_state_change_or_a_2d_draw_starts_a_new_draw() {
    // A register write between two 3D draws.
    let mut f = GxFifo::default();
    let mut s = Vec::new();
    view_3d(&mut s);
    s.extend(triangle());
    xf(&mut s, 11, &[(-20.0f32).to_bits()]);
    another(&mut s);
    f.write(&s, &Ram(&[]));
    assert_eq!(f.gx.take_frame().draws.len(), 2);
    // View-space draws (identity matrix: 2D quads) are placed one by one.
    let mut f = GxFifo::default();
    let mut s = triangle();
    another(&mut s);
    f.write(&s, &Ram(&[]));
    assert_eq!(f.gx.take_frame().draws.len(), 2);
    // A new frame starts its own draws.
    let mut f = GxFifo::default();
    let mut s = Vec::new();
    view_3d(&mut s);
    s.extend(triangle());
    f.write(&s, &Ram(&[]));
    assert_eq!(f.gx.take_frame().draws.len(), 1);
    let mut s = Vec::new();
    another(&mut s);
    f.write(&s, &Ram(&[]));
    assert_eq!(f.gx.take_frame().draws.len(), 1);
}

/// Two 3D triangles with `between` written between them.
fn draws_around(between: &[u8]) -> usize {
    let mut f = GxFifo::default();
    let mut s = Vec::new();
    view_3d(&mut s);
    s.extend(triangle());
    s.extend(between);
    another(&mut s);
    f.write(&s, &Ram(&[]));
    f.gx.take_frame().draws.len()
}

#[test]
fn writes_that_change_nothing_do_not_split_a_draw() {
    // The state the first draw already had, written again in each family.
    let mut same = Vec::new();
    view_3d(&mut same);
    for _ in 0..2 {
        bp(&mut same, 0x4000_0017); // z mode
        cp(&mut same, 0xA0, 0x1234); // array base
        xf(&mut same, 0x1018, &[0]); // matrix index
    }
    // The first of each is new; the repeats are not.
    let mut f = GxFifo::default();
    let mut s = same.clone();
    s.extend(triangle());
    s.extend(&same);
    another(&mut s);
    s.extend([0x98, 0, 0]); // a draw of no vertices
    another(&mut s);
    f.write(&s, &Ram(&[]));
    assert_eq!(f.gx.take_frame().draws.len(), 1);
    // A register this decoder doesn't model changes nothing a draw reads.
    let mut unmodelled = Vec::new();
    bp(&mut unmodelled, 0x3000_0001);
    cp(&mut unmodelled, 0x30, 1);
    xf(&mut unmodelled, 0x1008, &[1]);
    assert_eq!(draws_around(&unmodelled), 1);
}

#[test]
fn the_first_write_of_a_value_and_acting_registers_count() {
    for between in [
        {
            let mut b = Vec::new();
            bp(&mut b, 0x4000_0017);
            b
        },
        {
            let mut b = Vec::new();
            cp(&mut b, 0xA0, 0x1234);
            b
        },
        {
            let mut b = Vec::new();
            xf(&mut b, 0x1018, &[3]);
            b
        },
    ] {
        assert_eq!(draws_around(&between), 2, "{between:x?}");
    }
    // Texture invalidation acts each time it is written.
    let mut f = GxFifo::default();
    let mut s = Vec::new();
    view_3d(&mut s);
    bp(&mut s, 0x6600_0000);
    s.extend(triangle());
    bp(&mut s, 0x6600_0000);
    another(&mut s);
    f.write(&s, &Ram(&[]));
    assert_eq!(f.gx.take_frame().draws.len(), 2);
}

#[test]
fn a_repeat_decodes_again_when_what_it_reads_changed() {
    // GEN_MODE and XF both hold the channel count: the last write wins,
    // also when it repeats that register's own last value.
    let mut f = GxFifo::default();
    let mut s = Vec::new();
    view_3d(&mut s);
    xf(&mut s, 0x1009, &[1]);
    bp(&mut s, 0x0000_0020); // GEN_MODE: two channels
    xf(&mut s, 0x1009, &[1]);
    s.extend(triangle());
    f.write(&s, &Ram(&[]));
    let with = f.gx.take_frame();
    let mut f = GxFifo::default();
    let mut s = Vec::new();
    view_3d(&mut s);
    bp(&mut s, 0x0000_0020);
    xf(&mut s, 0x1009, &[1]);
    s.extend(triangle());
    f.write(&s, &Ram(&[]));
    let without = f.gx.take_frame();
    assert!(
        std::sync::Arc::ptr_eq(&with.draws[0].material, &without.draws[0].material)
            || format!("{:?}", with.draws[0].material)
                == format!("{:?}", without.draws[0].material)
    );
}
