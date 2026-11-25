//! Host side of the Dolphin OS thread, alarm and interrupt layer, written for
//! this project (behaviour per the GameCube SDK; no emulator code).
//!
//! Threads: the game's own OSThread code runs as recompiled (OSCreateThread,
//! OSResumeThread, OSSuspendThread, OSSleepThread/OSWakeupThread,
//! OSYieldThread, mutexes, message queues, SelectThread's priority pick).
//! Only its two context primitives are replaced:
//! - OSSaveContext(ctx) records that `ctx` belongs to the running thread
//!   (and stores its registers there), and returns 0;
//! - OSLoadContext(ctx) switches to the thread owning `ctx`, starting one
//!   for a context OSInitContext prepared.
//!
//! Each OS thread runs on a host thread of its own, holding its own `Cpu`
//! (GPRs, FPRs, LR, CTR, CR, SP: the saved context). One baton (memory, the
//! call table and the time base) moves between them, so exactly one runs at a
//! time and runs are deterministic. A switched-out thread waits inside
//! OSLoadContext; when its context is loaded again, OSLoadContext returns into
//! SelectThread, which then returns as it would from OSSaveContext.
//!
//! Interrupts: a frame (time, due alarms, frame hooks such as the VI retrace)
//! runs with the scheduler disabled, then reschedules, as an interrupt return
//! would. Frames come from VIWaitForRetrace ([`wait_frame`]), from the
//! scheduler idling (every thread waiting) and from a long spin
//! ([`rt::PREEMPT_EVERY`] loop iterations).

use crate::global::Global;
use crate::rt::{self, Cpu, Fpr, Func, Mem, Runtime};
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender};

/// Where the game's Dolphin OS functions and OSThread.c statics are. Every
/// GameCube game links its own copy of the SDK, so these differ per game;
/// [`OsAddrs::from_symbols`] finds them by their SDK names in a decomp's
/// symbols.txt, or a game's glue fills them in by hand.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OsAddrs {
    pub set_alarm: u32,
    pub set_periodic_alarm: u32,
    pub cancel_alarm: u32,
    /// Boots the DSP and waits for its mail; there is no DSP here, so it is skipped.
    pub init_audio_system: u32,
    pub save_context: u32,
    pub load_context: u32,
    pub init_context: u32,
    pub enable_interrupts: u32,
    pub disable_scheduler: u32,
    pub enable_scheduler: u32,
    pub reschedule: u32,
    pub resume_thread: u32,
    pub suspend_thread: u32,
    pub link: u32,
    pub unlink: u32,
    /// SelectThread's idle context (an OSThread.c static).
    pub idle_context: u32,
    /// The ready-queue bitmap (an OSThread.c static).
    pub run_queue_bits: u32,
    /// OSDisableScheduler's count (the OSThread.c static `Reschedule`).
    pub reschedule_count: u32,
}

impl OsAddrs {
    /// SDK names tried for each field, in order.
    const NAMES: &'static [(&'static str, &'static [&'static str])] = &[
            ("set_alarm", &["OSSetAlarm"]),
            ("set_periodic_alarm", &["OSSetPeriodicAlarm"]),
            ("cancel_alarm", &["OSCancelAlarm"]),
            ("init_audio_system", &["__OSInitAudioSystem", "OSInitAudioSystem"]),
            ("save_context", &["OSSaveContext"]),
            ("load_context", &["OSLoadContext"]),
            ("init_context", &["OSInitContext"]),
            ("enable_interrupts", &["OSEnableInterrupts"]),
            ("disable_scheduler", &["OSDisableScheduler"]),
            ("enable_scheduler", &["OSEnableScheduler"]),
            ("reschedule", &["__OSReschedule", "OSReschedule"]),
            ("resume_thread", &["OSResumeThread"]),
            ("suspend_thread", &["OSSuspendThread"]),
            ("link", &["OSLink"]),
            ("unlink", &["OSUnlink"]),
            ("idle_context", &["IdleContext"]),
            ("run_queue_bits", &["RunQueueBits"]),
            ("reschedule_count", &["Reschedule"]),
    ];

    /// Look every address up by name in a decomp-toolkit symbols.txt. Err
    /// lists the fields no name matched (fill those in by hand).
    pub fn from_symbols(symbols: &str) -> Result<Self, Vec<&'static str>> {
        let by_name: HashMap<String, u32> = crate::dol::parse_symbols(symbols).into_iter().map(|s| (s.name, s.addr)).collect();
        let mut out = OsAddrs::default();
        let mut missing = Vec::new();
        for &(field, names) in Self::NAMES {
            let Some(&addr) = names.iter().find_map(|n| by_name.get(*n)) else {
                missing.push(field);
                continue;
            };
            match field {
            "set_alarm" => out.set_alarm = addr,
            "set_periodic_alarm" => out.set_periodic_alarm = addr,
            "cancel_alarm" => out.cancel_alarm = addr,
            "init_audio_system" => out.init_audio_system = addr,
            "save_context" => out.save_context = addr,
            "load_context" => out.load_context = addr,
            "init_context" => out.init_context = addr,
            "enable_interrupts" => out.enable_interrupts = addr,
            "disable_scheduler" => out.disable_scheduler = addr,
            "enable_scheduler" => out.enable_scheduler = addr,
            "reschedule" => out.reschedule = addr,
            "resume_thread" => out.resume_thread = addr,
            "suspend_thread" => out.suspend_thread = addr,
            "link" => out.link = addr,
            "unlink" => out.unlink = addr,
            "idle_context" => out.idle_context = addr,
            "run_queue_bits" => out.run_queue_bits = addr,
            "reschedule_count" => out.reschedule_count = addr,
                _ => unreachable!(),
            }
        }
        if missing.is_empty() { Ok(out) } else { Err(missing) }
    }
}

static ADDRS: Global<OsAddrs> = crate::global!(OsAddrs::default());

/// The addresses [`install`] was given.
pub fn addrs() -> OsAddrs {
    ADDRS.get()
}

/// __OSCurrentThread and the current OSContext pointer (low memory).
pub const CURRENT_THREAD: u32 = 0x8000_00E4;
pub const CURRENT_CONTEXT: u32 = 0x8000_00D4;

/// Time base runs at bus clock / 4 (162 MHz / 4).
pub const TB_HZ: u64 = 40_500_000;
pub const TB_PER_FRAME: u64 = TB_HZ / 60;
/// Host stack for each game thread but the first (recompiled code nests a
/// Rust frame per guest call). Reserved, not committed.
#[cfg(not(target_family = "wasm"))]
const THREAD_STACK: usize = 256 << 20;
/// In wasm a guest call nests in the engine's own stack; this one (in
/// linear memory, shared by every thread) holds only spilled values.
#[cfg(target_family = "wasm")]
const THREAD_STACK: usize = 16 << 20;

struct Alarm {
    addr: u32,
    handler: u32,
    fire: u64,
    period: u64,
}

#[derive(Default)]
struct State {
    alarms: Vec<Alarm>,
    frame_hooks: Vec<Func>,
    pumps: u64,
    pumping: bool,
    /// Threads suspended in VIWaitForRetrace until the next frame.
    frame_waiters: Vec<u32>,
}

static S: Global<State> = crate::global!(State::default());

/// What a host thread waits for.
enum Msg {
    /// Run: the baton.
    Run(Baton),
    /// Your OSThread was reused; unwind and end.
    Exit,
    /// (To the first thread) another thread panicked with the baton.
    Died(Baton, Box<dyn Any + Send>),
}

struct Baton {
    mem: Mem,
    rt: Runtime,
    tb: u64,
}

#[derive(Default)]
struct Sched {
    /// Host thread index -> its inbox. Index 0 is the thread that booted.
    inbox: Vec<Sender<Msg>>,
    /// OSContext address -> host thread index.
    owner: HashMap<u32, usize>,
    running: usize,
}

static SCHED: Global<Sched> = crate::global!(Sched::default());

thread_local! {
    /// This host thread's index and inbox.
    static ME: RefCell<Option<(usize, Receiver<Msg>)>> = const { RefCell::new(None) };
}

/// Unwinds a host thread whose OS thread is gone.
struct ThreadGone;

/// Register a hook run on every frame (e.g. the VI retrace interrupt). The
/// same hook twice runs once.
pub fn add_frame_hook(f: Func) {
    S.with_borrow_mut(|s| {
        if !s.frame_hooks.iter().any(|h| std::ptr::fn_addr_eq(*h, f)) {
            s.frame_hooks.push(f);
        }
    });
}

pub fn pumps() -> u64 {
    S.with_borrow(|s| s.pumps)
}

/// Install on the host thread that will run the game's entry point.
pub fn install(c: &mut Cpu, os: OsAddrs) {
    ADDRS.set(os);
    c.rt.set_override(os.link, |c, m| relinked(c, m, addrs().link));
    c.rt.set_override(os.unlink, |c, m| relinked(c, m, addrs().unlink));
    S.set(State::default());
    let (tx, rx) = channel();
    SCHED.set(Sched {
        inbox: vec![tx],
        ..Sched::default()
    });
    ME.set(Some((0, rx)));
    c.rt.set_override(os.init_audio_system, |_, _| {});
    c.rt.set_override(os.set_alarm, set_alarm);
    c.rt.set_override(os.set_periodic_alarm, set_periodic_alarm);
    c.rt.set_override(os.cancel_alarm, cancel_alarm);
    c.rt.set_override(os.save_context, save_context);
    c.rt.set_override(os.load_context, load_context);
    c.rt.set_override(os.init_context, init_context);
    c.rt.set_override(os.enable_interrupts, enable_interrupts);
    c.rt.preempt = Some(preempt);
}

fn original(c: &mut Cpu, m: &mut Mem, addr: u32) {
    match (c.rt.generated)(addr) {
        Some(f) => f(c, m),
        None => panic!("rustcube-recomp: no recompiled function at {addr:#010x}"),
    }
}

/// Call a game function if there is one (tests run without the game).
fn guest(c: &mut Cpu, m: &mut Mem, addr: u32, r3: u32) {
    if c.rt.resolve(m, addr).is_some() {
        c.r[3] = r3;
        rt::call(c, m, addr);
    }
}

// ---- context switch ------------------------------------------------------

// OSContext layout.
const CTX_CR: u32 = 0x80;
const CTX_LR: u32 = 0x84;
const CTX_CTR: u32 = 0x88;
const CTX_FPR: u32 = 0x90;
const CTX_FPSCR: u32 = 0x194;
const CTX_SRR0: u32 = 0x198;
const CTX_SRR1: u32 = 0x19C;
const CTX_GQR: u32 = 0x1A4;
const CTX_PSF: u32 = 0x1C8;

fn save_context(c: &mut Cpu, m: &mut Mem) {
    let ctx = c.r[3];
    for i in 0..32 {
        m.w32(ctx + 4 * i, c.r[i as usize]);
        m.wf64(ctx + CTX_FPR + 8 * i, c.f[i as usize].ps0);
        m.wf64(ctx + CTX_PSF + 8 * i, c.f[i as usize].ps1);
    }
    for i in 0..8 {
        m.w32(ctx + CTX_GQR + 4 * i, c.gqr[i as usize]);
    }
    m.w32(ctx + CTX_CR, c.cr);
    m.w32(ctx + CTX_LR, c.lr);
    m.w32(ctx + CTX_CTR, c.ctr);
    m.w32(ctx + CTX_FPSCR, c.fpscr);
    m.w32(ctx + CTX_SRR0, c.lr);
    m.w32(ctx + CTX_SRR1, c.msr);
    let me = my_index();
    SCHED.with_borrow_mut(|s| s.owner.insert(ctx, me));
    c.r[3] = 0;
}

fn load_context(c: &mut Cpu, m: &mut Mem) {
    let ctx = c.r[3];
    let me = my_index();
    let owner = SCHED.with_borrow(|s| s.owner.get(&ctx).copied());
    let to = match owner {
        Some(i) if i == me => return, // picked again: back into SelectThread
        Some(i) => i,
        None => start_thread(c, m, ctx),
    };
    if std::env::var_os("RUSTCUBE_TRACE_THREADS").is_some() {
        eprintln!("thread: switch {me} -> {to} (context {ctx:#010x})");
    }
    handoff(c, m, to);
}

// OSInitContext(ctx, pc, sp): a context reused for a new thread; the host
// thread that owned it (an exited OS thread) ends.
fn init_context(c: &mut Cpu, m: &mut Mem) {
    let ctx = c.r[3];
    let old = SCHED.with_borrow_mut(|s| {
        let i = s.owner.remove(&ctx)?;
        (i != s.running).then(|| s.inbox[i].clone())
    });
    if let Some(tx) = old {
        let _ = tx.send(Msg::Exit);
    }
    original(c, m, addrs().init_context);
}

fn my_index() -> usize {
    ME.with_borrow(|me| me.as_ref().map_or(0, |m| m.0))
}

/// Registers a new thread starts with, read from its OSContext.
struct Start {
    r: [u32; 32],
    f: [Fpr; 32],
    cr: u32,
    lr: u32,
    ctr: u32,
    fpscr: u32,
    msr: u32,
    gqr: [u32; 8],
    spr: Box<[u32; 1024]>,
    pc: u32,
}

/// A host thread for the context OSInitContext prepared at `ctx`.
fn start_thread(c: &Cpu, m: &Mem, ctx: u32) -> usize {
    let s = Start {
        r: std::array::from_fn(|i| m.r32(ctx + 4 * i as u32)),
        f: std::array::from_fn(|i| Fpr {
            ps0: m.rf64(ctx + CTX_FPR + 8 * i as u32),
            ps1: m.rf64(ctx + CTX_PSF + 8 * i as u32),
        }),
        cr: m.r32(ctx + CTX_CR),
        lr: m.r32(ctx + CTX_LR),
        ctr: m.r32(ctx + CTX_CTR),
        fpscr: m.r32(ctx + CTX_FPSCR),
        msr: m.r32(ctx + CTX_SRR1),
        gqr: std::array::from_fn(|i| m.r32(ctx + CTX_GQR + 4 * i as u32)),
        spr: c.spr.clone(),
        pc: m.r32(ctx + CTX_SRR0),
    };
    let (tx, rx) = channel();
    let i = SCHED.with_borrow_mut(|sc| {
        sc.inbox.push(tx);
        sc.owner.insert(ctx, sc.inbox.len() - 1);
        sc.inbox.len() - 1
    });
    if std::env::var_os("RUSTCUBE_TRACE_THREADS").is_some() {
        let live = LIVE.load(std::sync::atomic::Ordering::Relaxed);
        eprintln!(
            "thread: start {i} at {:#010x} (context {ctx:#010x}, {live} host threads live)",
            s.pc
        );
    }
    let spawned = std::thread::Builder::new()
        .name(format!("game thread {i}"))
        .stack_size(THREAD_STACK)
        .spawn(move || thread_main(i, rx, s));
    if let Err(e) = spawned {
        panic!("rustcube-recomp: can't start a host thread for context {ctx:#010x}: {e}");
    }
    i
}

/// Game host threads alive now (besides the first).
pub static LIVE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

struct Alive;
impl Drop for Alive {
    fn drop(&mut self) {
        LIVE.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

fn thread_main(i: usize, rx: Receiver<Msg>, s: Start) {
    LIVE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let _alive = Alive;
    let b = match rx.recv() {
        Ok(Msg::Run(b)) => b,
        _ => return,
    };
    ME.set(Some((i, rx)));
    let mut c = Cpu {
        r: s.r,
        f: s.f,
        cr: s.cr,
        lr: s.lr,
        ctr: s.ctr,
        fpscr: s.fpscr,
        msr: s.msr,
        gqr: s.gqr,
        spr: s.spr,
        tb: b.tb,
        rt: b.rt,
        ..Cpu::default()
    };
    let mut m = b.mem;
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // The entry returns into OSExitThread (the context's LR), which
        // switches away for good.
        let exit = c.lr;
        rt::call(&mut c, &mut m, s.pc);
        if exit != 0 {
            rt::call(&mut c, &mut m, exit);
        }
        panic!("rustcube-recomp: game thread {i} returned past OSExitThread");
    }));
    let Err(p) = r else { unreachable!() };
    if p.is::<ThreadGone>() {
        return;
    }
    // The run is over: hand the baton and the panic to the first thread.
    let b = Baton {
        mem: m,
        rt: std::mem::take(&mut c.rt),
        tb: c.tb,
    };
    let first = SCHED.with_borrow(|s| s.inbox.first().cloned());
    if let Some(tx) = first {
        let _ = tx.send(Msg::Died(b, p));
    }
}

/// Give the baton to host thread `to` and wait to get it back.
fn handoff(c: &mut Cpu, m: &mut Mem, to: usize) {
    let empty = Mem {
        ram: Vec::new(),
        lc: Vec::new(),
        io_read: m.io_read,
        io_write: m.io_write,
    };
    let b = Baton {
        mem: std::mem::replace(m, empty),
        rt: std::mem::take(&mut c.rt),
        tb: c.tb,
    };
    let tx = SCHED.with_borrow_mut(|s| {
        s.running = to;
        s.inbox.get(to).cloned()
    });
    if !tx.is_some_and(|tx| tx.send(Msg::Run(b)).is_ok()) {
        panic!("rustcube-recomp: game thread {to} is gone");
    }
    let msg = ME.with_borrow(|me| me.as_ref().map(|(_, rx)| rx.recv()));
    match msg {
        Some(Ok(Msg::Run(b))) => {
            *m = b.mem;
            c.rt = b.rt;
            c.tb = b.tb;
        }
        Some(Ok(Msg::Died(b, p))) => {
            *m = b.mem;
            c.rt = b.rt;
            c.tb = b.tb;
            std::panic::resume_unwind(p);
        }
        // A normal end: unwind without a panic message.
        _ => std::panic::resume_unwind(Box::new(ThreadGone)),
    }
}

// ---- interrupts ----------------------------------------------------------

fn in_idle(m: &Mem) -> bool {
    m.r32(CURRENT_CONTEXT) == addrs().idle_context && m.r32(addrs().run_queue_bits) == 0
}

/// The next loop iteration preempts again (SelectThread's idle spin: each
/// pass is a frame until a thread is ready).
fn preempt_soon(c: &mut Cpu, m: &Mem) {
    if in_idle(m) {
        c.rt.ticks |= rt::PREEMPT_EVERY - 1;
        ARMED.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Set while an early preempt is meant for the idle spin only.
static ARMED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

// SelectThread's idle loop enables interrupts, then spins until a thread is
// ready: that is when the next interrupt comes.
fn enable_interrupts(c: &mut Cpu, m: &mut Mem) {
    original(c, m, addrs().enable_interrupts);
    if in_idle(m) {
        let r3 = c.r[3];
        pump(c, m);
        c.r[3] = r3;
        preempt_soon(c, m);
    }
}

/// Every `rt::PREEMPT_EVERY` loop iterations: an interrupt (a frame), after
/// which a higher-priority thread it readied runs.
fn preempt(c: &mut Cpu, m: &mut Mem) {
    // An early preempt armed for the idle spin, reached by a thread the last
    // frame woke instead: no frame has passed for it. Firing it ran a second
    // frame right after each idle one (two retraces per game frame, so 30
    // frames a second in the window).
    if ARMED.swap(false, std::sync::atomic::Ordering::Relaxed) && !in_idle(m) {
        return;
    }
    pump(c, m);
    preempt_soon(c, m);
}

/// VIWaitForRetrace: the calling thread sleeps until the next frame while
/// other ready threads run. With none ready, or before threads exist, the
/// frame comes at once.
pub fn wait_frame(c: &mut Cpu, m: &mut Mem) {
    let me = m.r32(CURRENT_THREAD);
    let can_suspend = c.rt.resolve(m, addrs().suspend_thread).is_some();
    let none_ready = m.r32(addrs().run_queue_bits) == 0;
    if me == 0 || none_ready || m.r32(addrs().reschedule_count) != 0 || !can_suspend {
        pump(c, m);
        return;
    }
    S.with_borrow_mut(|s| s.frame_waiters.push(me));
    let saved = (c.r, c.f, c.lr, c.ctr, c.cr);
    guest(c, m, addrs().suspend_thread, me);
    (c.r, c.f, c.lr, c.ctr, c.cr) = saved;
}

fn tick(hi: u32, lo: u32) -> u64 {
    (hi as u64) << 32 | lo as u64
}

// OSSetAlarm(OSAlarm* alarm, OSTime tick, handler): r3, r5:r6, r7
fn set_alarm(c: &mut Cpu, _m: &mut Mem) {
    let a = Alarm {
        addr: c.r[3],
        handler: c.r[7],
        fire: c.tb + tick(c.r[5], c.r[6]),
        period: 0,
    };
    S.with_borrow_mut(|s| {
        s.alarms.retain(|x| x.addr != a.addr);
        s.alarms.push(a);
    });
}

// OSSetPeriodicAlarm(alarm, OSTime start, OSTime period, handler): r3, r5:r6, r7:r8, r9
fn set_periodic_alarm(c: &mut Cpu, _m: &mut Mem) {
    let start = tick(c.r[5], c.r[6]);
    let period = tick(c.r[7], c.r[8]).max(1);
    let a = Alarm {
        addr: c.r[3],
        handler: c.r[9],
        fire: start.max(c.tb),
        period,
    };
    S.with_borrow_mut(|s| {
        s.alarms.retain(|x| x.addr != a.addr);
        s.alarms.push(a);
    });
}

fn cancel_alarm(c: &mut Cpu, _m: &mut Mem) {
    let addr = c.r[3];
    S.with_borrow_mut(|s| s.alarms.retain(|x| x.addr != addr));
}

/// One frame as an interrupt: time advances, due alarms fire, frame hooks
/// run and frame waiters resume, all with the scheduler disabled; then the
/// highest-priority ready thread runs (maybe not the caller). Registers the
/// interrupted code had are kept.
pub fn pump(c: &mut Cpu, m: &mut Mem) {
    if S.with_borrow_mut(|s| std::mem::replace(&mut s.pumping, true)) {
        return; // a frame inside a frame's handlers
    }
    let saved = (c.r, c.f, c.lr, c.ctr, c.cr);
    guest(c, m, addrs().disable_scheduler, 0);
    c.tb += TB_PER_FRAME;
    let now = c.tb;
    let due: Vec<(u32, u32)> = S.with_borrow_mut(|s| {
        s.pumps += 1;
        let mut due = Vec::new();
        s.alarms.retain_mut(|a| {
            if a.fire > now {
                return true;
            }
            due.push((a.addr, a.handler));
            if a.period > 0 {
                while a.fire <= now {
                    a.fire += a.period;
                }
                true
            } else {
                false
            }
        });
        due
    });
    // Handler(OSAlarm*, OSContext*): context pointer is unused by game handlers.
    for (addr, handler) in due {
        c.r[3] = addr;
        c.r[4] = 0;
        rt::call(c, m, handler);
    }
    // Debug: RUSTCUBE_STACK=N prints the guest call stack (saved LRs along the
    // back chain) at pump N, to find what a wait loop is waiting in.
    let at = std::env::var("RUSTCUBE_STACK")
        .ok()
        .and_then(|s| s.parse::<u64>().ok());
    // RUSTCUBE_STACK=stall: once, when 600 pumps go by with no new game frame (the main thread is
    // waiting on something), every thread's stack (as RUSTCUBE_STACK_THREAD=all).
    let stall = std::env::var("RUSTCUBE_STACK").is_ok_and(|v| v.trim() == "stall") && {
        use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
        static SEEN: AtomicU64 = AtomicU64::new(0);
        static SINCE: AtomicU64 = AtomicU64::new(0);
        let f = crate::host::FRAMES.load(Relaxed);
        if SEEN.swap(f, Relaxed) != f { SINCE.store(0, Relaxed); false } else { f > 600 && SINCE.fetch_add(1, Relaxed) + 1 == 600 }
    };
    if stall || (at.is_some() && at == Some(pumps())) {
        // RUSTCUBE_STACK_THREAD=addr: that thread's saved stack instead (its
        // OSContext keeps r1 at +4); =all: every thread on the OS's active
        // list (head at 0x800000DC, linked at +0x2FC), with its state.
        let which = if stall { "all".to_string() } else { std::env::var("RUSTCUBE_STACK_THREAD").unwrap_or_default() };
        let mut stacks: Vec<(String, u32)> = Vec::new();
        if which.trim() == "all" {
            let mut t = m.r32(0x8000_00DC);
            for _ in 0..64 {
                if t == 0 {
                    break;
                }
                let state = m.r16(t.wrapping_add(0x2C8));
                stacks.push((
                    format!(
                        "thread {t:#x} state {state} suspend {} prio {} pc {:#x}",
                        m.r32(t.wrapping_add(0x2CC)) as i32,
                        m.r32(t.wrapping_add(0x2D0)) as i32,
                        m.r32(t.wrapping_add(0x198))
                    ),
                    m.r32(t.wrapping_add(4)),
                ));
                t = m.r32(t.wrapping_add(0x2FC));
            }
        } else {
            let sp = u32::from_str_radix(which.trim().trim_start_matches("0x"), 16)
                .map_or(saved.0[1], |t| m.r32(t.wrapping_add(4)));
            stacks.push(("current".into(), sp));
        }
        for (who, mut sp) in stacks {
            eprint!("stack at pump {} ({who}):", at.unwrap_or(pumps()));
            for _ in 0..24 {
                sp = m.r32(sp);
                if sp == 0 || sp == u32::MAX {
                    break;
                }
                eprint!(" {:#010x}", m.r32(sp + 4));
            }
            eprintln!();
        }
    }
    let hooks: Vec<Func> = S.with_borrow(|s| s.frame_hooks.clone());
    for h in hooks {
        h(c, m);
    }
    let waiters = S.with_borrow_mut(|s| std::mem::take(&mut s.frame_waiters));
    for t in waiters {
        guest(c, m, addrs().resume_thread, t);
    }
    guest(c, m, addrs().enable_scheduler, 0);
    S.with_borrow_mut(|s| s.pumping = false);
    guest(c, m, addrs().reschedule, 0);
    (c.r, c.f, c.lr, c.ctr, c.cr) = saved;
}


/// A module's id and its sections' address ranges (OSModuleInfo: id +0x00,
/// section count +0x0C, section table +0x10 of 8-byte entries: address with
/// the code flag in bit 0, size). Only a linked module's table holds addresses.
fn module_ranges(m: &Mem, info: u32) -> (u32, Vec<(u32, u32)>) {
    let ram = |a: u32| (0x8000_0000..0x8C00_0000).contains(&a);
    if !ram(info) {
        return (0, Vec::new());
    }
    let (count, table) = (m.r32(info + 0x0C).min(32), m.r32(info + 0x10));
    let ranges = if ram(table) {
        (0..count).map(|i| (m.r32(table + 8 * i) & !1, m.r32(table + 8 * i + 4))).filter(|&(a, n)| ram(a) && n != 0).map(|(a, n)| (a, a + n)).collect()
    } else {
        Vec::new()
    };
    (m.r32(info), ranges)
}

thread_local! {
    /// Overrides taken off an unlinked module, by module id: (address, the
    /// start of the section it was in, function) (see relinked).
    static PARKED: std::cell::RefCell<std::collections::HashMap<u32, Vec<(u32, u32, crate::rt::Func)>>> = std::cell::RefCell::new(Default::default());
}

/// The game's own OSLink/OSUnlink, then forget cached calls into modules.
///
/// Overrides are kept by address. One set on a module's function stayed
/// after the module was unlinked, and ran in place of whatever function the
/// next module had at that address: leaving one menu module for another
/// stopped with "access outside RAM at 0x9421fff8" while the menu
/// module made its tasks (2026-11-24). An unlinked module's overrides are
/// taken off; each comes back when the same module is linked with that
/// section at the same address again (the glue sets its hooks once per load
/// address), and waits otherwise (two copies of one module can share an id and load at
/// different places).
fn relinked(c: &mut Cpu, m: &mut Mem, addr: u32) {
    let info = c.r[3];
    let before = if addr == addrs().unlink { module_ranges(m, info) } else { (0, Vec::new()) };
    if let Some(f) = (c.rt.generated)(addr) {
        f(c, m);
    }
    if addr == addrs().unlink {
        let (id, ranges) = before;
        let section = |a: u32| ranges.iter().find(|&&(s, e)| (s..e).contains(&a)).map(|r| r.0);
        let gone: Vec<(u32, u32, crate::rt::Func)> = c.rt.overrides.iter().filter_map(|(a, f)| Some((*a, section(*a)?, *f))).collect();
        for (a, _, _) in &gone {
            c.rt.overrides.remove(a);
        }
        if !gone.is_empty() {
            eprintln!("os: module {id} unlinked; {} overrides taken off with it", gone.len());
            PARKED.with_borrow_mut(|p| {
                let kept = p.entry(id).or_default();
                kept.retain(|k| !gone.iter().any(|g| g.0 == k.0));
                kept.extend(gone);
            });
        }
    } else {
        let (id, ranges) = module_ranges(m, info);
        let same = |a: u32, start: u32| ranges.iter().any(|&(s, e)| s == start && (s..e).contains(&a));
        let back: Vec<(u32, u32, crate::rt::Func)> = PARKED.with_borrow_mut(|p| p.get_mut(&id).map(|k| {
            let (back, wait) = std::mem::take(k).into_iter().partition(|&(a, start, _)| same(a, start));
            *k = wait;
            back
        })).unwrap_or_default();
        let mut n = 0;
        for (a, _, f) in back {
            if !c.rt.overrides.contains_key(&a) {
                c.rt.set_override(a, f);
                n += 1;
            }
        }
        if n != 0 {
            eprintln!("os: module {id} linked at the same place; {n} overrides back");
        }
    }
    c.rt.modules_changed();
}
