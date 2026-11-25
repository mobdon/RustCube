# Credits

RUST Cube stands on other people's work. Every source is named here, with
what was taken from it and under which license. Nothing below is disguised or
renamed. If you add code from somewhere, add it here in the same commit.

## Code included in this repository

### noclip.website (MIT)

`crates/rustcube-gx` is mostly a Rust translation of noclip.website's generic
GameCube GX code: GX enums, display lists and the vertex loader, texture
formats, materials and TEV shader generation, and the render state. GLSL was
rewritten as WGSL and noclip's graphics layer was replaced by wgpu. Only
noclip's generic GX hardware layer was used, never its code for any game.

- https://github.com/magcius/noclip.website, commit `2cebac2a5d1d7c44535397a14f51e764ca823767`
- Copyright (c) 2018 Jasper St. Pierre and noclip.website contributors, MIT License
- File-by-file map and the full license text: [crates/rustcube-gx/THIRD-PARTY.md](crates/rustcube-gx/THIRD-PARTY.md).
  Each derived file also starts with a notice naming its source.

### Rust libraries (fetched by Cargo, not copied)

| Library | Used for | License |
|---|---|---|
| [wgpu](https://github.com/gfx-rs/wgpu), [naga](https://github.com/gfx-rs/wgpu/tree/trunk/naga) | Drawing GX frames (WebGPU, Vulkan, DX12, Metal, GL); shader checks in tests | MIT OR Apache-2.0 |
| [bytemuck](https://github.com/Lokathor/bytemuck) | Uniform and vertex buffer packing | Zlib OR Apache-2.0 OR MIT |
| [pollster](https://github.com/zesterer/pollster) | Waiting on wgpu calls in headless tools | MIT OR Apache-2.0 |
| [png](https://github.com/image-rs/image-png) | Headless frame snapshots | MIT OR Apache-2.0 |

## Tools and projects the work relied on (nothing copied)

### decomp-toolkit

The recompiler reads function and object addresses from a decomp-toolkit
`symbols.txt` (the `name = .section:0xADDR; // type:function size:0x..`
format), and the OS and hardware layers find the SDK functions they replace by
name in that file. https://github.com/encounter/decomp-toolkit (MIT OR Apache-2.0).

### The decompilation project of the first game ported

RUST Cube was built while porting one GameCube game, using that game's
community decompilation project for its symbol map (`symbols.txt` and module
configs). Its symbols were read from a local checkout only; no symbol, address
or code from it is in this repository. Thanks to the people who mapped it.

### N64Recomp and XenonRecomp

The idea of static recompilation to a native language, one function per game
function with hardware replaced at its boundary, follows these projects.
RUST Cube was Rust writing Rust from its first commit; no code was taken from
either.

- N64Recomp by Mr-Wiseguy: https://github.com/N64Recomp/N64Recomp
- XenonRecomp by hedge-dev: https://github.com/hedge-dev/XenonRecomp

### Dolphin (reference only)

Dolphin's documentation and source were read to understand GameCube hardware
behaviour (registers, DSP mailbox, time base). **No Dolphin code is included.**
Dolphin is GPL-2.0-or-later, which is not compatible with this repository's MIT
license, so anything derived from it stays out of git. In particular the Gekko
`fres`/`frsqrte` estimate tables are not here: `rt::fres` and `rt::frsqrte`
return exact values. A game that needs the console's exact estimates can load
its own tables from a local, ignored folder. https://github.com/dolphin-emu/dolphin

### vgmstream (sound in the first port)

The first game's port decoded its sound files with vgmstream (core library
built to WebAssembly). That build is not part of this repository yet; when it
is added, its license text (ISC-style, see vgmstream's COPYING) and any codec
licenses go with it. https://github.com/vgmstream/vgmstream

### GameCube SDK behaviour

The OS thread, alarm and context code in `rustcube-recomp/src/os.rs` was
written for this project from the documented behaviour of the Dolphin OS SDK
functions it replaces. It contains no SDK code; the game's own SDK runs as
recompiled code around it.
