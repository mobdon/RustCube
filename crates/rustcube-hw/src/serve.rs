//! Server mode: another process (a test, a tool, a game server) drives the headless game one frame at a time
//! over stdin/stdout. Enabled by `RUSTCUBE_SERVE` (the host's frame hook calls
//! [`hook`]); the driver makes every game-logic decision through the game's
//! own recompiled code, and only reads, writes and calls at frame boundaries.
//!
//! One text line per command, one reply line each (`ok ...` or `err ...`):
//!
//! - `frame [N]`: let the game run N frames (default 1); replies `ok <pumps>`
//!   once the next N-th frame hook is reached.
//! - `read ADDR LEN`: `ok <hex bytes>` (LEN at most 64 KiB).
//! - `write ADDR HEX`: writes the bytes.
//! - `call ADDR [ARG...]`: runs the guest function with args in r3.. and
//!   replies `ok <r3>` (hex). Used for the game's own routines, e.g. a checksum.
//! - `sec MODULE SECTION`: `ok <address>` of a linked REL section, `err` if
//!   that module is not linked now.
//! - `quit`: exits the process.
//!
//! Numbers are hex with or without `0x`.
use crate::guest_call;
use rustcube_recomp::rt::{self, Cpu, Mem};
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

/// Frames still to run before the next command is read.
static LEFT: AtomicU64 = AtomicU64::new(0);

/// True when `RUSTCUBE_SERVE` is set.
pub fn enabled() -> bool {
    std::env::var_os("RUSTCUBE_SERVE").is_some()
}

fn num(s: Option<&str>) -> Result<u32, String> {
    let s = s.ok_or("missing number")?;
    u32::from_str_radix(s.trim_start_matches("0x"), 16).map_err(|e| format!("{s}: {e}"))
}

fn hex(bytes: impl IntoIterator<Item = u8>) -> String {
    bytes.into_iter().map(|b| format!("{b:02x}")).collect()
}

/// One command. `Ok(Some(n))` resumes the game for n frames.
fn command(c: &mut Cpu, m: &mut Mem, line: &str) -> Result<(Option<u64>, String), String> {
    let mut w = line.split_whitespace();
    let reply = match w.next() {
        Some("frame") => {
            let n = w
                .next()
                .map(|s| s.parse::<u64>().map_err(|e| e.to_string()))
                .transpose()?
                .unwrap_or(1);
            return Ok((Some(n.max(1)), String::new()));
        }
        Some("read") => {
            let (a, n) = (num(w.next())?, num(w.next())?);
            if n > 0x1_0000 {
                return Err("read longer than 64 KiB".into());
            }
            hex((0..n).map(|i| m.r8(a.wrapping_add(i))))
        }
        Some("write") => {
            let a = num(w.next())?;
            let s = w.next().ok_or("missing bytes")?;
            if s.len() % 2 != 0 {
                return Err("odd hex length".into());
            }
            for i in 0..s.len() / 2 {
                let b = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).map_err(|e| e.to_string())?;
                m.w8(a.wrapping_add(i as u32), b);
            }
            String::new()
        }
        Some("call") => {
            let f = num(w.next())?;
            let args = w.map(|s| num(Some(s))).collect::<Result<Vec<_>, _>>()?;
            if args.len() > 8 {
                return Err("at most 8 args (r3..r10)".into());
            }
            format!("{:x}", guest_call(c, m, f, &args))
        }
        Some("sec") => {
            let (id, s) = (num(w.next())?, num(w.next())?);
            // rt::sec panics for a module never linked; report that as an error.
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| rt::sec(m, id, s))) {
                Ok(a) => format!("{a:x}"),
                Err(_) => return Err(format!("module {id} not linked")),
            }
        }
        Some("quit") => std::process::exit(0),
        Some(other) => return Err(format!("unknown command {other}")),
        None => return Err("empty line".into()),
    };
    Ok((None, reply))
}

/// The frame hook body: count down frames, then answer commands until the
/// server lets the game run again. stdin closing ends the process.
pub fn hook(c: &mut Cpu, m: &mut Mem) {
    if LEFT.load(Relaxed) > 1 {
        LEFT.fetch_sub(1, Relaxed);
        return;
    }
    let out = std::io::stdout();
    let mut out = out.lock();
    if LEFT.swap(0, Relaxed) == 1 {
        let _ = writeln!(out, "ok {}", rustcube_recomp::os::pumps());
        let _ = out.flush();
    }
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        match command(c, m, &line) {
            Ok((Some(n), _)) => {
                LEFT.store(n, Relaxed);
                return;
            }
            Ok((None, r)) if r.is_empty() => {
                let _ = writeln!(out, "ok");
            }
            Ok((None, r)) => {
                let _ = writeln!(out, "ok {r}");
            }
            Err(e) => {
                let _ = writeln!(out, "err {e}");
            }
        }
        let _ = out.flush();
    }
    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_write_and_errors() {
        let mut c = Cpu::default();
        let mut m = Mem::default();
        assert_eq!(
            command(&mut c, &mut m, "write 80001000 deadbeef").unwrap(),
            (None, String::new())
        );
        assert_eq!(
            command(&mut c, &mut m, "read 0x80001000 4").unwrap().1,
            "deadbeef"
        );
        assert_eq!(command(&mut c, &mut m, "frame 3").unwrap().0, Some(3));
        assert_eq!(command(&mut c, &mut m, "frame").unwrap().0, Some(1));
        assert!(command(&mut c, &mut m, "write 80001000 abc").is_err());
        assert!(command(&mut c, &mut m, "read 80001000 10001").is_err());
        assert!(command(&mut c, &mut m, "sec 3c 1").is_err());
        assert!(command(&mut c, &mut m, "bogus").is_err());
    }
}
