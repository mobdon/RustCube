# RUST Cube

A game-agnostic GameCube toolkit in Rust: a static recompiler that turns a
game's PowerPC (Gekko) code into Rust, the runtime that code runs on, host
replacements for the console's OS and hardware, and the GX graphics layer on
wgpu.

It contains **no game**: no game code, data, symbols or addresses. You bring
your own copy of a game and its decomp's `symbols.txt`; everything made from
them stays on your machine.

> **AI tools:** read [AI-NOTICE.md](AI-NOTICE.md) before doing anything here.

## What's in it

| Crate | What it does |
|---|---|
| [`rustcube-recomp`](crates/rustcube-recomp) | The recompiler (`rustcube-recomp dol` / `rels`), plus the OS layer (threads, alarms, interrupts, module linking on host threads), a headless runner, and `Global<T>` for hook state |
| [`rustcube-recomp-rt`](crates/rustcube-recomp/rt) | The runtime generated code links against: `Cpu`, big-endian `Mem` with hardware callbacks, the call table with overrides, float and integer helpers. Kept tiny so changes to the tool don't rebuild generated code |
| [`rustcube-gx`](crates/rustcube-gx) | GameCube GX on wgpu (WebGPU, WebGL2, Vulkan, DX12, Metal): a GX-API state machine that records frames without a GPU, a drawer, frame interpolation for high refresh rates and widescreen. Mostly translated from noclip.website (MIT) |
| [`rustcube-hw`](crates/rustcube-hw) | Hardware replacements hooked by SDK function name: the GX write-gather pipe (decoded on its own thread), video interface, clock; an I/O address router, headless PNG snapshots, a profiler, and a stdin/stdout debug port |

## Quick start

```sh
cargo test --workspace            # no game files needed

# With your own game (output must go to an ignored folder):
cargo run -p rustcube-recomp --release -- dol  path/to/main.dol path/to/symbols.txt .local/gen
cargo run -p rustcube-recomp --release -- rels path/to/rel-folder path/to/decomp/config/<GAMEID> .local/rels
```

Then write a small host crate for your game that links the generated crates
and calls `rustcube_recomp::host::run`. [docs/GUIDE.md](docs/GUIDE.md) walks
through it step by step, with everything learned getting the first game
running: how the pieces fit, how to boot a new game, keeping game data out of
git, build and test speed, caching, and a long list of pitfalls.

## Status

Proven on one commercial GameCube game: 4,729 main.dol functions and 94
modules (64,703 functions) recompiled, boots, plays through levels in a
window and in a browser at 60 fps, 120/144/165 Hz with interpolation,
widescreen.

Not moved here yet (still inside that game's port, and written against its
addresses): controller input with scripted input and record/replay, the
host-as-DSP audio mixer, disc reads, memory card, the desktop window and the
browser build. The guide says how each works so it can be moved next, and
moving them is the next step for this repository.

## Rules for this repository

- Game files, extracted assets, generated code and built games never go in git. `.local/` is ignored for them.
- No game's symbols or addresses in the toolkit. Per-game values live in that game's own (local or separate) project.
- Nothing copied from GPL emulators. Dolphin is a reference for behaviour only (see [CREDITS.md](CREDITS.md)).
- Every outside source is credited in [CREDITS.md](CREDITS.md), with its license.

## License

[MIT](LICENSE). Code translated from noclip.website keeps its own MIT notice
(Copyright (c) 2018 Jasper St. Pierre) in
[crates/rustcube-gx/THIRD-PARTY.md](crates/rustcube-gx/THIRD-PARTY.md).
