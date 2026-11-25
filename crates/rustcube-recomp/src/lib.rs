//! RUST Cube recompiler: GameCube main.dol and REL modules (PowerPC Gekko)
//! to Rust.
//!
//! The tool (`rustcube-recomp`) reads the player's local main.dol and the decomp's
//! symbols.txt and writes Rust into an ignored folder (`.local`). No game
//! bytes or generated code are part of this crate.

pub mod dol;
pub mod gen;
pub mod global;
pub mod host;
pub mod os;
pub mod rel;
pub mod rels;
pub use rustcube_recomp_rt::rt;
