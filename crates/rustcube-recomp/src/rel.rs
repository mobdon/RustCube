//! Dolphin REL module reader and SEGA PRS decompression.

#[derive(Debug, Clone)]
pub struct RelSection {
    pub offset: u32,
    pub size: u32,
    pub exec: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reloc {
    /// Section and offset being patched.
    pub section: u8,
    pub offset: u32,
    pub kind: u8,
    /// Module the target lives in (0 = main.dol, absolute address in `addend`).
    pub module: u32,
    pub target_section: u8,
    pub addend: u32,
}

#[derive(Debug, Clone)]
pub struct Rel {
    pub id: u32,
    pub sections: Vec<RelSection>,
    pub relocs: Vec<Reloc>,
    pub bytes: Vec<u8>,
}

fn be(b: &[u8], o: usize) -> Result<u32, String> {
    b.get(o..o + 4).map(|s| u32::from_be_bytes(s.try_into().unwrap())).ok_or_else(|| "REL truncated".to_string())
}

impl Rel {
    pub fn parse(bytes: Vec<u8>) -> Result<Rel, String> {
        let b = &bytes;
        let id = be(b, 0)?;
        let n = be(b, 0xC)? as usize;
        let tab = be(b, 0x10)? as usize;
        let imp = be(b, 0x28)? as usize;
        let imp_size = be(b, 0x2C)? as usize;
        let mut sections = Vec::new();
        for i in 0..n {
            let off = be(b, tab + i * 8)?;
            let size = be(b, tab + i * 8 + 4)?;
            sections.push(RelSection { offset: off & !1, size, exec: off & 1 != 0 });
        }
        let mut relocs = Vec::new();
        for e in 0..imp_size / 8 {
            let module = be(b, imp + e * 8)?;
            let mut p = be(b, imp + e * 8 + 4)? as usize;
            let (mut sec, mut off) = (0u8, 0u32);
            loop {
                let delta = u16::from_be_bytes(b.get(p..p + 2).ok_or("REL truncated")?.try_into().unwrap());
                let kind = b[p + 2];
                let ts = b[p + 3];
                let addend = be(b, p + 4)?;
                p += 8;
                off += delta as u32;
                match kind {
                    203 => break,
                    202 => {
                        sec = ts;
                        off = 0;
                    }
                    201 | 0 => {}
                    _ => relocs.push(Reloc { section: sec, offset: off, kind, module, target_section: ts, addend }),
                }
            }
        }
        Ok(Rel { id, sections, relocs, bytes })
    }

    pub fn text(&self) -> Option<(u32, &RelSection)> {
        self.sections.iter().enumerate().find(|(_, s)| s.exec && s.size > 0).map(|(i, s)| (i as u32, s))
    }

    pub fn word(&self, sec: &RelSection, off: u32) -> u32 {
        let o = (sec.offset + off) as usize;
        self.bytes.get(o..o + 4).map(|s| u32::from_be_bytes(s.try_into().unwrap())).unwrap_or(0)
    }
}

/// SEGA PRS decompression. Returns the input unchanged if it is a REL already.
pub fn prs(src: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0usize;
    let (mut bits, mut nbits) = (0u8, 0u32);
    let mut bit = |i: &mut usize| -> Option<bool> {
        if nbits == 0 {
            bits = *src.get(*i)?;
            *i += 1;
            nbits = 8;
        }
        let v = bits & 1 != 0;
        bits >>= 1;
        nbits -= 1;
        Some(v)
    };
    loop {
        let Some(lit) = bit(&mut i) else { break };
        if lit {
            let Some(&v) = src.get(i) else { break };
            out.push(v);
            i += 1;
            continue;
        }
        let (off, size): (isize, usize);
        if bit(&mut i) == Some(true) {
            let Some(s) = src.get(i..i + 2) else { break };
            let v = u16::from_le_bytes([s[0], s[1]]) as usize;
            i += 2;
            if v == 0 {
                break;
            }
            off = ((v >> 3) as isize) - 0x2000;
            size = if v & 7 == 0 {
                let n = src.get(i).copied().unwrap_or(0) as usize + 1;
                i += 1;
                n
            } else {
                (v & 7) + 2
            };
        } else {
            let hi = bit(&mut i).unwrap_or(false) as usize;
            let lo = bit(&mut i).unwrap_or(false) as usize;
            size = (hi << 1 | lo) + 2;
            let Some(&v) = src.get(i) else { break };
            i += 1;
            off = v as isize - 0x100;
        }
        let start = out.len() as isize + off;
        if start < 0 {
            break;
        }
        for k in 0..size {
            let v = out[start as usize + k];
            out.push(v);
        }
    }
    out
}
