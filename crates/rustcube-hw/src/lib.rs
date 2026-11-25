//! RUST Cube hardware layer: host replacements for the GameCube parts a
//! recompiled game talks to (GX pipe, video interface, clock), plus the
//! address-range router they share, headless PNG snapshots, a profiler and a
//! stdin/stdout debug port.
//!
//! Each part hooks the game's own copy of the SDK by function name, looked up
//! in the game's decomp symbols.txt, so nothing here knows any game's
//! addresses. A name a game doesn't have is reported, not guessed.
use rustcube_recomp::rt::{call, Cpu, Func, Mem};
use std::collections::HashMap;

pub mod clock;
pub mod gx;
pub mod io;
pub mod prof;
pub mod serve;
#[cfg(not(target_os = "wasi"))]
pub mod snap;
pub mod vi;

/// A decomp-toolkit symbols.txt, by name.
#[derive(Default, Clone, Debug)]
pub struct Symbols(HashMap<String, u32>);

impl Symbols {
    pub fn parse(text: &str) -> Self {
        Symbols(rustcube_recomp::dol::parse_symbols(text).into_iter().map(|s| (s.name, s.addr)).collect())
    }

    pub fn get(&self, name: &str) -> Option<u32> {
        self.0.get(name).copied()
    }

    /// Override the function called `name` with `f`. Returns its address, or
    /// adds the name to `missing`.
    pub fn hook(&self, c: &mut Cpu, name: &'static str, f: Func, missing: &mut Vec<&'static str>) -> Option<u32> {
        let a = self.get(name);
        match a {
            Some(a) => c.rt.set_override(a, f),
            None => missing.push(name),
        }
        a
    }
}

impl FromIterator<(String, u32)> for Symbols {
    fn from_iter<I: IntoIterator<Item = (String, u32)>>(it: I) -> Self {
        Symbols(it.into_iter().collect())
    }
}

/// Install the GX pipe, VI and clock replacements. Returns the SDK function
/// names the game's symbols don't have (those stay recompiled as they are).
pub fn install_all(c: &mut Cpu, m: &mut Mem, syms: &Symbols) -> Vec<&'static str> {
    let mut missing = Vec::new();
    gx::install(c, m, syms, &mut missing);
    vi::install(c, m, syms, &mut missing);
    clock::install(c, m, syms, &mut missing);
    missing
}

/// Call a guest function with `args` in r3.., keeping the caller's registers;
/// returns r3.
pub fn guest_call(c: &mut Cpu, m: &mut Mem, f: u32, args: &[u32]) -> u32 {
    let saved = (c.r, c.lr, c.ctr, c.cr);
    for (i, a) in args.iter().enumerate() {
        c.r[3 + i] = *a;
    }
    call(c, m, f);
    let r3 = c.r[3];
    (c.r, c.lr, c.ctr, c.cr) = saved;
    r3
}

/// Run the recompiled original at `addr` (for hooks that wrap it).
pub fn original(c: &mut Cpu, m: &mut Mem, addr: u32) {
    if let Some(f) = (c.rt.generated)(addr) {
        f(c, m);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hooks_by_name_and_reports_the_rest() {
        let syms: Symbols = [("GXCopyDisp".to_string(), 0x8010_0000), ("OSGetTick".to_string(), 0x8010_0100)].into_iter().collect();
        let (mut c, mut m) = (Cpu::default(), Mem::default());
        let missing = install_all(&mut c, &mut m, &syms);
        assert!(c.rt.overrides.contains_key(&0x8010_0000) && c.rt.overrides.contains_key(&0x8010_0100));
        assert!(missing.contains(&"VIWaitForRetrace") && !missing.contains(&"GXCopyDisp"));
    }
}
