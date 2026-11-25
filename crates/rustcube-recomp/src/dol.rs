//! DOL executable and decomp-toolkit `symbols.txt` readers.

#[derive(Debug, Clone)]
pub struct Section {
    pub addr: u32,
    pub data: Vec<u8>,
    pub text: bool,
}

#[derive(Debug, Clone)]
pub struct Dol {
    pub sections: Vec<Section>,
    pub bss_addr: u32,
    pub bss_size: u32,
    pub entry: u32,
}

fn be(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes(b[o..o + 4].try_into().unwrap())
}

impl Dol {
    pub fn parse(b: &[u8]) -> Result<Dol, String> {
        if b.len() < 0x100 {
            return Err("DOL shorter than its header".into());
        }
        let mut sections = Vec::new();
        for i in 0..18 {
            let off = be(b, i * 4) as usize;
            let addr = be(b, 0x48 + i * 4);
            let size = be(b, 0x90 + i * 4) as usize;
            if size == 0 {
                continue;
            }
            let data = b
                .get(off..off + size)
                .ok_or_else(|| format!("section {i} runs past the file"))?
                .to_vec();
            sections.push(Section { addr, data, text: i < 7 });
        }
        Ok(Dol { sections, bss_addr: be(b, 0xD8), bss_size: be(b, 0xDC), entry: be(b, 0xE0) })
    }

    pub fn word(&self, a: u32) -> Option<u32> {
        self.sections.iter().find_map(|s| {
            let o = a.checked_sub(s.addr)? as usize;
            (o + 4 <= s.data.len()).then(|| be(&s.data, o))
        })
    }

    /// Copy the sections into memory and clear .bss, as the IPL does.
    pub fn load(&self, m: &mut crate::rt::Mem) {
        for i in 0..self.bss_size {
            m.w8(self.bss_addr + i, 0);
        }
        for s in &self.sections {
            m.load(s.addr, &s.data);
        }
    }
}

#[derive(Debug, Clone)]
pub struct Symbol {
    pub name: String,
    pub section: String,
    pub addr: u32,
    pub size: u32,
    /// A callable entry: a function, or a global label in .text.
    pub function: bool,
}

/// Parse decomp-toolkit symbols.txt lines:
/// `name = .text:0x80003100; // type:function size:0x10 scope:global`
pub fn parse_symbols(text: &str) -> Vec<Symbol> {
    text.lines()
        .filter_map(|l| {
            let (name, rest) = l.split_once(" = ")?;
            let (loc, attrs) = rest.split_once(';')?;
            let (section, addr) = loc.split_once(':')?;
            let addr = u32::from_str_radix(addr.trim().trim_start_matches("0x"), 16).ok()?;
            let mut size = 0;
            let (mut function, mut label, mut global) = (false, false, false);
            for a in attrs.trim_start_matches(|c| c == ' ' || c == '/').split_whitespace() {
                if let Some(v) = a.strip_prefix("size:0x") {
                    size = u32::from_str_radix(v, 16).unwrap_or(0);
                }
                function |= a == "type:function";
                label |= a == "type:label";
                global |= a == "scope:global";
            }
            // Global labels in code are extra entry points (_savegpr_14.., _restgpr_..).
            let function = function || (label && global && section.trim() == ".text");
            Some(Symbol { name: name.trim().into(), section: section.trim().into(), addr, size, function })
        })
        .collect()
}
