# Third-party code in rustcube-gx

## noclip.website

Most of this crate is a Rust translation of noclip.website's generic GameCube
GX rendering code. Only generic GX code was used, never noclip's code for a
specific game.

- Source: https://github.com/magcius/noclip.website
- Commit: `2cebac2a5d1d7c44535397a14f51e764ca823767` (2026-11-14)
- Author: Jasper St. Pierre and noclip.website contributors

| rustcube-gx file | noclip source |
|---|---|
| `src/enums.rs` | `src/gx/gx_enum.ts` |
| `src/vtx.rs` | `src/gx/gx_displaylist.ts` |
| `src/texture.rs` | `rust/src/gx_texture.rs`, `src/gx/gx_texture.ts` |
| `src/material.rs` | `src/gx/gx_material.ts`, `src/gx/GXMaterialBuilder.ts`, `src/gx/gx_render.ts` |
| `src/shader.rs` | `src/gx/gx_material.ts` (GX_Program, GLSL rewritten as WGSL) |
| `src/params.rs` | `src/gx/gx_render.ts` |
| `src/gpu.rs` | `src/gx/gx_render.ts` (noclip's Gfx layer replaced by wgpu) |
| `src/api.rs` | parts of `src/gx/DDraw.ts` and `src/gx/GXMaterialBuilder.ts`; the GX API layer is new |

Each of these files starts with a notice naming its source and this license.

noclip's own LICENSE notes that some of its files re-implement reverse-engineered
game software and invites questions about which files are clean. The files
above are its generic GX hardware layer (constants, vertex loading, texture
formats, TEV shader generation), not any game's code.

### License

MIT License.

Some files written specifically for the noclip project have not been reverse engineered,
and are available solely under the MIT License as displayed below. Please contact
Jasper St. Pierre at jstpierre@mecheye.net for further details on which source files are
completely clean from a reverse-engineering standpoint, and which are not.



MIT License

Copyright (c) 2018 Jasper St. Pierre

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
