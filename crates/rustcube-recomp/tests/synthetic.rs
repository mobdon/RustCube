#![allow(unused_assignments, unreachable_code, unused_variables, unused_mut, clippy::all)]
mod g {
    include!(concat!(env!("OUT_DIR"), "/synthetic.rs"));
}
use rustcube_recomp::rt::{Cpu, Func, Mem};

fn dispatch(a: u32) -> Option<Func> {
    match a {
        0x8000_3000 => Some(g::f_80003000),
        0x8000_3040 => Some(g::f_80003040),
        0x8000_3048 => Some(g::f_80003048),
        _ => None,
    }
}

fn setup() -> (Cpu, Mem) {
    let mut c = Cpu::default();
    c.rt.generated = dispatch;
    c.r[1] = 0x8100_0000;
    (c, Mem::default())
}

#[test]
fn loop_call_and_stack() {
    let (mut c, mut m) = setup();
    c.lr = 0xDEAD_BEEF;
    rustcube_recomp::rt::call(&mut c, &mut m, 0x8000_3000);
    assert_eq!(c.r[3], 110); // (10 + 9 + ... + 1) * 2
    assert_eq!(c.r[1], 0x8100_0000);
    assert_eq!(c.lr, 0xDEAD_BEEF);
    assert_eq!(c.ctr, 0);
}

#[test]
fn override_replaces_function() {
    let (mut c, mut m) = setup();
    c.rt.set_override(0x8000_3040, |c, _| c.r[3] = 7);
    rustcube_recomp::rt::call(&mut c, &mut m, 0x8000_3000);
    assert_eq!(c.r[3], 7);
}

#[test]
fn single_float_round_trip() {
    let (mut c, mut m) = setup();
    m.w32(0x8000_8000, 1.5f32.to_bits());
    c.r[3] = 0x8000_8000;
    rustcube_recomp::rt::call(&mut c, &mut m, 0x8000_3048);
    assert_eq!(f32::from_bits(m.r32(0x8000_8004)), 3.0);
    assert_eq!(c.f[1].ps1, 3.0);
}

#[test]
fn layout_splits_at_call_and_exit_targets() {
    use rustcube_recomp::gen::layout;
    // 0x00: bl 0x10 (a ladder entry with no symbol); 0x04: blr
    // 0x08: b 0x14 (leaves [0x08, 0x10) once 0x10 is a start); 0x0C: blr
    // 0x10..0x18: plain words, 0x14 reached only from the branch at 0x08.
    let code = [0x4800_0011u32, 0x4E80_0020, 0x4800_000C, 0x4E80_0020, 0x6000_0000, 0x6000_0000];
    let word = |a: u32| code.get(a as usize / 4).copied();
    let starts = [(0, 0), (8, 0)].into_iter().collect();
    let funcs = layout(starts, &[(0, 0x18)], word, |_| false);
    assert_eq!(funcs, vec![(0, 8), (8, 0x10), (0x10, 0x14), (0x14, 0x18)]);
    // A relocated instruction's branch field is not a target.
    let starts = [(0, 0)].into_iter().collect();
    assert_eq!(layout(starts, &[(0, 0x18)], word, |pc| pc == 0 || pc == 8), vec![(0, 0x18)]);
}

#[test]
fn madd_matches_fused_mul_add() {
    use rustcube_recomp::rt::madd;
    let mut s = 0x1234_5678_9abc_def0u64;
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    let special = [0.0, -0.0, 1.0, f64::INFINITY, f64::NEG_INFINITY, f64::NAN, f32::MIN_POSITIVE as f64, 1e-45, f32::MAX as f64];
    for i in 0..200_000 {
        let pick = |r: u64, i: usize| match i % 4 {
            0 => special[(r % special.len() as u64) as usize],
            1 => f64::from_bits(r),
            _ => f32::from_bits(r as u32) as f64,
        };
        let (a, c, b) = (pick(next(), i), pick(next(), i / 4), pick(next(), i / 16));
        let (x, y) = (madd(a, c, b), a.mul_add(c, b));
        assert!(x.to_bits() == y.to_bits() || (x.is_nan() && y.is_nan()), "{a:e} {c:e} {b:e}");
    }
}
