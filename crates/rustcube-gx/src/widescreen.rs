//! Widescreen: the game draws for 4:3. For a wider window the 3D view gets a
//! wider horizontal field of view (more is seen, nothing is stretched), and
//! the HUD keeps its proportions: an element in the left or right third moves
//! to that edge, one in the middle stays centred, and one spanning the whole
//! screen (a backdrop, a fade) is stretched to cover it.
use crate::api::{DrawCmd, Frame};
use crate::vtx::{InputFormat, VertexInput};

/// How far toward the screen edge side HUD moves, as a share of the extra
/// width. Tuned on the first game ported (its official 16:9 PC release puts
/// the HUD about 8.2% in from each side where the 4:3 layout has 2.5%);
/// another game may want its own value.
pub const HUD_EDGE: f32 = 0.494;

/// How the HUD's screen-space sprites are drawn: a view matrix that maps
/// pixel coordinates (about 1/457 a pixel) with no translation.
pub fn is_hud(d: &DrawCmd) -> bool {
    let m = d.draw_params.pos_mtx[0];
    !d.material.use_pn_mtx_idx
        && m[0][3] == 0.0
        && m[1][3] == 0.0
        && m[2] == [0.0, 0.0, 1.0, 0.0]
        && m[0][0] != 0.0
        && m[0][0].abs() < 0.01
        && m[1][1].abs() < 0.01
}

/// Where the draw lands on screen: normalised (min x, max x, min y, max y).
pub fn extent(d: &DrawCmd) -> Option<(f32, f32, f32, f32)> {
    let pos = d.layout.input(VertexInput::Pos)?;
    if pos.format != InputFormat::F32(4) && pos.format != InputFormat::F32(3) {
        return None;
    }
    let stride = d.layout.stride.max(1);
    let m = d.draw_params.pos_mtx[0];
    let p = d.projection;
    let f = |b: &[u8], o: usize| {
        b.get(o..o + 4)
            .map(|x| f32::from_le_bytes([x[0], x[1], x[2], x[3]]))
    };
    let mut out: Option<(f32, f32, f32, f32)> = None;
    for v in d.vertices.chunks(stride) {
        let (x, y, z) = (
            f(v, pos.offset)?,
            f(v, pos.offset + 4)?,
            f(v, pos.offset + 8)?,
        );
        let view: [f32; 3] =
            std::array::from_fn(|r| m[r][0] * x + m[r][1] * y + m[r][2] * z + m[r][3]);
        let clip = |row: usize| {
            p[0][row] * view[0] + p[1][row] * view[1] + p[2][row] * view[2] + p[3][row]
        };
        let w = clip(3);
        if w.abs() < 1e-6 {
            continue;
        }
        let (nx, ny) = (clip(0) / w, clip(1) / w);
        out = Some(out.map_or((nx, nx, ny, ny), |(a, b, c, e)| {
            (a.min(nx), b.max(nx), c.min(ny), e.max(ny))
        }));
    }
    out
}

/// Set clip-space x to `scale * x + shift` (normalised), whatever the
/// projection (column-major).
fn remap_x(p: &mut [[f32; 4]; 4], scale: f32, shift: f32) {
    for col in p.iter_mut() {
        col[0] = col[0] * scale + shift * col[3];
    }
}

/// Fit `frame`, drawn for 4:3, to a target `aspect` (width / height) wider
/// than 4:3. Passes drawn into a texture (EFB copies) are left alone.
pub fn widen(frame: &mut Frame, aspect: f32) {
    let k = (4.0 / 3.0) / aspect;
    if !(k > 0.1 && k < 0.999) {
        return;
    }
    let first = frame.tex_copies.iter().map(|c| c.draw).max().unwrap_or(0);
    // Only full-screen views (not split screen or a small window).
    let on_screen = |d: &DrawCmd| d.viewport[2] >= 600.0 && d.viewport[3] >= 440.0;
    // A screen of 2D art only (title, menus: sprites and view-space quads, no
    // 3D scene) is shown at its own proportions in the middle (pillarboxed):
    // its art is 4:3, and parts the game draws past the 4:3 edges stay cut
    // off as on a GameCube.
    let flat = frame
        .draws
        .iter()
        .skip(first)
        .filter(|d| on_screen(d))
        .all(|d| is_hud(d) || is_view_space(d));
    let spans = |d: &DrawCmd| extent(d).is_some_and(|(a, b, _, _)| a <= -0.95 && b >= 0.95);
    let textured = |d: &DrawCmd| {
        d.material_params
            .texture_mappings
            .iter()
            .any(|m| m.texture.is_some())
    };
    // A movie frame: a big picture across the screen. Overlays across it
    // (tints, letterbox bars) stay with the picture.
    let movie = frame
        .draws
        .iter()
        .skip(first)
        .any(|d| is_hud(d) && spans(d) && d.material_params.texture_mappings[0].width >= 256.0);
    let centres = group_centres(frame, first);
    let block = centred_block(&centres);
    // HUD drawn before the last 3D draw marks something in the scene (a
    // lock-on reticle at its target's screen spot): it follows the 3D view.
    let last_3d = frame
        .draws
        .iter()
        .rposition(|d| on_screen(d) && !is_hud(d) && !is_view_space(d))
        .unwrap_or(0);
    for (i, d) in frame.draws.iter_mut().enumerate().skip(first) {
        if !on_screen(d) {
            continue;
        }
        let ext = (is_hud(d) || flat).then(|| extent(d)).flatten();
        let span = ext.is_some_and(|(a, b, _, _)| a <= -0.95 && b >= 0.95);
        // A fade or flash covers the whole screen, top to bottom.
        let fade = span && !textured(d) && ext.is_some_and(|(_, _, t, b)| b - t >= 1.9);
        if flat || (span && is_hud(d) && (movie || textured(d))) {
            if fade && !flat {
                // A fade or flash over a movie: across the whole window (over 2D art it
                // stays with the art, so a white flash leaves the side bars black).
                remap_x(&mut d.projection, k, 0.0);
                let m = &mut d.draw_params.pos_mtx[0];
                m[0] = m[0].map(|v| v / k);
            } else {
                // 2D art (a menu, a movie frame): 4:3 in the middle.
                pillarbox(d, k);
            }
            continue;
        }
        // Every other screen draw gets the same wider projection (draws with
        // different projections can't share a render pass, and a pass per HUD
        // element cost a fifth of the window's time); a HUD element's move to
        // its edge or stretch goes into its own position matrix instead.
        remap_x(&mut d.projection, k, 0.0);
        let Some((a, b, top, bottom)) = ext.filter(|_| is_hud(d)) else {
            continue;
        };
        let m = &mut d.draw_params.pos_mtx[0];
        if (a <= -0.95 && b >= 0.95) || bottom - top >= 1.8 {
            // Spans the screen across or top to bottom (a backdrop or one of
            // its tiles, a fade): stretch it to the full width, every piece
            // the same way so the tiles stay joined.
            m[0] = m[0].map(|v| v / k);
            continue;
        }
        if i < last_3d {
            continue;
        }
        // Where its group sits decides (a row of glyphs moves as one).
        let (c, span) = centres.get(i).copied().flatten().unwrap_or(((a + b) / 2.0, (a, b, top, bottom)));
        // Text laid over a centred panel stays with the panel (the result screen's labels
        // and numbers start and end at its edges: moved to the screen edges they hung
        // out of it on both sides): a group whose middle is over the panels' width and
        // that shares some height with them.
        if block.is_some_and(|(l, r, t, btm)| (l..=r).contains(&c) && span.2 <= btm && span.3 >= t) {
            continue;
        }
        let edge = (1.0 - k) * HUD_EDGE;
        let shift = if c < -1.0 / 3.0 {
            -edge
        } else if c > 1.0 / 3.0 {
            edge
        } else {
            continue;
        };
        // Normalised x moves by `shift`: clip x gains shift * w, and for a
        // perspective (w = -z_view) that is x_view - shift / p00 * z_view.
        let p00 = d.projection[0][0];
        let w_z = d.projection[2][3];
        if p00 != 0.0 {
            let coef = shift * w_z / p00;
            let z = m[2];
            for (x, z) in m[0].iter_mut().zip(z) {
                *x += coef * z;
            }
        }
    }
}

/// The centre of each HUD piece's group: consecutive pieces side by side on a
/// row (overlapping vertically, at most a small gap apart) form one, such as
/// the digits of a time or a label and its value. Anchoring by each glyph's
/// own centre split "05:21:02" between the middle and the right edge.
fn group_centres(frame: &Frame, first: usize) -> Vec<Option<(f32, (f32, f32, f32, f32))>> {
    const GAP: f32 = 0.12;
    let piece = |d: &DrawCmd| {
        extent(d).filter(|&(a, b, t, btm)| {
            is_hud(d) && !(a <= -0.95 && b >= 0.95) && btm - t < 1.8
        })
    };
    let mut out = vec![None; frame.draws.len()];
    let mut group: Vec<usize> = Vec::new();
    let mut span = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let close = |g: &mut Vec<usize>, s: (f32, f32, f32, f32), out: &mut Vec<Option<(f32, (f32, f32, f32, f32))>>| {
        for &j in g.iter() {
            out[j] = Some(((s.0 + s.1) / 2.0, s));
        }
        g.clear();
    };
    for (i, d) in frame.draws.iter().enumerate().skip(first) {
        let Some((a, b, t, btm)) = piece(d) else {
            continue;
        };
        let joins = !group.is_empty()
            && t <= span.3
            && btm >= span.2
            && a <= span.1 + GAP
            && b >= span.0 - GAP;
        if joins {
            span = (span.0.min(a), span.1.max(b), span.2.min(t), span.3.max(btm));
        } else {
            close(&mut group, span, &mut out);
            span = (a, b, t, btm);
        }
        group.push(i);
    }
    close(&mut group, span, &mut out);
    out
}

/// The screen area of the centred HUD panels (min x, max x, min y, max y, normalised), widened
/// by a group gap to each side: groups of pieces (a panel may be drawn in parts) between 30%
/// and 80% of the 4:3 width whose middle is within 7.5% of the screen's. The result screen's
/// two score panels are 68% wide. None without one.
/// Shortcut: sizes read off the result screen, not a game value; a centred panel outside
/// them keeps the old behaviour, add its size here when one turns up.
fn centred_block(groups: &[Option<(f32, (f32, f32, f32, f32))>]) -> Option<(f32, f32, f32, f32)> {
    const GAP: f32 = 0.12;
    groups
        .iter()
        .flatten()
        .filter(|&&(c, (a, b, _, _))| (0.6..=1.6).contains(&(b - a)) && c.abs() <= 0.15)
        .fold(None, |acc: Option<(f32, f32, f32, f32)>, &(_, (a, b, t, btm))| {
            Some(acc.map_or((a - GAP, b + GAP, t, btm), |(l, r, tt, bb)| (l.min(a - GAP), r.max(b + GAP), tt.min(t), bb.max(btm))))
        })
}

/// Drawn with no view transform (vertices already in view space): 2D quads.
pub(crate) fn is_view_space(d: &DrawCmd) -> bool {
    !d.material.use_pn_mtx_idx
        && d.draw_params.pos_mtx[0]
            == [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
            ]
}

/// Draw `d` unchanged into the middle 4:3 of the window (its viewport is in
/// 640-wide EFB units spread across the window's width).
fn pillarbox(d: &mut DrawCmd, k: f32) {
    let full = 640.0;
    d.viewport[0] = d.viewport[0] * k + full * (1.0 - k) / 2.0;
    d.viewport[2] *= k;
}

/// Whether every vertex's first texture coordinate lies within the texture
/// (a sprite showing its texture once, not a tiled backdrop).
fn within_texture(d: &DrawCmd) -> bool {
    let Some(t) = d.layout.input(VertexInput::Tex01) else {
        return false;
    };
    let stride = d.layout.stride.max(1);
    let f = |b: &[u8], o: usize| {
        b.get(o..o + 4)
            .map(|x| f32::from_le_bytes([x[0], x[1], x[2], x[3]]))
    };
    d.vertices.chunks(stride).all(|v| {
        [f(v, t.offset), f(v, t.offset + 4)]
            .into_iter()
            .all(|c| c.is_some_and(|c| (-0.01..=1.01).contains(&c)))
    })
}

/// HUD sprites that show their texture once sample it clamped. They're drawn
/// with repeat, and at more pixels than 640x480 the filter at a sprite's
/// edge blended in the texture's opposite edge: thin lines around the lives
/// icon and menu items. Tiled HUD backdrops keep repeat.
pub fn clamp_hud_textures(frame: &mut Frame) {
    for d in frame
        .draws
        .iter_mut()
        .filter(|d| is_hud(d) && within_texture(d))
    {
        for m in &mut d.material_params.texture_mappings {
            m.sampler.wrap_s = crate::enums::wrap_mode::CLAMP;
            m.sampler.wrap_t = crate::enums::wrap_mode::CLAMP;
        }
    }
}
