//! VI without the video interface: every OS frame is a retrace (the clock
//! moves a field, the retrace count goes up, the post-retrace callback runs),
//! VIWaitForRetrace sleeps the calling thread until the next one, and the
//! setup calls do nothing. VISetNextFrameBuffer is remembered for the
//! host to show.
use crate::{guest_call, Symbols};
use rustcube_recomp::rt::{Cpu, Mem};
use rustcube_recomp::{global, global::Global};


static RETRACES: Global<u32> = global!(0);
static NEXT_FB: Global<u32> = global!(0);
static POST_CB: Global<u32> = global!(0);
static LAST_POLL: Global<u64> = global!(0);

fn nothing(_c: &mut Cpu, _m: &mut Mem) {}

/// The caller sleeps until the next frame (other ready threads run meanwhile).
fn wait_for_retrace(c: &mut Cpu, m: &mut Mem) {
    rustcube_recomp::os::wait_frame(c, m);
}

/// Every OS frame (a frame hook): the retrace interrupt.
fn retrace(c: &mut Cpu, m: &mut Mem) {
    // pump() already moved the time base one frame; moving it here too ran
    // the game clock at twice real time, so the game skipped every other
    // draw to catch up (30 frames a second, movement in double steps).
    let n = RETRACES.get().wrapping_add(1);
    RETRACES.set(n);
    let cb = POST_CB.get();
    if cb != 0 {
        guest_call(c, m, cb, &[n]);
    }
}

fn set_post_retrace_callback(c: &mut Cpu, _m: &mut Mem) {
    c.r[3] = POST_CB.replace(c.r[3]);
}

fn set_next_frame_buffer(c: &mut Cpu, _m: &mut Mem) {
    NEXT_FB.set(c.r[3]);
}

fn get_retrace_count(c: &mut Cpu, m: &mut Mem) {
    // Polled back to back (no other call in between): the game is waiting for
    // the next retrace, so wait for one.
    let calls = rustcube_recomp::rt::CALLS.load(std::sync::atomic::Ordering::Relaxed);
    if calls.wrapping_sub(LAST_POLL.replace(calls)) <= 2 {
        let saved = (c.r, c.lr, c.ctr, c.cr);
        wait_for_retrace(c, m);
        (c.r, c.lr, c.ctr, c.cr) = saved;
    }
    c.r[3] = RETRACES.get();
}

fn zero(c: &mut Cpu, _m: &mut Mem) {
    c.r[3] = 0;
}

/// The frame buffer the game asked to show next.
pub fn next_frame_buffer() -> u32 {
    NEXT_FB.get()
}

pub fn retrace_count() -> u32 {
    RETRACES.get()
}

pub fn install(c: &mut Cpu, _m: &mut Mem, syms: &Symbols, missing: &mut Vec<&'static str>) {
    for name in ["VIInit", "VIConfigure", "VIFlush", "VISetBlack"] {
        syms.hook(c, name, nothing, missing);
    }
    syms.hook(c, "VIWaitForRetrace", wait_for_retrace, missing);
    rustcube_recomp::os::add_frame_hook(retrace);
    syms.hook(c, "VISetPostRetraceCallback", set_post_retrace_callback, missing);
    syms.hook(c, "VISetNextFrameBuffer", set_next_frame_buffer, missing);
    syms.hook(c, "VIGetRetraceCount", get_retrace_count, missing);
    // NTSC, no component cable.
    syms.hook(c, "VIGetTvFormat", zero, missing);
    syms.hook(c, "VIGetDTVStatus", zero, missing);
}
