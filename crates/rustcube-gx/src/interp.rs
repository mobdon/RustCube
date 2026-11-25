//! Frames between game frames: the game runs at 60 a second, a monitor may
//! refresh faster. Each draw of the newest frame is matched to one of the
//! frame before (same material, textures and vertex/index counts; among
//! those, the nearest, and no further than a frame's movement), and its position matrices, which carry both the
//! object's and the camera's movement, are blended between the two. A draw
//! with no match, or one that moved or turned too far in a frame (a camera
//! cut, a teleport), shows as it is.
use crate::api::{FastMap, Frame};
use crate::params::Mtx34;
use std::hash::{Hash, Hasher};

/// A matrix translation that moves more than this in one frame is a jump
/// (camera cut, respawn), not motion to smooth.
const JUMP: f32 = 400.0;
/// A rotation/scale entry that changes more than this in one frame is a cut
/// (the camera looking somewhere else), not a turn to smooth.
const TURN: f32 = 0.5;
/// See [`reach`]: view units a frame, and the share of the distance.
const STEP: f32 = 16.0;
const SWEEP: f32 = 0.25;
/// How many same-looking draws of the frame before are compared to find the
/// nearest (bounds the work for scenes with hundreds of copies).
const CANDIDATES: usize = 48;

/// The matrices to blend from, for each draw of the newest frame.
pub struct Tween {
    from: Vec<Option<[Mtx34; 10]>>,
    to: Vec<[Mtx34; 10]>,
}

/// A draw's look: material and vertex/index counts, with its textures
/// (`textures`) or without. An effect that steps through texture frames (a
/// flame, a jet) has other textures than it had a frame
/// before; with them in the key its only match was another copy of the
/// effect at that texture frame, somewhere else in the room, and the
/// pictures in between showed it on the way there (2026-11-24: the flame
/// off the head and about the room, only on displays above 60).
fn key(d: &crate::api::DrawCmd, textures: bool) -> u64 {
    let mut h = crate::api::FastHasher::default();
    d.material.hash(&mut h);
    d.vertices.len().hash(&mut h);
    d.indices.len().hash(&mut h);
    if textures {
        for m in &d.material_params.texture_mappings {
            m.texture.hash(&mut h);
        }
    }
    h.finish()
}

/// The furthest a draw's place may be from its match's (squared): a fixed
/// step plus a share of its distance from the camera, which is what a
/// camera turn moves it by. Further is another object, not this one a frame
/// earlier, and it shows as it is.
fn reach(to: &[Mtx34; 10]) -> f32 {
    let far: f32 = (0..3).map(|r| to[0][r][3].powi(2)).sum::<f32>().sqrt();
    (STEP + SWEEP * far).powi(2)
}

/// How far the first matrix's translation moved (squared).
fn moved(a: &[Mtx34; 10], b: &[Mtx34; 10]) -> f32 {
    (0..3).map(|r| (a[0][r][3] - b[0][r][3]).powi(2)).sum()
}

fn jumped(a: &[Mtx34; 10], b: &[Mtx34; 10]) -> bool {
    a.iter().zip(b).any(|(a, b)| {
        let d: f32 = (0..3).map(|r| (a[r][3] - b[r][3]).powi(2)).sum();
        let turn = (0..3)
            .flat_map(|r| (0..3).map(move |c| (r, c)))
            .map(|(r, c)| (a[r][c] - b[r][c]).abs())
            .fold(0.0f32, f32::max);
        d.is_nan() || d > JUMP * JUMP || turn.is_nan() || turn > TURN
    })
}

impl Tween {
    /// Match `cur`'s draws to `prev`'s.
    pub fn new(prev: &Frame, cur: &Frame) -> Self {
        // By look with textures, and by look alone.
        let mut by_key: [FastMap<u64, Vec<usize>>; 2] = Default::default();
        for (i, d) in prev.draws.iter().enumerate() {
            by_key[0].entry(key(d, true)).or_default().push(i);
            by_key[1].entry(key(d, false)).or_default().push(i);
        }
        let mut used = vec![false; prev.draws.len()];
        let to: Vec<[Mtx34; 10]> = cur.draws.iter().map(|d| d.draw_params.pos_mtx).collect();
        let from = cur
            .draws
            .iter()
            .zip(&to)
            .map(|(d, to)| {
                // The nearest of the first candidates still unmatched, so a
                // copy that vanished doesn't shift every later copy onto its
                // neighbour. Same textures first; the same look with other
                // textures only when that one is nearer.
                let nearest = |bucket: Option<&Vec<usize>>, used: &[bool]| {
                    bucket?
                        .iter()
                        .filter(|&&i| !used[i])
                        .take(CANDIDATES)
                        .map(|&i| (i, moved(&prev.draws[i].draw_params.pos_mtx, to)))
                        .min_by(|a, b| a.1.total_cmp(&b.1))
                };
                let same = nearest(by_key[0].get(&key(d, true)), &used);
                let look = nearest(by_key[1].get(&key(d, false)), &used);
                let (i, far) = match (same, look) {
                    (Some(a), Some(b)) => if b.1 < a.1 { b } else { a },
                    (a, b) => a.or(b)?,
                };
                if !(far <= reach(to)) {
                    return None;
                }
                used[i] = true;
                let from = prev.draws[i].draw_params.pos_mtx;
                (!jumped(&from, to)).then_some(from)
            })
            .collect();
        Self { from, to }
    }

    /// Draws that will be blended (for tests and the profiler).
    pub fn matched(&self) -> usize {
        self.from.iter().flatten().count()
    }

    /// Set `cur`'s position matrices to `t` of the way from the previous
    /// frame's (0) to its own (1). `cur` must be the frame given to `new`.
    pub fn apply(&self, cur: &mut Frame, t: f32) {
        let t = t.clamp(0.0, 1.0);
        for ((d, from), to) in cur.draws.iter_mut().zip(&self.from).zip(&self.to) {
            d.draw_params.pos_mtx = match from {
                Some(from) if t < 1.0 => std::array::from_fn(|k| {
                    std::array::from_fn(|r| {
                        std::array::from_fn(|c| from[k][r][c] + (to[k][r][c] - from[k][r][c]) * t)
                    })
                }),
                _ => *to,
            };
        }
    }
}
