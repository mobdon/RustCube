//! GX: stores to the write-gather pipe (0xCC008000) feed an `rustcube_gx`
//! [`GxFifo`] on a thread of its own, one frame behind the game. The pipe's
//! store hook can't see memory, so bytes queue up; at GXCopyDisp (the end of
//! a frame) the queue and a copy of RAM (vertex arrays, textures, called
//! display lists) go to the GX thread, and the game runs on while it decodes.
//! shortcut: RAM is read as it is at the end of the frame, not at each draw;
//! a game that rewrites vertex data mid-frame would show the later data.
use crate::{io, original, Symbols};
use rustcube_gx::api::{Frame, Ram};
use rustcube_gx::fifo::GxFifo;
use rustcube_recomp::rt::{Cpu, Mem};
use rustcube_recomp::{global, global::Global};
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::Mutex;

/// The write-gather pipe.
pub const PIPE: u32 = 0xCC00_8000;

/// The game's GXCopyDisp (wrapped, then run as recompiled).
static COPY_DISP: Global<u32> = global!(0);

/// Pipe bytes of the frame being built.
static QUEUED: Global<Vec<u8>> = global!(Vec::new());

/// The GX state machine (owned by the GX thread while it decodes).
static FIFO: Mutex<Option<GxFifo>> = Mutex::new(None);
/// Decoded frames, with the XFB each was copied to.
static FRAMES: Mutex<Vec<(u32, Frame)>> = Mutex::new(Vec::new());
/// RAM copies to reuse (no 24 MB allocation per frame).
static SPARE: Mutex<Vec<Vec<u8>>> = Mutex::new(Vec::new());

struct Job {
    fifo: Vec<u8>,
    ram: Vec<u8>,
    dest: u32,
}

/// To the GX thread. Room for one frame: when it is a frame behind, the game
/// waits, so decode and game overlap but never drift apart.
static WORK: Mutex<Option<SyncSender<Job>>> = Mutex::new(None);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn pipe_write(_a: u32, size: u32, v: u32) {
    let b = v.to_be_bytes();
    let n = (size as usize).min(4);
    QUEUED.with_borrow_mut(|q| q.extend_from_slice(&b[4 - n..]));
}

fn decode(job: Job) {
    let t = std::time::Instant::now();
    let frame = {
        let mut fifo = lock(&FIFO);
        let fifo = fifo.get_or_insert_with(GxFifo::default);
        fifo.write(&job.fifo, &Ram(&job.ram));
        fifo.gx.take_frame()
    };
    crate::prof::add(&crate::prof::GX, t);
    lock(&FRAMES).push((job.dest, frame));
    lock(&SPARE).push(job.ram);
}

fn worker() -> SyncSender<Job> {
    let (tx, rx) = sync_channel::<Job>(1);
    let spawned = std::thread::Builder::new()
        .name("gx".into())
        .spawn(move || {
            for job in rx {
                decode(job);
            }
        });
    if spawned.is_err() {
        eprintln!("gx: no decode thread; decoding on the game thread");
    }
    tx
}

/// GXFlush/GXDrawDone: the bytes stay queued until the frame ends.
fn flush(_c: &mut Cpu, _m: &mut Mem) {}

fn copy_disp(c: &mut Cpu, m: &mut Mem) {
    let dest = c.r[3];
    original(c, m, COPY_DISP.get());
    let fifo = QUEUED.with_borrow_mut(std::mem::take);
    let mut ram = lock(&SPARE).pop().unwrap_or_default();
    ram.clear();
    ram.extend_from_slice(&m.ram);
    let job = Job { fifo, ram, dest };
    let tx = lock(&WORK).get_or_insert_with(worker).clone();
    if let Err(e) = tx.send(job) {
        decode(e.0); // no thread: decode here
    }
}

/// Frames finished since the last call, with the XFB each was copied to.
pub fn take_frames() -> Vec<(u32, Frame)> {
    std::mem::take(&mut *lock(&FRAMES))
}

pub fn install(c: &mut Cpu, m: &mut Mem, syms: &Symbols, missing: &mut Vec<&'static str>) {
    io::route_write(m, PIPE, PIPE + 8, pipe_write);
    syms.hook(c, "GXFlush", flush, missing);
    syms.hook(c, "GXDrawDone", flush, missing);
    if let Some(a) = syms.hook(c, "GXCopyDisp", copy_disp, missing) {
        COPY_DISP.set(a);
    }
}

/// GX commands and registers the state machine ignored so far, with counts.
pub fn unhandled() -> std::collections::BTreeMap<String, u64> {
    lock(&FIFO)
        .as_ref()
        .map(|f| f.gx.unhandled())
        .unwrap_or_default()
}

static REPORTED: Global<std::collections::BTreeSet<String>> = global!(Default::default());

/// Unhandled GX entries not returned by an earlier call (for logging).
pub fn unhandled_new() -> Vec<String> {
    let all = unhandled();
    REPORTED.with_borrow_mut(|r| all.into_keys().filter(|k| r.insert(k.clone())).collect())
}
