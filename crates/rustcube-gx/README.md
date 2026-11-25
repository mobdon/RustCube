# rustcube-gx

GameCube GX on wgpu (WebGPU, WebGL2 fallback, native for tests). It is noclip.website's generic GX code translated to Rust, plus a GX-API layer for the recompiled game. Credits and the MIT license are in [THIRD-PARTY.md](THIRD-PARTY.md).

## Two ways in

**GX calls (recompiled game).** `api::Gx` takes the GX functions in snake case (`set_tev_order`, `set_blend_mode`, `begin` / `position3f32` / `end`, `call_display_list`, `load_tex_obj`, `load_pos_mtx_imm`, ...). It only records; it needs no GPU. Calls that read game memory take a `&dyn GxMemory` (`api::Ram` maps 0x8.../0xC... addresses onto a RAM slice).

```rust
let mut gx = Gx::new();
// ... GX calls for a frame ...
let frame = gx.take_frame();            // draws + texture uploads, headless
renderer.render_frame(&frame, &view, [w, h]);   // gpu::GxRenderer
```

**Model data (a game's own model formats).**
- `vtx::VtxLoader::new(vat, vcd).run_vertices(dl, &arrays)` turns a display list into a flat vertex buffer and triangle indices.
- `material::GxMaterialBuilder` builds the material (or `set_from_registers` from a material display list).
- `gpu::GxRenderer::render` draws `GpuDraw`s (pipelines are cached per material and vertex layout).
- `texture::decode_mip_chain` decodes every GX texel format, including CI formats with TLUTs.

## Notes

- Projection: `Gx::set_projection` takes the GX matrix (z from -1 at near to 0 at far). It is converted to wgpu's 0..1 depth, with near at 0.
- Textures are stored as `Rgba8Unorm`, like noclip. Write to an sRGB view if the output must be gamma-encoded.
- Not on the web: LOGIC blending (drawn as no blend, as noclip does), line and point primitives (skipped), and fog range adjustment (noclip has none either).
- `cargo test` checks every generated shader with naga. `tests/gpu_draw.rs` draws a frame on a real adapter and skips when there is none; in the cloud it runs on Mesa's lavapipe.
