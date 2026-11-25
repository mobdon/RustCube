//! GameCube GX on wgpu.
//!
//! Most of this crate is noclip.website's generic GX code (Copyright (c)
//! 2018 Jasper St. Pierre, MIT; see THIRD-PARTY.md) translated from
//! TypeScript to Rust, with WGSL in place of GLSL and wgpu in place of
//! noclip's Gfx layer. `api.rs` adds the GX-API-shaped state machine the
//! recompiled game calls. No game-specific noclip code is included.
#![forbid(unsafe_code)]

pub mod api;
pub mod enums;
pub mod fifo;
pub mod gpu;
pub mod interp;
pub mod material;
pub mod params;
pub mod shader;
pub mod texture;
pub mod vtx;
pub mod widescreen;
