// Derived from noclip.website (https://github.com/magcius/noclip.website),
// rust/src/gx_texture.rs and src/gx/gx_texture.ts at commit
// 2cebac2a5d1d7c44535397a14f51e764ca823767.
// Copyright (c) 2018 Jasper St. Pierre. MIT License: see THIRD-PARTY.md.
// Translated from TypeScript to Rust for rustcube-gx (the decoders were already Rust).
//! GX texture decoding: every GX texel format (I4, I8, IA4, IA8, RGB565,
//! RGB5A3, RGBA8, CMPR, and the C4/C8/C14X2 palette formats with IA8,
//! RGB565 and RGB5A3 TLUTs) to RGBA8, plus size and mip-chain helpers.
use crate::enums::{tex_format, tex_palette};

/// One texture as GX describes it (`GXInitTexObj` + `GXInitTlutObj`).
#[derive(Debug, Clone, Copy)]
pub struct TextureInput<'a> {
    pub format: u32,
    pub width: u32,
    pub height: u32,
    /// Texel data, starting at mip level 0.
    pub data: &'a [u8],
    pub mip_count: u32,
    pub palette_format: u32,
    /// Big-endian 16-bit palette entries (C4/C8/C14X2 only).
    pub palette: &'a [u8],
}

fn expand_n_to_8(n: u32, v: u8) -> u8 {
    match n {
        3 => (v << 5) | (v << 2) | (v >> 1),
        4 => (v << 4) | v,
        5 => (v << 3) | (v >> 2),
        6 => (v << 2) | (v >> 4),
        _ => v,
    }
}

fn be16(src: &[u8], at: usize) -> u16 {
    match src.get(at..at + 2) {
        Some(b) => u16::from_be_bytes([b[0], b[1]]),
        None => 0,
    }
}

fn byte(src: &[u8], at: usize) -> u8 {
    src.get(at).copied().unwrap_or(0)
}

fn s3tcblend(a: u8, b: u8) -> u8 {
    // (a*3 + b*5) / 8
    let (a, b) = (u32::from(a), u32::from(b));
    ((((a << 1) + a) + ((b << 2) + b)) >> 3) as u8
}

fn halfblend(a: u8, b: u8) -> u8 {
    ((u32::from(a) + u32::from(b)) >> 1) as u8
}

fn rgb5a3(dst: &mut [u8], p: u16) {
    if p & 0x8000 != 0 {
        dst[0] = expand_n_to_8(5, ((p >> 10) & 0x1F) as u8);
        dst[1] = expand_n_to_8(5, ((p >> 5) & 0x1F) as u8);
        dst[2] = expand_n_to_8(5, (p & 0x1F) as u8);
        dst[3] = 0xFF;
    } else {
        dst[0] = expand_n_to_8(4, ((p >> 8) & 0x0F) as u8);
        dst[1] = expand_n_to_8(4, ((p >> 4) & 0x0F) as u8);
        dst[2] = expand_n_to_8(4, (p & 0x0F) as u8);
        dst[3] = expand_n_to_8(3, ((p >> 12) & 0x07) as u8);
    }
}

fn rgb565(dst: &mut [u8], p: u16) {
    dst[0] = expand_n_to_8(5, ((p >> 11) & 0x1F) as u8);
    dst[1] = expand_n_to_8(6, ((p >> 5) & 0x3F) as u8);
    dst[2] = expand_n_to_8(5, (p & 0x1F) as u8);
    dst[3] = 0xFF;
}

/// Walk 2D tiles of `bw`×`bh` texels in GX order, calling `px(index, dst)`
/// for each texel inside the image.
fn decode_tiled(
    w: usize,
    h: usize,
    bw: usize,
    bh: usize,
    mut px: impl FnMut(usize, &mut [u8]),
) -> Vec<u8> {
    let mut idx = 0usize;
    let mut dst = vec![0u8; w * h * 4];
    for yy in (0..h).step_by(bh) {
        for xx in (0..w).step_by(bw) {
            for y in 0..bh {
                for x in 0..bw {
                    if xx + x < w && yy + y < h {
                        let o = ((yy + y) * w + (xx + x)) * 4;
                        px(idx, &mut dst[o..o + 4]);
                    }
                    idx += 1;
                }
            }
        }
    }
    dst
}

fn decode_rgba8(src: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut so = 0usize;
    let mut dst = vec![0u8; w * h * 4];
    for yy in (0..h).step_by(4) {
        for xx in (0..w).step_by(4) {
            // AR plane, then GB plane.
            for plane in 0..2 {
                for y in 0..4 {
                    for x in 0..4 {
                        if xx + x < w && yy + y < h {
                            let o = ((yy + y) * w + (xx + x)) * 4;
                            if plane == 0 {
                                dst[o + 3] = byte(src, so);
                                dst[o] = byte(src, so + 1);
                            } else {
                                dst[o + 1] = byte(src, so);
                                dst[o + 2] = byte(src, so + 1);
                            }
                        }
                        so += 2;
                    }
                }
            }
        }
    }
    dst
}

fn decode_cmpr(src: &[u8], w: usize, h: usize) -> Vec<u8> {
    // CMPR swizzles 4x4 DXT1 blocks into 8x8 macroblocks (UL, UR, BL, BR).
    let mut so = 0usize;
    let mut dst = vec![0u8; w * h * 4];
    for yy in (0..h).step_by(8) {
        for xx in (0..w).step_by(8) {
            for yb in (0..8).step_by(4) {
                for xb in (0..8).step_by(4) {
                    let b = so;
                    so += 8;
                    // Big-endian colors, unlike DXT1.
                    let c1 = be16(src, b);
                    let c2 = be16(src, b + 2);
                    let mut t = [0u8; 16];
                    rgb565(&mut t[0..4], c1);
                    rgb565(&mut t[4..8], c2);
                    if c1 > c2 {
                        for k in 0..3 {
                            t[8 + k] = s3tcblend(t[4 + k], t[k]);
                            t[12 + k] = s3tcblend(t[k], t[4 + k]);
                        }
                        t[11] = 0xFF;
                        t[15] = 0xFF;
                    } else {
                        for k in 0..3 {
                            t[8 + k] = halfblend(t[k], t[4 + k]);
                            // GX fills the fourth entry with the midpoint
                            // at alpha 0.
                            t[12 + k] = t[8 + k];
                        }
                        t[11] = 0xFF;
                        t[15] = 0x00;
                    }
                    for y in 0..4 {
                        let mut bits = byte(src, b + 4 + y);
                        for x in 0..4 {
                            if xx + xb + x < w && yy + yb + y < h {
                                let o = ((yy + yb + y) * w + (xx + xb + x)) * 4;
                                let ci = usize::from((bits >> 6) & 3) * 4;
                                dst[o..o + 4].copy_from_slice(&t[ci..ci + 4]);
                            }
                            bits <<= 2;
                        }
                    }
                }
            }
        }
    }
    dst
}

/// A TLUT (big-endian 16-bit entries) as RGBA8.
pub fn decode_palette(palette_format: u32, src: &[u8]) -> Vec<u8> {
    let n = src.len() / 2;
    let mut dst = vec![0u8; n * 4];
    for i in 0..n {
        let d = &mut dst[i * 4..i * 4 + 4];
        match palette_format {
            tex_palette::IA8 => {
                let (a, l) = (src[i * 2], src[i * 2 + 1]);
                d.copy_from_slice(&[l, l, l, a]);
            }
            tex_palette::RGB565 => rgb565(d, be16(src, i * 2)),
            _ => rgb5a3(d, be16(src, i * 2)),
        }
    }
    dst
}

fn pal(p: &[u8], i: usize, dst: &mut [u8]) {
    match p.get(i * 4..i * 4 + 4) {
        Some(c) => dst.copy_from_slice(c),
        None => dst.copy_from_slice(&[0, 0, 0, 0]),
    }
}

/// Decode one mip level (`t.width`×`t.height` at `t.data`) to RGBA8.
pub fn decode_texture(t: &TextureInput<'_>) -> Vec<u8> {
    let (w, h, src) = (t.width as usize, t.height as usize, t.data);
    match t.format {
        tex_format::I4 => decode_tiled(w, h, 8, 8, |i, d| {
            let v = expand_n_to_8(
                4,
                (byte(src, i >> 1) >> if i & 1 != 0 { 0 } else { 4 }) & 0xF,
            );
            d.copy_from_slice(&[v, v, v, v]);
        }),
        tex_format::I8 => decode_tiled(w, h, 8, 4, |i, d| {
            let v = byte(src, i);
            d.copy_from_slice(&[v, v, v, v]);
        }),
        tex_format::IA4 => decode_tiled(w, h, 8, 4, |i, d| {
            let ia = byte(src, i);
            let a = expand_n_to_8(4, ia >> 4);
            let l = expand_n_to_8(4, ia & 0xF);
            d.copy_from_slice(&[l, l, l, a]);
        }),
        tex_format::IA8 => decode_tiled(w, h, 4, 4, |i, d| {
            let (a, l) = (byte(src, i * 2), byte(src, i * 2 + 1));
            d.copy_from_slice(&[l, l, l, a]);
        }),
        tex_format::RGB565 => decode_tiled(w, h, 4, 4, |i, d| rgb565(d, be16(src, i * 2))),
        tex_format::RGB5A3 => decode_tiled(w, h, 4, 4, |i, d| rgb5a3(d, be16(src, i * 2))),
        tex_format::RGBA8 => decode_rgba8(src, w, h),
        tex_format::CMPR => decode_cmpr(src, w, h),
        tex_format::C4 => {
            let p = decode_palette(t.palette_format, t.palette);
            decode_tiled(w, h, 8, 8, |i, d| {
                let k = (byte(src, i >> 1) >> if i & 1 != 0 { 0 } else { 4 }) & 0xF;
                pal(&p, usize::from(k), d)
            })
        }
        tex_format::C8 => {
            let p = decode_palette(t.palette_format, t.palette);
            decode_tiled(w, h, 8, 4, |i, d| pal(&p, usize::from(byte(src, i)), d))
        }
        tex_format::C14X2 => {
            let p = decode_palette(t.palette_format, t.palette);
            decode_tiled(w, h, 4, 4, |i, d| {
                pal(&p, usize::from(be16(src, i * 2) & 0x3FFF), d)
            })
        }
        // Unknown format: opaque white, like noclip's dummy decode.
        _ => vec![0xFF; w * h * 4],
    }
}

/// Palette size in bytes for a palette texture format.
pub fn calc_palette_size(format: u32) -> usize {
    let entries = match format {
        tex_format::C4 => 16,
        tex_format::C8 => 256,
        tex_format::C14X2 => 16384,
        _ => 0,
    };
    entries * 2
}

/// (block width, block height, bytes-per-pixel shift; positive = pixels per
/// byte shift, negative = bytes per pixel shift).
fn block_info(format: u32) -> (u32, u32, i32) {
    match format {
        tex_format::I4 | tex_format::C4 | tex_format::CMPR => (8, 8, 1),
        tex_format::I8 | tex_format::IA4 | tex_format::C8 => (8, 4, 0),
        tex_format::RGBA8 => (4, 4, -2),
        _ => (4, 4, -1),
    }
}

fn align(v: u32, a: u32) -> u32 {
    v.div_ceil(a) * a
}

/// Bytes one mip level takes in GX memory.
pub fn calc_texture_size(format: u32, width: u32, height: u32) -> usize {
    let (bw, bh, shift) = block_info(format);
    let n = (align(width, bw) * align(height, bh)) as usize;
    if shift > 0 {
        n >> shift
    } else {
        n << -shift
    }
}

/// One level of a mip chain: its size and byte offset from level 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MipLevel {
    pub width: u32,
    pub height: u32,
    pub offset: usize,
    pub size: usize,
}

/// The levels of a mip chain (levels are 32-byte aligned). Stops early when
/// a level would run past `data_len` (noclip's guard for short chains).
pub fn calc_mip_chain(
    format: u32,
    width: u32,
    height: u32,
    mip_count: u32,
    data_len: usize,
) -> Vec<MipLevel> {
    let mut out = Vec::new();
    let (mut w, mut h, mut offset) = (width, height, 0usize);
    for _ in 0..mip_count.max(1) {
        let size = calc_texture_size(format, w.max(1), h.max(1));
        if offset + size > data_len {
            break;
        }
        out.push(MipLevel {
            width: w.max(1),
            height: h.max(1),
            offset,
            size,
        });
        offset += size.max(32);
        w /= 2;
        h /= 2;
        if w == 0 && h == 0 {
            break;
        }
    }
    out
}

/// Every level of `t` decoded to RGBA8, largest first.
pub fn decode_mip_chain(t: &TextureInput<'_>) -> Vec<(u32, u32, Vec<u8>)> {
    calc_mip_chain(t.format, t.width, t.height, t.mip_count, t.data.len())
        .into_iter()
        .map(|m| {
            let level = TextureInput {
                width: m.width,
                height: m.height,
                data: &t.data[m.offset..m.offset + m.size],
                ..*t
            };
            (m.width, m.height, decode_texture(&level))
        })
        .collect()
}

/// Display name of a format, e.g. `C8 (RGB5A3)`.
pub fn format_name(format: u32, palette_format: u32) -> String {
    let p = match palette_format {
        tex_palette::IA8 => "IA8",
        tex_palette::RGB565 => "RGB565",
        _ => "RGB5A3",
    };
    match format {
        tex_format::I4 => "I4".into(),
        tex_format::I8 => "I8".into(),
        tex_format::IA4 => "IA4".into(),
        tex_format::IA8 => "IA8".into(),
        tex_format::RGB565 => "RGB565".into(),
        tex_format::RGB5A3 => "RGB5A3".into(),
        tex_format::RGBA8 => "RGBA8".into(),
        tex_format::CMPR => "CMPR".into(),
        tex_format::C4 => format!("C4 ({p})"),
        tex_format::C8 => format!("C8 ({p})"),
        tex_format::C14X2 => format!("C14X2 ({p})"),
        _ => "invalid".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(calc_texture_size(tex_format::CMPR, 64, 64), 2048);
        assert_eq!(calc_texture_size(tex_format::RGBA8, 4, 4), 64);
        assert_eq!(calc_texture_size(tex_format::I4, 1, 1), 32);
        let chain = calc_mip_chain(tex_format::RGB565, 8, 8, 4, 1 << 20);
        assert_eq!(chain.len(), 4);
        assert_eq!(chain[1].offset, 128);
        assert_eq!(chain[3].width, 1);
    }

    #[test]
    fn rgb5a3_and_rgba8() {
        // One 4x4 RGB5A3 block, all opaque pure red (0xFC00).
        let src = [0xFCu8, 0x00].repeat(16);
        let t = TextureInput {
            format: tex_format::RGB5A3,
            width: 4,
            height: 4,
            data: &src,
            mip_count: 1,
            palette_format: 0,
            palette: &[],
        };
        assert_eq!(&decode_texture(&t)[0..4], &[255, 0, 0, 255]);
        // RGBA8: AR plane then GB plane.
        let mut src = vec![0u8; 64];
        src[0] = 0x80; // A
        src[1] = 0x11; // R
        src[32] = 0x22; // G
        src[33] = 0x33; // B
        let t = TextureInput {
            format: tex_format::RGBA8,
            data: &src,
            ..t
        };
        assert_eq!(&decode_texture(&t)[0..4], &[0x11, 0x22, 0x33, 0x80]);
    }

    #[test]
    fn c8_palette() {
        let src = vec![1u8; 32];
        let palette = [0x00u8, 0x00, 0xF8, 0x00]; // RGB565: black, red
        let t = TextureInput {
            format: tex_format::C8,
            width: 8,
            height: 4,
            data: &src,
            mip_count: 1,
            palette_format: tex_palette::RGB565,
            palette: &palette,
        };
        assert_eq!(&decode_texture(&t)[0..4], &[255, 0, 0, 255]);
    }
}
