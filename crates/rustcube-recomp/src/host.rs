//! Headless host: load main.dol into memory, wire the recompiled dispatch and
//! REL modules, stub the hardware, call the entry point and run until the
//! first panic, the call budget or the time limit. A game's own host
//! crate links the generated code in and calls [`run`].

use crate::dol::{self, Dol};
use crate::rt::{self, Cpu, Func, Mem, Runtime, CALLS};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::AtomicU64;
use std::sync::Mutex;

/// Frames the game has submitted (counted by the setup hook, e.g. from GX).
pub static FRAMES: AtomicU64 = AtomicU64::new(0);
use std::time::Duration;

/// Hardware registers touched so far: address -> (reads, writes).
static IO: Mutex<BTreeMap<u32, (u64, u64)>> = Mutex::new(BTreeMap::new());
/// Most recent hardware access, for a stuck report.
static LAST_IO: Mutex<Option<(u32, bool)>> = Mutex::new(None);

/// Last value written to each hardware register.
static REGS: Mutex<BTreeMap<u32, u32>> = Mutex::new(BTreeMap::new());

fn io_read(a: u32, _size: u32) -> u32 {
    outside_ram(a);
    io_touch(a, false);
    let v = REGS.lock().unwrap_or_else(|e| e.into_inner()).get(&a).copied().unwrap_or(0);
    match a {
        // DSP control: reset, ARAM/DSP DMA and DSP-init busy bits always
        // clear, DSP interrupt (0x20) always raised.
        0xCC00_500A => (v & !0x0E01) | 0x0020,
        // DSP->CPU mailbox: init both drains it (waits for empty) and waits for
        // the DSP's reply (full). With no DSP, alternate so either poll ends.
        0xCC00_5004 => {
            static FLIP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if FLIP.fetch_xor(true, Relaxed) { v | 0x8000 } else { v & !0x8000 }
        }
        // CPU->DSP mailbox: the DSP has always taken the last mail.
        0xCC00_5000 => v & !0x8000,
        // Transfers finish at once: EXI channel control (TSTART), SI
        // communication control, DI control.
        0xCC00_6800..=0xCC00_6837 if (a - 0xCC00_6800) % 0x14 == 0xC => v & !1,
        0xCC00_6434 | 0xCC00_6018 => v & !1,
        _ => v,
    }
}

fn io_write(a: u32, _size: u32, v: u32) {
    outside_ram(a);
    io_touch(a, true);
    REGS.lock().unwrap_or_else(|e| e.into_inner()).insert(a, v);
}

fn io_touch(a: u32, write: bool) {
    let mut io = IO.lock().unwrap_or_else(|e| e.into_inner());
    let first = !io.contains_key(&a);
    let e = io.entry(a).or_default();
    if write {
        e.1 += 1;
    } else {
        e.0 += 1;
    }
    *LAST_IO.lock().unwrap_or_else(|e| e.into_inner()) = Some((a, write));
    if first {
        eprintln!("io: first {} {a:#010x}", if write { "write" } else { "read" });
    }
}

/// Boot globals the IPL and apploader leave in low memory before the DOL runs
/// (GameCube boot conventions: magic, memory size, console type, clocks).
fn boot_globals(m: &mut Mem) {
    m.w32(0x8000_0020, 0x0D15_EA5E);
    m.w32(0x8000_0024, 1);
    m.w32(0x8000_0028, rt::mem1());
    m.w32(0x8000_002C, 0x1000_0006); // retail console
    m.w32(0x8000_00CC, 0); // NTSC
    m.w32(0x8000_00F0, rt::mem1());
    m.w32(0x8000_00F8, 162_000_000); // bus clock
    m.w32(0x8000_00FC, 486_000_000); // CPU clock
}

/// Run main.dol from its entry point and print where it stopped: a panic, the
/// call budget, or still running after `secs` (then the process exits).
pub fn run(dol_bytes: &[u8], symbols: &str, dispatch: fn(u32) -> Option<Func>, register: fn(&mut Runtime), setup: fn(&mut Cpu, &mut Mem), budget: u64, secs: u64) -> Result<(), String> {
    let dol = Dol::parse(dol_bytes)?;
    let entry = dol.entry;
    let os = crate::os::OsAddrs::from_symbols(symbols).map_err(|m| format!("symbols.txt has no address for {m:?}"))?;
    let (tx, rx) = std::sync::mpsc::channel();
    // Recompiled code nests a Rust frame per guest call; give it room.
    std::thread::Builder::new()
        .stack_size(if cfg!(target_family = "wasm") { 64 << 20 } else { 1 << 30 })
        .spawn(move || {
            let mut m = Mem { io_read, io_write, ..Mem::default() };
            dol.load(&mut m);
            boot_globals(&mut m);
            let mut c = Cpu::default();
            c.rt.generated = dispatch;
            c.rt.budget = budget;
            register(&mut c.rt);
            crate::os::install(&mut c, os);
            setup(&mut c, &mut m);
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| rt::call(&mut c, &mut m, entry)));
            let msg = match r {
                Ok(()) => "entry point returned".to_string(),
                Err(p) => p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default(),
            };
            if std::env::var_os("RUSTCUBE_REGS").is_some() {
                let r: Vec<String> = c.r.iter().enumerate().map(|(i, v)| format!("r{i}={v:08x}")).collect();
                eprintln!("regs: {} lr={:08x} ctr={:08x} cr={:08x}", r.join(" "), c.lr, c.ctr, c.cr);
                eprintln!("gqr: {:08x?}", c.gqr);
            }
            // Debug: RUSTCUBE_PEEK=addr,... prints each word and the 8 words it points at.
            for a in std::env::var("RUSTCUBE_PEEK").unwrap_or_default().split(',').filter_map(|s| u32::from_str_radix(s.trim().trim_start_matches("0x"), 16).ok()) {
                let v = m.r32(a);
                let at: Vec<String> = (0..8).map(|i| format!("{:08x}", m.r32(v.wrapping_add(4 * i)))).collect();
                eprintln!("peek {a:#010x} = {v:#010x} -> [{}]", at.join(" "));
            }
            let _ = tx.send(msg);
        })
        .map_err(|e| e.to_string())?;
    let stopped = match rx.recv_timeout(Duration::from_secs(secs)) {
        Ok(msg) => format!("stopped: {msg}"),
        Err(_) => {
            let io = *LAST_IO.lock().unwrap_or_else(|e| e.into_inner());
            format!("still running after {secs}s (stuck in a loop without calls?); last hardware access {io:x?}")
        }
    };
    print(symbols, &stopped);
    Ok(())
}

fn print(symbols: &str, stopped: &str) {
    let names: BTreeMap<u32, String> =
        dol::parse_symbols(symbols).into_iter().filter(|s| s.function).map(|s| (s.addr, s.name)).collect();
    let name = |a: u32| match names.range(..=a).next_back() {
        Some((s, n)) if *s == a => n.clone(),
        Some((s, n)) => format!("{n}+{:#x}", a - s),
        None => "?".into(),
    };
    println!("calls {} | frames {}", CALLS.load(Relaxed), FRAMES.load(Relaxed));
    println!("last calls (oldest first):");
    for a in rt::trace() {
        println!("  {a:#010x} {}", name(a));
    }
    println!("{stopped}");
    let io = IO.lock().unwrap_or_else(|e| e.into_inner());
    let regions: BTreeSet<u32> = io.keys().map(|a| a & !0xFFF).collect();
    println!("hardware addresses touched: {} in {} 4K regions {:x?}", io.len(), regions.len(), regions);
}

/// Hardware lives at 0xCC00xxxx (and the EFB at 0xC8xxxxxx); anything else
/// that isn't RAM is a wild pointer: stop there instead of logging it.
fn outside_ram(a: u32) {
    if !matches!(a >> 24, 0xCC | 0xCD | 0xC8) {
        panic!("access outside RAM and hardware at {a:#010x}");
    }
}
