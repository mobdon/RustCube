//! Runtime for the recompiled code. Generated crates depend on this crate
//! only (as `rustcube_recomp`), so changing the generator or the host does not
//! rebuild ~65k generated functions.

#[path = "../../src/rt.rs"]
pub mod rt;
