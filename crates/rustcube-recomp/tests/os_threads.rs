//! The OS context switch without the game: threads made of overrides switch
//! through OSSaveContext/OSLoadContext as SelectThread does, each keeping its
//! own registers, one running at a time.
use rustcube_recomp::os::{self, OsAddrs};
use rustcube_recomp::rt::{call, Cpu, Func, Mem};
use std::sync::Mutex;

/// The scheduler is process-wide: one test at a time.
static LOCK: Mutex<()> = Mutex::new(());

/// Made-up addresses for the OS functions, found by name as a game's would be.
const SYMBOLS: &str = "\
OSSaveContext = .text:0x80F00000; // type:function size:0x80 scope:global
OSLoadContext = .text:0x80F00100; // type:function size:0x104 scope:global
OSInitContext = .text:0x80F00200; // type:function size:0x40 scope:global
OSSetAlarm = .text:0x80F00300; // type:function scope:global
OSSetPeriodicAlarm = .text:0x80F00400; // type:function scope:global
OSCancelAlarm = .text:0x80F00500; // type:function scope:global
__OSInitAudioSystem = .text:0x80F00600; // type:function scope:global
OSEnableInterrupts = .text:0x80F00700; // type:function scope:global
OSDisableScheduler = .text:0x80F00800; // type:function scope:global
OSEnableScheduler = .text:0x80F00900; // type:function scope:global
__OSReschedule = .text:0x80F00A00; // type:function scope:global
OSResumeThread = .text:0x80F00B00; // type:function scope:global
OSSuspendThread = .text:0x80F00C00; // type:function scope:global
OSLink = .text:0x80F00D00; // type:function scope:global
OSUnlink = .text:0x80F00E00; // type:function scope:global
IdleContext = .bss:0x80F00F00; // type:object size:0x2C8 scope:local
RunQueueBits = .sbss:0x80F01000; // type:object size:0x4 scope:local
Reschedule = .sbss:0x80F01100; // type:object size:0x4 scope:local
";
const OS_SAVE_CONTEXT: u32 = 0x80F0_0000;
const OS_LOAD_CONTEXT: u32 = 0x80F0_0100;
const OS_INIT_CONTEXT: u32 = 0x80F0_0200;

const MAIN_CTX: u32 = 0x8000_2000;
const T_CTX: u32 = 0x8000_3000;
const ENTRY: u32 = 0x8100_0000;
const LOG: u32 = 0x8000_1000;
const STACK: u32 = 0x8010_0000;

fn log(m: &mut Mem, v: u32) {
    let n = m.r32(LOG);
    m.w32(LOG + 4 + 4 * n, v);
    m.w32(LOG, n + 1);
}

fn logged(m: &Mem) -> Vec<u32> {
    (0..m.r32(LOG)).map(|i| m.r32(LOG + 4 + 4 * i)).collect()
}

/// SelectThread's switch: save `from`, load `to`.
fn switch(c: &mut Cpu, m: &mut Mem, from: u32, to: u32) {
    c.r[3] = from;
    call(c, m, OS_SAVE_CONTEXT);
    assert_eq!(c.r[3], 0);
    c.r[3] = to;
    call(c, m, OS_LOAD_CONTEXT);
}

fn setup() -> (Cpu, Mem) {
    let mut c = Cpu::default();
    let m = Mem::default();
    // The "recompiled" OSInitContext the OS layer wraps.
    c.rt.generated = |a| (a == OS_INIT_CONTEXT).then_some(init_context as Func);
    os::install(&mut c, OsAddrs::from_symbols(SYMBOLS).expect("every OS symbol"));
    (c, m)
}

/// OSInitContext(ctx, pc, sp): the parts the switch reads.
fn init_context(c: &mut Cpu, m: &mut Mem) {
    let ctx = c.r[3];
    for o in (0..0x2C8).step_by(4) {
        m.w32(ctx + o, 0);
    }
    m.w32(ctx + 0x198, c.r[4]);
    m.w32(ctx + 4, c.r[5]);
}

/// A new thread for ENTRY with `param`, as OSCreateThread prepares it.
fn create(c: &mut Cpu, m: &mut Mem, param: u32) {
    let saved = c.r;
    c.r[3..6].copy_from_slice(&[T_CTX, ENTRY, STACK]);
    call(c, m, OS_INIT_CONTEXT);
    m.w32(T_CTX + 4 * 3, param);
    c.r = saved;
}

fn worker(c: &mut Cpu, m: &mut Mem) {
    assert_eq!(c.r[1], STACK);
    log(m, c.r[3]); // its param
    c.r[20] = 0x2222;
    switch(c, m, T_CTX, MAIN_CTX);
    assert_eq!(c.r[20], 0x2222, "the worker's own registers survive");
    log(m, 3);
    switch(c, m, T_CTX, MAIN_CTX);
    unreachable!("nothing loads the worker again");
}

#[test]
fn threads_switch_and_keep_their_registers() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (mut c, mut m) = setup();
    c.rt.set_override(ENTRY, worker);
    create(&mut c, &mut m, 1);
    c.r[20] = 0x1111;
    switch(&mut c, &mut m, MAIN_CTX, T_CTX);
    assert_eq!(c.r[20], 0x1111, "main's registers survive");
    log(&mut m, 2);
    switch(&mut c, &mut m, MAIN_CTX, T_CTX);
    log(&mut m, 4);
    assert_eq!(logged(&m), [1, 2, 3, 4]);
    // Saving stored main's registers in its context.
    assert_eq!(m.r32(MAIN_CTX + 4 * 20), 0x1111);
}

#[test]
fn loading_your_own_context_returns() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (mut c, mut m) = setup();
    switch(&mut c, &mut m, MAIN_CTX, MAIN_CTX);
    log(&mut m, 9);
    assert_eq!(logged(&m), [9]);
}

#[test]
fn reused_thread_context_starts_fresh() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (mut c, mut m) = setup();
    c.rt.set_override(ENTRY, worker);
    create(&mut c, &mut m, 1);
    switch(&mut c, &mut m, MAIN_CTX, T_CTX);
    // The worker is parked mid-way; OSCreateThread reuses its OSThread.
    create(&mut c, &mut m, 7);
    switch(&mut c, &mut m, MAIN_CTX, T_CTX);
    assert_eq!(logged(&m), [1, 7]);
}

#[test]
fn a_crash_in_a_thread_reaches_the_host_with_memory() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (mut c, mut m) = setup();
    c.rt.set_override(ENTRY, |_, m| {
        log(m, 5);
        panic!("worker broke");
    });
    create(&mut c, &mut m, 0);
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        switch(&mut c, &mut m, MAIN_CTX, T_CTX);
    }));
    let p = r.expect_err("the worker's panic comes back");
    assert_eq!(p.downcast_ref::<&str>().copied(), Some("worker broke"));
    assert_eq!(logged(&m), [5]);
}

#[test]
fn missing_symbols_are_named() {
    let missing = OsAddrs::from_symbols("OSSaveContext = .text:0x80001000; // type:function").unwrap_err();
    assert!(missing.contains(&"load_context") && !missing.contains(&"save_context"));
}
