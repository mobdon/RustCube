//! RUSTCUBE_PROFILE: once a second, frames per second and where a frame's time
//! went (GX decode, the host's frame hook, the window's render; the rest of
//! the game thread's time is the recompiled code itself).
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::Mutex;
use std::time::Instant;

pub static GX: AtomicU64 = AtomicU64::new(0);
pub static HOOK: AtomicU64 = AtomicU64::new(0);
pub static RENDER: AtomicU64 = AtomicU64::new(0);
/// The window limiter sleeping to hold 60 frames a second.
pub static SLEEP: AtomicU64 = AtomicU64::new(0);
/// Pictures the window presented (more than game frames when it blends).
pub static PRESENTS: AtomicU64 = AtomicU64::new(0);
/// Per presented picture: blending, waiting for a swapchain image, recording
/// the frame, the GPU's own time (RUSTCUBE_PROFILE_GPU) and presenting.
pub static TWEEN: AtomicU64 = AtomicU64::new(0);
pub static ACQUIRE: AtomicU64 = AtomicU64::new(0);
pub static DRAW: AtomicU64 = AtomicU64::new(0);
pub static GPU: AtomicU64 = AtomicU64::new(0);
pub static PRESENT: AtomicU64 = AtomicU64::new(0);
static FRAMES: AtomicU64 = AtomicU64::new(0);
/// Frame hook calls: emulated retraces (60 a second of game time).
static RETRACES: AtomicU64 = AtomicU64::new(0);
static SINCE: Mutex<Option<Instant>> = Mutex::new(None);

pub fn on() -> bool {
    std::env::var_os("RUSTCUBE_PROFILE").is_some()
}

/// Add the time since `t` to `counter`.
pub fn add(counter: &AtomicU64, t: Instant) {
    counter.fetch_add(t.elapsed().as_nanos() as u64, Relaxed);
}

/// Count `n` game frames; print and reset each second.
pub fn frames(n: u64) {
    FRAMES.fetch_add(n, Relaxed);
    RETRACES.fetch_add(1, Relaxed);
    let Ok(mut since) = SINCE.lock() else { return };
    let start = *since.get_or_insert_with(Instant::now);
    let secs = start.elapsed().as_secs_f64();
    if secs < 1.0 {
        return;
    }
    let f = FRAMES.swap(0, Relaxed).max(1) as f64;
    let r = RETRACES.swap(0, Relaxed) as f64 / secs;
    let presents = PRESENTS.swap(0, Relaxed);
    let shown = presents as f64 / secs;
    let pic = |c: &AtomicU64| c.swap(0, Relaxed) as f64 / 1e6 / presents.max(1) as f64;
    let (tw, acq, draw, gpu, pres) = (
        pic(&TWEEN),
        pic(&ACQUIRE),
        pic(&DRAW),
        pic(&GPU),
        pic(&PRESENT),
    );
    if presents > 0 {
        eprintln!(
            "prof window: {shown:.0} pictures/s, per picture: blend {tw:.2}, acquire {acq:.2}, record {draw:.2}, gpu {gpu:.2}, present {pres:.2} ms"
        );
    }
    let ms = |c: &AtomicU64| c.swap(0, Relaxed) as f64 / 1e6 / f;
    let (gx, hook, render, sleep) = (ms(&GX), ms(&HOOK), ms(&RENDER), ms(&SLEEP));
    let total = secs * 1000.0 / f;
    eprintln!(
        "prof: {:.1} fps ({r:.0} retraces/s, window shows {shown:.0}/s) | {total:.1} ms/frame: game {:.1}, gx decode {gx:.1} (own thread), frame hook {hook:.1}, limiter sleep {sleep:.1} | window render {render:.1} (own thread)",
        f / secs,
        (total - hook - sleep).max(0.0)
    );
    *since = Some(Instant::now());
}
