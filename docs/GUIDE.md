# RUST Cube guide: porting a GameCube game, and everything we learned

This is the write-up for starting a new GameCube project on RUST Cube. It
covers how the toolkit works, how to boot a new game, and what cost us time
on the first game, so the next one goes faster. It names no game: the first
port is called "the first game" throughout.

Contents:

1. [The idea](#1-the-idea)
2. [How the pieces fit](#2-how-the-pieces-fit)
3. [Porting a new game, step by step](#3-porting-a-new-game-step-by-step)
4. [The recompiler in detail](#4-the-recompiler-in-detail)
5. [OS, threads and time](#5-os-threads-and-time)
6. [Graphics](#6-graphics)
7. [Sound](#7-sound)
8. [Parts still to move here](#8-parts-still-to-move-here)
9. [Keeping game data out of git](#9-keeping-game-data-out-of-git)
10. [Testing and debugging](#10-testing-and-debugging)
11. [Build speed](#11-build-speed)
12. [Caching](#12-caching)
13. [Lessons and pitfalls](#13-lessons-and-pitfalls)
14. [Switches](#14-switches)

---

## 1. The idea

Static recompilation: a tool reads the game's machine code and writes one
Rust function per game function, instruction by instruction. The game's own
logic then runs natively with nothing invented or rewritten by hand. The
console's hardware is replaced at its boundary by host code.

Two choices made it work:

- **Every call goes through one function, `rt::call`.** That single choke
  point gives overrides by address (replace any game function with Rust), a
  call counter, a 32-entry trace for crash reports, and a call budget.
  Hardware, fixes, mods and tests all hook in the same way.
- **Replace the chip, not the API.** The game's own SDK (GX graphics, AX
  audio, OS) is linked into the executable and runs as recompiled code. We
  replace only the hardware under it: the GX FIFO pipe, the DSP, the disc.
  That avoided hundreds of per-function hooks and carries to any GameCube
  game, because they all use the same hardware.

The decomp's real value is its **symbol map** (`symbols.txt`), not C code:
there is usually no decompiled C to translate. The map gives function starts
and names, which is all the recompiler and the hooks need.

---

## 2. How the pieces fit

```
main.dol + REL modules + decomp symbols.txt      (your own files, .local only)
        |  rustcube-recomp dol / rels            (this repo)
        v
generated crates (one for main.dol, one per REL)  (.local only, "Do not commit")
        |  link against
        v
rustcube-recomp-rt (Cpu, Mem, call table)
  + rustcube-recomp os/host (threads, alarms, module linking, headless runner)
  + rustcube-hw (GX pipe, VI, clock, I/O router, snapshots, debug port)
  + rustcube-gx (GX state machine and wgpu drawer)
  + your game's own glue crate (its hooks, input, sound, files)
        |
        +--> headless runner (frames to PNG, scripted input)
        +--> desktop window / browser tab (your glue, see section 8)
```

Crate layers depend one way: `rt` (tiny, rarely changes) <- generated code
<- OS, hardware and game hooks <- host. A hook edit never rebuilds generated
code.

---

## 3. Porting a new game, step by step

1. **Get the files.** Your own disc's `main.dol`, its `.rel` modules (PRS
   packed or plain), and the game's decomp project checked out locally (its
   `config/<GAMEID>/symbols.txt` and one folder per module). Put them under
   `.local/`. Hash them (`sha1sum`) and write the hashes down: every address
   you find belongs to that exact build.
2. **Recompile main.dol.**
   `cargo run -p rustcube-recomp --release -- dol .local/game/main.dol .local/decomp/config/GAMEID/symbols.txt .local/gen`
   It prints functions found, unknown instructions (by opcode) and `bl`
   targets that have no function. Unknown instructions become a `panic!`
   naming the word and address, so a build always compiles.
3. **Recompile the modules.**
   `cargo run -p rustcube-recomp --release -- rels .local/game/rels .local/decomp/config/GAMEID .local/rels`
   One crate per module, one Cargo feature per module (`all` for every one),
   and `REPORT.txt` with per-module counts. Boot needs main.dol only.
4. **Write a host crate** in `.local/host` (or in the game's own private
   repo). Minimal version:

   ```rust
   use rustcube_hw::Symbols;
   use std::sync::OnceLock;

   static SYMS: OnceLock<String> = OnceLock::new();

   fn setup(c: &mut rustcube_recomp::rt::Cpu, m: &mut rustcube_recomp::rt::Mem) {
       let syms = Symbols::parse(SYMS.get().unwrap());
       let missing = rustcube_hw::install_all(c, m, &syms);
       eprintln!("not hooked (left recompiled): {missing:?}");
       // Your game's hooks go here: disc reads, pad, audio, card...
   }

   fn main() {
       let a: Vec<String> = std::env::args().collect();
       let dol = std::fs::read(&a[1]).unwrap();
       let syms = SYMS.get_or_init(|| std::fs::read_to_string(&a[2]).unwrap());
       rustcube_recomp::host::run(&dol, syms, gen::dispatch, |_| {}, setup, 100_000_000, 20).unwrap();
   }
   ```

   `host::run` loads the DOL, writes the boot globals the IPL leaves in low
   memory, finds the OS functions by name (`OsAddrs::from_symbols`; it tells
   you which names are missing so you can fill them in by hand), runs from
   the entry point and stops at the first panic, the call budget or the time
   limit. It then prints the last 32 calls with symbol names and every
   hardware address the game touched.
5. **Boot by fixing the first panic, over and over.** That is how the first
   game was booted. Typical stops, in order: an unknown instruction (add it
   to `gen.rs`, with a synthetic test), a hardware register nobody answers
   (the runner's `io_read` makes transfers finish at once and mailboxes
   alternate; add yours), a spin on a register or a clock (something must
   move: see lesson 7), a missing SDK function name (hook it by hand).
6. **First frames.** With `rustcube-hw` installed, `gx::take_frames()` gives
   decoded GX frames; `snap::frame` writes every Nth one as a PNG headless.
   Look at the picture: a log line is not proof.
7. **Then**: disc reads, controller, sound, memory card, window, browser
   (section 8), and a "start straight in scene X" hook for fast testing.
8. **Exit bar for "it works"**: boots headless without a panic, produces
   frames, a recording replays identically, a person finishes a level and
   gets back to the menu with the game still running, twice, in several
   levels (lesson 33).

---

## 4. The recompiler in detail

Files in `crates/rustcube-recomp/src`:

| File | What it does |
|---|---|
| `gen.rs` | Gekko instruction to Rust text. `translate(word, pc, ctx)` returns code, local branch targets, "ends block" and "unknown". `function()` emits one function; `layout()` finds function ranges |
| `rt.rs` (crate `rustcube-recomp-rt`) | `Cpu` (32 GPRs, 32 two-slot FPRs, CR, LR, CTR, XER bits, GQRs, SPRs, time base), big-endian `Mem` with hardware callbacks, `Runtime` call table with overrides, helpers |
| `dol.rs`, `rel.rs` | DOL and REL parsers, decomp-toolkit `symbols.txt` parser, SEGA PRS decompression for packed modules |
| `rels.rs` | Every REL in a decomp config to one crate each, relocations turned into Rust expressions |
| `os.rs` | Dolphin OS threads, alarms, interrupts and module linking (section 5) |
| `host.rs` | Headless runner |
| `global.rs` | `Global<T>`: a mutex cell with the `thread_local!` method names, because game threads run on separate host threads |

How the generated code works:

- **One Rust function per game function**, `fn(&mut Cpu, &mut Mem)`. The body
  is `loop { match pc { addr => { .. } } }` over basic blocks. A branch inside
  the function sets `pc` and continues; a branch out of it is a tail call
  through `rt::call` then `return`.
- **Overrides by address**: `Runtime::set_override(addr, f)` replaces any
  recompiled function. A hook can still run the original through
  `(c.rt.generated)(addr)`.
- **Function discovery** (`layout`): start from symbol addresses, add every
  `bl` target and every branch that leaves its function, repeat until nothing
  changes. Needed because register save/restore ladders have one symbol and
  many entry points, and some modules have no symbols.
- **Jump tables**: code pointers found in data sections become extra block
  starts; `bctr` becomes `pc = ctr; continue`. `looks_like_entry` tells a
  function pointer from a jump-table case.
- **Relocated code**: REL addresses are section offsets; the section's load
  address is looked up at run time (`rt::sec(m, id, sec)`) because the game's
  own `OSLink` places modules.
- **Back-edges tick the runtime**: every backward branch calls
  `rt::backedge`, and every 2^20 ticks a preempt hook runs. That is how spin
  waits end and how "interrupts" arrive.
- **Floats**: FPRs are `f64` pairs. Single-precision results go through
  `rs(v) = v as f32 as f64`. Fused multiply-add uses plain `a*c+b` when both
  factors hold f32 values (exact in f64) and `mul_add` otherwise, because
  `mul_add` is a slow software routine on wasm. `fres`/`frsqrte` return exact
  values, not the console's estimate tables (those come from GPL code and
  stay out of git).
- **Memory**: `RUSTCUBE_MEM1_MB` (default 200, retail 24) gives the game more
  RAM than the console had, which lets a game keep more modules loaded. Above
  192 MiB the uncached mirror would reach the hardware registers at
  0xCC000000; the runtime keeps those as hardware (`tests/mem_large.rs`).

---

## 5. OS, threads and time

`os.rs` replaces only two context primitives of the game's own OS thread
code; everything else (OSCreateThread, mutexes, message queues, the
scheduler's priority pick) runs recompiled.

- `OSSaveContext(ctx)` records that `ctx` belongs to the running thread and
  returns 0; `OSLoadContext(ctx)` switches to the thread owning `ctx`.
- Each OS thread runs on a host thread of its own with its own `Cpu`. **One
  baton** (memory, call table, time base) moves between them, so exactly one
  runs at a time and every run is deterministic.
- Interrupts: once a frame, with the scheduler disabled, time moves, due
  alarms fire and frame hooks (the VI retrace) run; then it reschedules as an
  interrupt return would. Frames come from `VIWaitForRetrace`, from the
  scheduler idling, and from a long spin.
- `__OSInitAudioSystem` is skipped (it boots the DSP and waits for mail).
- **Module unlink**: an override set on a module's function is taken off when
  that module unlinks and restored when the same module links at the same
  place again. Without this, the next module loaded at that address ran the
  old override (the crash showed as "access outside RAM at 0x9421fff8", which
  is a `stwu r1,-8(r1)` instruction read as an address; lesson 30).

Addresses: `OsAddrs::from_symbols(symbols_txt)` looks up the SDK names
(`OSSaveContext`, `OSLoadContext`, `OSInitContext`, `OSSetAlarm`,
`OSSetPeriodicAlarm`, `OSCancelAlarm`, `__OSInitAudioSystem`,
`OSEnableInterrupts`, `OSDisableScheduler`, `OSEnableScheduler`,
`__OSReschedule`, `OSResumeThread`, `OSSuspendThread`, `OSLink`, `OSUnlink`)
and the OSThread.c statics `IdleContext`, `RunQueueBits`, `Reschedule`. If a
decomp names one differently, fill that field in by hand.

The clock (`rustcube-hw/src/clock.rs`) answers `OSGetTime`, `OSGetTick` and
`__OSGetSystemTime` from the CPU time base, which moves one NTSC field per
frame and one microsecond per read. Polling loops end, and runs repeat
exactly. One bug worth knowing: moving the clock in the VI retrace as well
as in the OS frame ran game time at double speed, and the game skipped every
other draw to catch up.

---

## 6. Graphics

`rustcube-gx` is GameCube GX on wgpu; most of it is noclip.website's generic
GX code translated to Rust (see [CREDITS.md](../CREDITS.md)).

How the game reaches it (`rustcube-hw/src/gx.rs`): the game's own GX library
writes to the write-gather pipe at 0xCC008000. Those bytes queue up during the
frame; at `GXCopyDisp` the queue and a copy of RAM (vertex arrays, textures,
display lists all live there) go to a GX thread, which decodes them into a
`Frame` one frame behind the game. The window or the snapshotter draws it.
Shortcut: RAM is read as it is at the end of the frame, so a game that
rewrites vertex data mid-frame would show the later data.

| File | Role |
|---|---|
| `api.rs` | `Gx`: a GX-API-shaped state machine. It only records; `take_frame()` returns draws and texture uploads, no GPU needed |
| `fifo.rs` | The FIFO as a byte sink: pieces of any size through the display-list parser |
| `vtx.rs` | Display lists and the vertex loader |
| `material.rs`, `shader.rs` | Material state and WGSL: light channels, texgens, indirect stages, TEV, alpha test, fog |
| `texture.rs` | Every GX texel format to RGBA8, palettes, mip chains |
| `gpu.rs`, `params.rs` | The wgpu side: state, pipeline cache, textures, samplers, uniforms |
| `interp.rs` | Pictures between game frames for 120/144/165 Hz |
| `widescreen.rs` | Wider field of view with the HUD re-anchored |

Decisions and notes:

- GX depth runs -1 at near to 0 at far; converted to wgpu's 0 to 1.
- Not on the web, as in noclip: LOGIC blending, lines and points, fog range
  adjustment.
- **High refresh without touching game speed**: the game always runs at 60.
  Each draw of the newest frame is matched to one in the frame before (same
  material, vertex counts, nearest among those, textures not in the key so a
  flipbook effect matches itself) and its matrices are blended. No match, or a
  jump (camera cut), shows as it is.
- **Widescreen**: a wider horizontal field of view, nothing stretched. HUD in
  the left or right third moves to that edge, the middle stays centred,
  full-screen backdrops stretch. Keep together what belongs together: a row
  of digits, a panel and the text on it. A HUD element that follows a 3D
  object (a lock-on reticle) follows the 3D view. `HUD_EDGE` was tuned on the
  first game; another game may want its own. The game's own culling area has
  to be widened too, or objects pop at the old 4:3 edges (that part is game
  code, a hook per game).
- Bugs that cost real time:
  - Partial GPU buffer uploads zeroed up to 3 bytes when a change ended
    mid-word. It looked like warped meshes, a stray line, wrong letters.
    Found by frame-by-frame headless comparison.
  - The game re-uploads about 160 textures every frame; a GPU texture for
    each grew driver memory 8 MB every 10 seconds. Keep only the newest data
    per texture id and upload when it is drawn (`snap.rs` does this).
  - Two depth textures per picture in the browser blew up GPU memory. One per
    size.
  - Striped or noisy textures can mean the **game's own heap is full**, not a
    renderer bug (lesson 27).
- Still open on the first game: flicker on distant walls (suspect depth
  precision; reversed Z was the next thing to try).

`cargo test` checks every generated shader with naga and needs no GPU.
`tests/gpu_draw.rs` draws on a real adapter and skips when there is none.

---

## 7. Sound

The first game's mixer is not in this repo yet (section 8). How it works, so
it can be moved:

- The game's own AX audio and mixer libraries run recompiled. Only
  `__AXOutInit` is skipped (it boots the DSP).
- Audio RAM is real host memory; `ARStartDMA` and `ARQPostRequest` move bytes
  and run their callbacks.
- Every 5 ms of game time a frame hook does the DSP's job: sync the game's
  voice changes into the parameter blocks, mix every running voice from ARAM
  (DSP-ADPCM, PCM16, PCM8) to stereo at 32 kHz, write back the state the DSP
  would update (position, ADPCM history, volumes), then run the voice
  callbacks and the game's frame callback.
- Output to a .wav (also how the browser gets it) or a sound device, with a
  mute switch.
- Checked by correlation: a voice line matched its source file at 1.000.
- Gaps: no reverb or surround, simple resampling, stereo pan unconfirmed.
- **The bug worth remembering**: the `ARQPostRequest` stand-in returned
  success but did not fill the request record. Streaming code read that
  record, credited every transfer to stream 0, and stereo music never
  started. **A stub must fill every field the caller reads.**

For sound files outside the game (tools, web previews), vgmstream decodes
most formats; build its core library to wasm with a small shim and keep its
license with it.

---

## 8. Parts still to move here

These work in the first game's port but are written against its addresses
and file layout. Each should come here hooked by SDK name, like `rustcube-hw`.

| Part | What it does | Notes for moving |
|---|---|---|
| Disc (`DVDOpen`, `DVDReadPrio`...) | Serves reads from a `GameFiles` trait: a folder, layered folders, or files a browser picker read. Reads finish at once and run their callbacks | Trait is generic; drop the first game's path rules |
| Controller (`PADRead`...) | Host keyboard or gamepad to a raw `PADStatus`; the game's own clamp code runs on it. Scripted input, record and replay | Scripted input anchored to a **file the game opens** (`title.adx+60~60:1000`: press Start 60 frames after that file loads, repeat until the next one opens) survived every timing change. Best idea in the harness |
| Audio | Host-as-DSP mixer (section 7) | |
| Memory card | CARD library level: each card file is a host file | |
| Window | winit + wgpu present, frame pacing, input, capture of presented pictures | |
| Browser build | Game as `wasm32-wasip1-threads`, one web worker per game thread on shared memory; page renders with `rustcube-gx`; device files `/dev/gx`, `/dev/pad`, `/dev/audio` | Needs nightly with `build-std` and SIMD. Cap shared memory (browsers commit the whole maximum up front); send only the RAM pages the decoder read, not all of RAM; addresses unsigned; run tests under a memory watchdog |

---

## 9. Keeping game data out of git

Copy these on day one of any project:

1. **`.local/` is ignored** and holds everything derived from the game: the
   files, generated Rust, built hosts, launchers, captures, saves, tools.
2. **The tool is in git, its output never is.** Generated files start with
   "Do not commit".
3. **Tests carry no game bytes.** Synthetic tests are hand-assembled. Tests
   that need real files read an environment variable and skip when unset.
4. **Ignore patterns for game formats** (`*.iso`, `*.rvz`, `*.rel`, `*.prs`,
   `*.dol`, audio, textures, saves) as a second net.
5. **Bring your own copy.** Never host game files or a recompiled build (it
   contains the game's code).
6. **Nothing from GPL emulators**, not even tables. Load them from `.local`
   with an exact-math fallback if a game needs them.
7. **No game's addresses or symbols in this toolkit.** They live in the
   game's own project.

---

## 10. Testing and debugging

- **Test the translator, not the game.** `build.rs` assembles small programs
  by hand and runs them through the real translator (`tests/synthetic.rs`).
  The next step is per-instruction differential tests against a CPU emulator
  on random register values: no game bytes, runs in CI, and covers every
  game. (Unicorn's PowerPC core has no paired singles; those need a hand
  reference.)
- **Determinism makes every bug replayable**: time moves by frame and by
  read, one guest thread at a time. A recording is a regression test.
- **Headless first.** Never open a window, take focus or play sound during
  automated tests on someone's PC.
- **Debug port** (`rustcube-hw/src/serve.rs`, `RUSTCUBE_SERVE`): another
  process drives the game one frame at a time over stdin/stdout:
  `frame N`, `read ADDR LEN`, `write ADDR HEX`, `call ADDR ARGS`,
  `sec MODULE SECTION`, `quit`. Scripts and agents should use this rather
  than adding one-off switches.
- **Compare pictures, not logs.** Side-by-side sheets of the same frames
  from two builds; a blink detector (pixels that differ from both neighbours
  while the neighbours agree) finds one-frame flicker. These were one-off
  scripts on the first project; put them in a crate here.
- **A test is a folder**: input script or recording, frames to capture,
  expected hashes and tolerance; one command prints pass or fail and a sheet
  for failures. Golden images are game output and stay in `.local`.
- **Say how far a change was checked** in every report: compiled, unit
  tested, run in the game, seen by a person.
- When a build "worked last week", keep one saved build per merged change so
  it is a two-build bisect.

---

## 11. Build speed

What slowed the first project, all observed:

| Problem | Fix |
|---|---|
| A change to the runtime rebuilt ~65,000 generated functions | The runtime is its own tiny crate (`rustcube-recomp-rt`); keep its API stable; put experiments behind a function pointer in `Runtime` |
| One new function start reshuffled every generated file (150 functions per file by position) | Lay out generated files by address range and write a file only when its bytes changed (not done yet: do it here) |
| rustc crashed with 24 module crates at once on 32 GB; one full build used 102 GB | `jobs = 6` in the generated workspace (the tool writes it), opt-level 1 for generated code, retry crashing crates at lower optimisation, split huge functions at emit time |
| Threads shared one build folder | One build folder per worktree, with a shared compile cache (sccache) |
| A running game locks its exe and blocks the next link | Run a copy of the built exe |
| Every hook edit relinked the whole game | Hooks in their own crate linked last; a fast linker (lld or mold) |
| Reaching the scene under test took boot, logos and menus every run | A per-game "start straight in scene X" hook, early |
| ~200 environment switches read all over the code | One config file per run with `--set key=value`; the tool lists every key |
| Each guest call nests a Rust frame | 1 GiB host stack on desktop (the runner sets it), 64 MiB on wasm |

Keep pure logic (translator, parsers, maths) in crates with no window, sound
or GPU dependencies, so they build and test anywhere, including the cloud.

---

## 12. Caching

| Technique | Where |
|---|---|
| Call table: a flat array by address for main.dol, filled on first use (not a hash, not a giant match) | `rt.rs` |
| Module call cache, cleared when a module links or unlinks (`modules_changed`) | `rt.rs`, `os.rs` |
| I/O routes copied per thread and refreshed by a version number (the GX pipe sends millions of writes a frame; a lock per write cost ~8%) | `rustcube-hw/src/io.rs` |
| RAM copies for the GX thread reused, not allocated per frame | `rustcube-hw/src/gx.rs` |
| Material, vertex loader, pipeline, sampler and bind-group caches | `rustcube-gx` |
| Textures: newest data per id, uploaded only when drawn | `snap.rs` |
| Texture cache by address and format, re-checked by content hash; render-to-texture copies tracked by address | first game's renderer; move here |
| GPU buffers kept between frames, upload only what changed | `rustcube-gx` |
| Redraw a recorded frame for high refresh instead of running the game faster | `interp.rs` |
| Decoded sound cached, **but never music** | first game's audio |
| File reads: block cache, stat cache | first game's disc layer |
| Browser: build files revalidated by ETag, game files never cached; compressed build beside the plain one, replaced together; compile the wasm once and share it with ready workers; service worker for the page shell only | first game's web build |

What to avoid (all happened): sharing one build folder between threads;
cleaning the build cache to "fix" something; rewriting all generated files
every run; a runtime crate that changes under the generated code; dropping
GPU textures without `destroy()` in a browser; a stale compressed build next
to a new one with no build id on screen; caching music; caching a failed file
read when permission can arrive later; trusting a cache stays small (log its
size).

---

## 13. Lessons and pitfalls

Design:

1. There is usually no decompiled C. The decomp's value is its symbol map.
2. Replace hardware at the lowest cheap boundary; let the game's own
   libraries run.
3. One call choke point with overrides by address is the whole extension
   story.
4. Determinism first.
5. Keep game speed fixed; add smoothness in the renderer.
6. Stubs must fill every field the caller reads.
7. Polling loops need something that moves: the time base advances on each
   read, mailbox registers alternate, transfers finish at once, long loops get
   a preempt tick.
8. Game code may call into a module after unlinking it. Track where each
   module was linked.
9. Two builds of one game (disc and a later PC release, two regions) are not
   interchangeable beyond relocations. Diff first. Aligning data by content
   is fragile: zero padding matches at any shift.
10. Check the original game's behaviour before calling something a bug.

Performance:

11. A locked atomic add on every guest call cost ~5% of the game thread. Use
    plain loads and stores for diagnostic counters.
12. A mutex and hash insert per module address lookup cost ~9%. Use a
    lock-free slot per module id.
13. Flat call table for the main executable.
14. Software fused multiply-add is slow on wasm (see section 4).
15. Measure before optimising: real tail calls and cheaper thread switches
    were measured as not needed at 60 fps.

Build and operations:

16. Split the runtime into its own crate from day one.
17. Limit parallel compiles of generated crates.
18. Run a copy of the built exe.
19. One build folder per worktree.
20. Memory guards on long builds and on every browser test.
21. Headless only on someone else's PC.
22. A log line is not a picture. Look at the frame.
23. Close or mute any page that plays audio once it is checked.

Legal and credit:

24. Emulators are a behaviour reference only.
25. Credit every source from the first commit, with pinned commits and
    license texts.
26. Generated code and hosted builds contain the game. Keep them local.

Found while getting the first game fully playable:

27. **The game's heap is a fixed budget, and running out does not look like
    an error.** Extra data crowded it and showed up as garbled textures and
    a freeze on a scene change. Measure free heap before and after anything
    you add; fail a test when it drops under a floor; keep added things in
    host memory where possible.
28. A one-frame texture flash on the first draw of a new element. Capture the
    first frames after anything appears, every frame.
29. Widescreen: labels over a centred panel must stay with the panel.
30. A crash "address" that is an instruction (`0x9421FFF8` is
    `stwu r1,-8(r1)`) means a function pointer was overwritten or stale.
31. Hit shapes: one shape can serve several purposes (damage and pickups);
    the game may write sizes back after you set them. Measure in the running
    game after a few frames and after a state change.
32. Custom-drawn objects skip passes that run over the game's ordinary models
    (shadows, reflections, outlines) unless their draw code feeds them.

Getting a plain game running:

33. "Loads and produces frames" is not a pass. Finish a level, see a clean
    result screen, get back to the menu, twice, in several levels. Scene
    changes are where memory problems show.
34. Shortcuts (direct scene starts, forced states) skip set-up code. Confirm
    once on the normal route.
35. The first frame of a scene is not normal: half the objects don't exist
    yet. Hooks that decide from game state must cope.
36. One clean start proves little. Run a start several times; in a
    deterministic design a result that changes between runs is the bug.
37. Never report "works" from one number. Compare frames.
38. Say how far a change was checked (compiled, unit tested, run, seen).
39. A freeze with memory still free is a different bug from running out
    (likely a damaged heap list). The heap log tells you which.
40. Memory taken on one path may not come back on another. Measure free heap
    after each way into a scene (finish, die, quit).
41. Hash the game files and refuse a mismatch. Every absolute address belongs
    to one build only.
42. Know what silence is expected before chasing it (logos and movies are
    silent). Write audio to .wav headless and correlate with the source.
43. Known mixer gaps: no reverb or surround, simple resampling.
44. Keep pure logic free of heavy dependencies so it tests anywhere.
45. Cap the number of builds kept, and back up every local commit the same
    day.
46. Values tuned by eye (angles, offsets) should be settable without a
    rebuild; the confirmed numbers then become the defaults.

---

## 14. Switches

Environment variables read by this repository:

| Switch | Effect |
|---|---|
| `RUSTCUBE_MEM1_MB=N` | RAM size given to the game, 24 to 256 (default 200) |
| `RUSTCUBE_REGS` | Headless runner: print registers when it stops |
| `RUSTCUBE_PEEK=addr,...` | Headless runner: print each word and the 8 words it points at |
| `RUSTCUBE_STACK=N` / `stall`, `RUSTCUBE_STACK_THREAD=addr` / `all` | Guest call stacks (once on a stall, or of every OS thread) |
| `RUSTCUBE_TRACE_THREADS` | Log OS thread starts and switches |
| `RUSTCUBE_SERVE` | Debug port on stdin/stdout (the host's frame hook calls `serve::hook`) |
| `RUSTCUBE_PROFILE`, `RUSTCUBE_PROFILE_GPU` | fps and where each frame's time went, once a second |
| `RUSTCUBE_FRAMES_EVERY`, `RUSTCUBE_FRAMES_RANGE=a-b` | Which frames `snap::frame` writes (the host passes the folder) |
| `RUSTCUBE_SNAP_INFO`, `_2D`, `_WIDE`, `_ROP`, `_COPIES`, `_CHECK`, `_TWEEN`, `RUSTCUBE_GPU_REPORT` | Extra snapshot diagnostics: draw list, HUD draws, widescreen view, blend state, EFB copies, redraw check, in-between picture, GPU memory |

Plan: replace these with one config file per run (section 11).
