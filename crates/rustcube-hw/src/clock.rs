//! OSGetTime/OSGetTick from `Cpu.tb`. The time base moves one frame at each
//! frame (`rustcube_recomp::os::pump`) and a microsecond per read, so code that polls
//! the clock inside a frame still sees it move. Runs are repeatable.
use crate::Symbols;
use rustcube_recomp::rt::{Cpu, Mem};

/// Bus clock / 4.
pub const TB_HZ: u64 = 40_500_000;
/// One NTSC field (59.94 Hz).
pub const TB_PER_FRAME: u64 = TB_HZ * 1001 / 60_000;
const TB_PER_READ: u64 = TB_HZ / 1_000_000;
/// OSSystemTimeAdjust (u64), added by __OSGetSystemTime.
const SYSTEM_TIME_ADJUST: u32 = 0x8000_30D8;

fn read(c: &mut Cpu) -> u64 {
    c.tb += TB_PER_READ;
    c.tb
}

fn get_time(c: &mut Cpu, _m: &mut Mem) {
    let t = read(c);
    c.r[3] = (t >> 32) as u32;
    c.r[4] = t as u32;
}

fn get_tick(c: &mut Cpu, _m: &mut Mem) {
    c.r[3] = read(c) as u32;
}

fn get_system_time(c: &mut Cpu, m: &mut Mem) {
    let t = read(c).wrapping_add(m.r64(SYSTEM_TIME_ADJUST));
    c.r[3] = (t >> 32) as u32;
    c.r[4] = t as u32;
}

pub fn install(c: &mut Cpu, _m: &mut Mem, syms: &Symbols, missing: &mut Vec<&'static str>) {
    syms.hook(c, "OSGetTime", get_time, missing);
    syms.hook(c, "OSGetTick", get_tick, missing);
    syms.hook(c, "__OSGetSystemTime", get_system_time, missing);
}
