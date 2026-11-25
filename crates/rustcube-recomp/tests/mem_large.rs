//! MEM1 above 192 MiB: the uncached mirror would reach 0xCC000000, where the
//! GameCube's hardware registers are. Those must stay hardware, or the boot
//! hangs (RUSTCUBE_MEM1_MB=193 and up showed no hardware access at all).
use rustcube_recomp::rt::Mem;
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

static WROTE: AtomicU32 = AtomicU32::new(0);

#[test]
fn hardware_stays_hardware_with_256_mib() {
    std::env::set_var("RUSTCUBE_MEM1_MB", "256");
    let mut m = Mem { io_write: |a, _, _| WROTE.store(a, Relaxed), io_read: |_, _| 0x1234, ..Mem::default() };
    m.w32(0xCC00_3000, 1);
    assert_eq!(WROTE.load(Relaxed), 0xCC00_3000);
    assert_eq!(m.r32(0xCC00_3000), 0x1234);
    // RAM past 192 MiB still works through the cached view.
    m.w32(0x8C80_0000, 0xDEAD_BEEF);
    assert_eq!(m.r32(0x8C80_0000), 0xDEAD_BEEF);
}
