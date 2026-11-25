//! Offscreen snapshots of the game's GX frames for headless runs: every
//! `every`-th frame is drawn at 640x480 and written as a PNG. Texture uploads
//! between snapshots are kept as the latest data per texture id and reach
//! the GPU only when a snapshot is drawn.
use rustcube_gx::api::{Frame, TextureUpload};
use rustcube_gx::gpu::GxRenderer;
use rustcube_gx::params::TextureId;
use rustcube_recomp::{global, global::Global};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const W: u32 = 640;
const H: u32 = 480;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

type Levels = Vec<(u32, u32, Vec<u8>)>;

struct Snap {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: GxRenderer,
    target: wgpu::Texture,
    buf: wgpu::Buffer,
    /// Uploads since the last snapshot, newest per id.
    pending: HashMap<TextureId, Levels>,
    /// The frame before a snapshot frame (RUSTCUBE_SNAP_TWEEN).
    before: Option<Frame>,
}

/// Keep the newest data for each uploaded texture. The game re-uploads every
/// texture every frame; making a GPU texture for each (about 160 a frame) grew
/// the process by ~8 MB every 10 s in the driver, outside wgpu's own memory.
fn note_uploads(pending: &mut HashMap<TextureId, Levels>, uploads: &[TextureUpload]) {
    for u in uploads {
        pending.insert(u.id, u.levels.clone());
    }
}

static SNAP: Global<Option<Snap>> = global!(None);

fn open() -> Result<Snap, String> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::LowPower,
        ..Default::default()
    }))
    .map_err(|e| format!("no wgpu adapter: {e}"))?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits:
            wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits()),
        ..Default::default()
    }))
    .map_err(|e| format!("no wgpu device: {e}"))?;
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("snapshot"),
        size: wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("snapshot readback"),
        size: u64::from(W * 4 * H),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let renderer = GxRenderer::new(device.clone(), queue.clone(), FORMAT);
    Ok(Snap {
        device,
        queue,
        renderer,
        target,
        buf,
        pending: HashMap::new(),
        before: None,
    })
}

/// Feed frame number `n`; on every `every`-th frame write `dir/frame_NNNNNN.png`
/// and return its path.
pub fn frame(f: &Frame, n: u64, every: u64, dir: &Path) -> Result<Option<PathBuf>, String> {
    SNAP.with_borrow_mut(|s| {
        if s.is_none() {
            *s = Some(open()?);
        }
        let Some(s) = s.as_mut() else { return Ok(None) };
        note_uploads(&mut s.pending, &f.uploads);
        let tween = std::env::var_os("RUSTCUBE_SNAP_TWEEN").is_some();
        // RUSTCUBE_FRAMES_RANGE=a-b: only frames a..=b (with RUSTCUBE_FRAMES_EVERY=1,
        // every frame there: flicker shows only in consecutive frames).
        let in_range = std::env::var("RUSTCUBE_FRAMES_RANGE")
            .ok()
            .and_then(|r| {
                let (a, b) = r.split_once('-')?;
                Some((a.trim().parse::<u64>().ok()?, b.trim().parse::<u64>().ok()?))
            })
            .is_none_or(|(a, b)| (a..=b).contains(&n));
        if every == 0 || !n.is_multiple_of(every) || !in_range {
            if tween && every != 0 && (n + 1).is_multiple_of(every) {
                s.before = Some(f.clone());
            }
            return Ok(None);
        }
        for (id, levels) in s.pending.drain() {
            s.renderer.replace_texture(id, &levels);
        }
        if std::env::var_os("RUSTCUBE_SNAP_INFO").is_some() {
            eprintln!("frame {n}: clear {:?}, {} draws, {} uploads", f.clear_color, f.draws.len(), f.uploads.len());
            let mut seen = std::collections::BTreeSet::new();
            for d in &f.draws {
                let key = format!("{:?} {:?}", d.viewport, d.projection);
                if seen.insert(key) && seen.len() <= 6 {
                    eprintln!("  viewport {:?} scissor {:?} proj {:?} verts {}B idx {}", d.viewport, d.scissor, d.projection, d.vertices.len(), d.indices.len());
                }
            }
            eprintln!("  {} distinct viewport/projection pairs", seen.len());
        }
        // RUSTCUBE_SNAP_2D: the last 40 draws (the HUD) and their textures.
        if std::env::var_os("RUSTCUBE_SNAP_2D").is_some() {
            for (i, d) in f.draws.iter().enumerate().skip(f.draws.len().saturating_sub(40)) {
                for m in d.material_params.texture_mappings.iter().filter(|m| m.texture.is_some()) {
                    eprintln!(
                        "frame {n} 2d draw {i}: {} verts, tex {:?} {}x{} wrap {}/{} filter {}/{} lod {}..{}",
                        d.indices.len(),
                        m.texture,
                        m.width,
                        m.height,
                        m.sampler.wrap_s,
                        m.sampler.wrap_t,
                        m.sampler.min_filter,
                        m.sampler.mag_filter,
                        m.sampler.min_lod_x16,
                        m.sampler.max_lod_x16
                    );
                }
                eprintln!("frame {n} 2d draw {i}: mtx0 {:?} proj {:?} vp {:?} pn {}", d.draw_params.pos_mtx[0], d.projection, d.viewport, d.material.use_pn_mtx_idx);
            }
        }
        // RUSTCUBE_SNAP_WIDE: how widescreen sees each draw (HUD or 3D, where it
        // lands, its texture size), the frame drawn widened for 16:9 (squeezed
        // into the 4:3 snapshot, so gaps show as black).
        let mut wide;
        let f = if std::env::var_os("RUSTCUBE_SNAP_WIDE").is_some() {
            for (i, d) in f.draws.iter().enumerate() {
                let tex = d.material_params.texture_mappings[0];
                eprintln!(
                    "wide frame {n} draw {i}: hud {} extent {:?} tex {:?} {}x{} vp {:?} mtx {:?} proj00 {}",
                    rustcube_gx::widescreen::is_hud(d),
                    rustcube_gx::widescreen::extent(d),
                    tex.texture,
                    tex.width,
                    tex.height,
                    d.viewport,
                    d.draw_params.pos_mtx[0],
                    d.projection[0][0]
                );
            }
            wide = f.clone();
            rustcube_gx::widescreen::widen(&mut wide, 16.0 / 9.0);
            rustcube_gx::widescreen::clamp_hud_textures(&mut wide);
            &wide
        } else {
            f
        };
        // RUSTCUBE_SNAP_ROP: blend, alpha-test and depth state of the frame's draws,
        // grouped (which kinds of draw a frame is made of).
        if std::env::var_os("RUSTCUBE_SNAP_ROP").is_some() {
            let mut kinds: std::collections::BTreeMap<String, usize> = Default::default();
            for d in &f.draws {
                let m = &d.material;
                let tex = d.material_params.texture_mappings[0];
                let k = format!(
                    "view-space {} rop {:?} alpha {:?} stages {} tex0 {}x{}",
                    d.draw_params.pos_mtx[0][0] == [1.0, 0.0, 0.0, 0.0],
                    m.rop,
                    m.alpha_test,
                    m.tev_stages.len(),
                    tex.width,
                    tex.height
                );
                *kinds.entry(k).or_default() += 1;
            }
            for (k, n) in kinds {
                eprintln!("rop: {n} x {k}");
            }
        }
        // RUSTCUBE_SNAP_COPIES: the frame's EFB copies and every draw sampling one.
        if std::env::var_os("RUSTCUBE_SNAP_COPIES").is_some() {
            for c in &f.tex_copies {
                eprintln!("frame {n}: copy {c:?}");
            }
            let ids: Vec<_> = f.tex_copies.iter().map(|c| c.id).collect();
            for (i, d) in f.draws.iter().enumerate() {
                let maps = &d.material_params.texture_mappings;
                let Some(map) = maps.iter().position(|m| m.texture.is_some_and(|t| ids.contains(&t))) else {
                    continue;
                };
                eprintln!("  draw {i} samples copy at map {map}: texgens {:?}", d.material.tex_gens);
                eprintln!("    tev {:?}", d.material.tev_stages.iter().map(|s| (s.tex_coord_id, s.tex_map)).collect::<Vec<_>>());
                eprintln!("    pos_mtx0 {:?}", d.draw_params.pos_mtx[0]);
                eprintln!("    tex_mtx {:?}", &d.material_params.tex_mtx[..3]);
                eprintln!("    post_tex_mtx {:?}", &d.material_params.post_tex_mtx[..3]);
                eprintln!("    proj {:?} viewport {:?}", d.projection, d.viewport);
            }
        }
        let px = shoot(s, f)?;
        // RUSTCUBE_SNAP_CHECK: draw the frame again (from the renderer's pass
        // cache, as the window does between game frames) and count pixels
        // that differ from the first drawing.
        if std::env::var_os("RUSTCUBE_SNAP_CHECK").is_some() {
            let again = shoot(s, f)?;
            let diff = px.chunks(4).zip(again.chunks(4)).filter(|(a, b)| a != b).count();
            eprintln!("snapcheck frame {n}: {diff} pixels differ when drawn again");
        }
        // RUSTCUBE_GPU_REPORT: what the snapshot device holds (finding leaks).
        if std::env::var_os("RUSTCUBE_GPU_REPORT").is_some() {
            let [p, t, sm, b] = s.renderer.cache_counts();
            let a = s.device.generate_allocator_report();
            let (alloc, reserved, blocks, count) = a.map_or((0, 0, 0, 0), |a| {
                (a.total_allocated_bytes >> 20, a.total_reserved_bytes >> 20, a.blocks.len(), a.allocations.len())
            });
            eprintln!("gpu: frame {n} pipelines {p} textures {t} samplers {sm} binds {b} | allocated {alloc} MB reserved {reserved} MB in {blocks} blocks, {count} allocations");
        }
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let path = dir.join(format!("frame_{n:06}.png"));
        save_png(&path, &px)?;
        // RUSTCUBE_SNAP_TWEEN: also the picture halfway from the frame before, as
        // the window blends it above 60 a second.
        if let Some(before) = s.before.take().filter(|_| tween) {
            let tw = rustcube_gx::interp::Tween::new(&before, f);
            let mut mid = f.clone();
            mid.serial = 0;
            tw.apply(&mut mid, 0.5);
            eprintln!("tween frame {n}: {} of {} draws blended", tw.matched(), f.draws.len());
            let px = shoot(s, &mid)?;
            save_png(&dir.join(format!("frame_{n:06}_half.png")), &px)?;
            let px = shoot(s, &before)?;
            save_png(&dir.join(format!("frame_{n:06}_before.png")), &px)?;
        }
        Ok(Some(path))
    })
}

fn save_png(path: &Path, px: &[u8]) -> Result<(), String> {
    let file = std::io::BufWriter::new(std::fs::File::create(path).map_err(|e| e.to_string())?);
    let mut enc = png::Encoder::new(file, W, H);
    // RGB: the EFB has no alpha the game cares about (it writes 0).
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(|e| e.to_string())?;
    let rgb: Vec<u8> = px
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    w.write_image_data(&rgb).map_err(|e| e.to_string())
}

/// Draw `f` into the snapshot target and read the pixels back.
fn shoot(s: &mut Snap, f: &Frame) -> Result<Vec<u8>, String> {
    let view = s.target.create_view(&Default::default());
    s.renderer.render_frame(f, &view, [W, H]);
    let mut enc = s.device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        s.target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &s.buf,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(W * 4),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d {
            width: W,
            height: H,
            depth_or_array_layers: 1,
        },
    );
    s.queue.submit([enc.finish()]);
    s.buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    s.device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| e.to_string())?;
    let px = s
        .buf
        .slice(..)
        .get_mapped_range()
        .map_err(|e| format!("{e:?}"))?
        .to_vec();
    s.buf.unmap();
    Ok(px)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn upload(id: TextureId, v: u8) -> TextureUpload {
        TextureUpload {
            id,
            levels: vec![(1, 1, vec![v; 4])],
        }
    }

    #[test]
    fn pending_uploads_stay_one_per_texture() {
        // 1000 frames re-uploading the same 3 textures, as the game does.
        let mut pending = HashMap::new();
        for frame in 0..1000u32 {
            let v = (frame % 256) as u8;
            note_uploads(&mut pending, &[upload(1, v), upload(2, v), upload(7, v)]);
        }
        assert_eq!(pending.len(), 3);
        assert_eq!(
            pending[&7][0].2,
            vec![(999 % 256) as u8; 4],
            "the newest data wins"
        );
    }
}
