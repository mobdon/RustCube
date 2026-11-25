//! Gekko instruction -> Rust translation.
//!
//! Each game function becomes one Rust function: a `loop { match pc { .. } }`
//! over its basic blocks. Branches inside the function set `pc`; calls and
//! branches out of it go through `rt::call`, so overrides apply everywhere.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

/// What the translator needs to know about the function being emitted.
#[derive(Default)]
pub struct Ctx {
    pub start: u32,
    pub end: u32,
    /// REL code: (module id, section). Addresses are then section offsets and
    /// the section's load address is looked up at run time.
    pub rel: Option<(u32, u32)>,
    /// Relocations by instruction address: (type, Rust expression for the target).
    pub relocs: std::collections::HashMap<u32, (u8, String)>,
}

pub const R_ADDR16_LO: u8 = 4;
pub const R_ADDR16_HI: u8 = 5;
pub const R_ADDR16_HA: u8 = 6;
pub const R_REL24: u8 = 10;

impl Ctx {
    fn local(&self, t: u32) -> bool {
        (self.start..self.end).contains(&t)
    }
    /// Absolute address expression for an address in this code's space.
    pub fn abs(&self, t: &str) -> String {
        match self.rel {
            Some((id, sec)) => format!("rt::sec(m, {id}, {sec}).wrapping_add({t})"),
            None => t.to_string(),
        }
    }
    fn base(&self) -> String {
        self.abs("0")
    }
}

pub struct Ins {
    pub code: String,
    /// Local branch targets (block starts).
    pub targets: Vec<u32>,
    /// Control never falls through to the next instruction.
    pub ends: bool,
    /// Instruction the translator does not know.
    pub unknown: bool,
}

fn simm(w: u32) -> i32 {
    w as u16 as i16 as i32
}

fn ea_d(ra: u32, d: i32) -> String {
    if ra == 0 {
        format!("{:#x}u32", d as u32)
    } else {
        format!("c.r[{ra}].wrapping_add({:#x})", d as u32)
    }
}

fn ea_x(ra: u32, rb: u32) -> String {
    if ra == 0 {
        format!("c.r[{rb}]")
    } else {
        format!("c.r[{ra}].wrapping_add(c.r[{rb}])")
    }
}

fn mask(mb: u32, me: u32) -> u32 {
    let a = u32::MAX >> mb;
    let b = u32::MAX << (31 - me);
    if mb <= me { a & b } else { a | b }
}

fn bc_cond(bo: u32, bi: u32) -> (String, String) {
    let pre = if bo & 4 == 0 { "c.ctr = c.ctr.wrapping_sub(1); ".to_string() } else { String::new() };
    let mut parts = Vec::new();
    if bo & 4 == 0 {
        parts.push(if bo & 2 != 0 { "c.ctr == 0".to_string() } else { "c.ctr != 0".to_string() });
    }
    if bo & 0x10 == 0 {
        parts.push(if bo & 8 != 0 { format!("c.bit({bi})") } else { format!("!c.bit({bi})") });
    }
    let cond = if parts.is_empty() { "true".to_string() } else { parts.join(" && ") };
    (pre, cond)
}

/// Branch to `t` without link: local jump, or tail call out of the function.
fn goto(ctx: &Ctx, pc: u32, t: u32, targets: &mut Vec<u32>) -> String {
    if ctx.local(t) {
        targets.push(t);
        // Loops tick the runtime, which can interrupt a spin-wait.
        let tick = if t <= pc { "rt::backedge(c, m); " } else { "" };
        format!("{tick}pc = {t:#010x}; continue;")
    } else {
        format!("{} return;", call(&ctx.abs(&format!("{t:#010x}"))))
    }
}

pub fn translate(w: u32, pc: u32, ctx: &Ctx) -> Ins {
    let op = w >> 26;
    let d = w >> 21 & 31;
    let a = w >> 16 & 31;
    let b = w >> 11 & 31;
    let cr = w >> 6 & 31;
    let rc = w & 1 != 0;
    let xo10 = w >> 1 & 0x3FF;
    let xo5 = w >> 1 & 31;
    let next = pc.wrapping_add(4);
    let mut targets = Vec::new();
    let mut ends = false;
    let mut unknown = false;
    let si = simm(w);
    let reloc = ctx.relocs.get(&pc).cloned();
    let ui = w & 0xFFFF;
    let rd = |v: &str| format!("c.r[{d}] = {v};");
    let ra_set = |v: &str| format!("c.r[{a}] = {v};");
    let recd = if rc { format!(" c.rec(c.r[{d}]);") } else { String::new() };
    let reca = if rc { format!(" c.rec(c.r[{a}]);") } else { String::new() };
    let (fa, fb, fc) = (format!("c.f[{a}].ps0"), format!("c.f[{b}].ps0"), format!("c.f[{cr}].ps0"));
    let fill = |v: &str| format!("{{ let v = rs({v}); c.f[{d}] = Fpr {{ ps0: v, ps1: v }}; }}");
    let set0 = |v: &str| format!("c.f[{d}].ps0 = {v};");

    let code: String = match op {
        3 => String::new(), // twi: conditional traps are debug checks
        4 => {
            let xo6 = w >> 1 & 0x3F;
            match xo6 {
                6 | 38 => {
                    let (wb, i) = (w >> 10 & 1 != 0, w >> 7 & 7);
                    let up = if xo6 == 38 { format!(" c.r[{a}] = ea;") } else { String::new() };
                    format!("{{ let ea = {}; c.f[{d}] = psq_load(c, m, ea, {wb}, {i});{up} }}", ea_x(a, b))
                }
                7 | 39 => {
                    let (wb, i) = (w >> 10 & 1 != 0, w >> 7 & 7);
                    let up = if xo6 == 39 { format!(" c.r[{a}] = ea;") } else { String::new() };
                    format!("{{ let ea = {}; let f = c.f[{d}]; psq_store(c, m, ea, {wb}, {i}, f);{up} }}", ea_x(a, b))
                }
                _ => {
                    let pair = |p0: &str, p1: &str| {
                        format!(
                            "{{ let a = c.f[{a}]; let b = c.f[{b}]; let cc = c.f[{cr}]; let _ = (a, b, cc); c.f[{d}] = Fpr {{ ps0: rs({p0}), ps1: rs({p1}) }}; }}"
                        )
                    };
                    let raw = |p0: &str, p1: &str| {
                        format!("{{ let a = c.f[{a}]; let b = c.f[{b}]; let _ = (a, b); c.f[{d}] = Fpr {{ ps0: {p0}, ps1: {p1} }}; }}")
                    };
                    match xo5 {
                        10 => pair("a.ps0 + b.ps1", "cc.ps1"),
                        11 => pair("cc.ps0", "a.ps0 + b.ps1"),
                        12 => pair("a.ps0 * cc.ps0", "a.ps1 * cc.ps0"),
                        13 => pair("a.ps0 * cc.ps1", "a.ps1 * cc.ps1"),
                        14 => pair("madd(a.ps0, cc.ps0, b.ps0)", "madd(a.ps1, cc.ps0, b.ps1)"),
                        15 => pair("madd(a.ps0, cc.ps1, b.ps0)", "madd(a.ps1, cc.ps1, b.ps1)"),
                        18 => pair("a.ps0 / b.ps0", "a.ps1 / b.ps1"),
                        20 => pair("a.ps0 - b.ps0", "a.ps1 - b.ps1"),
                        21 => pair("a.ps0 + b.ps0", "a.ps1 + b.ps1"),
                        23 => pair("fsel(a.ps0, cc.ps0, b.ps0)", "fsel(a.ps1, cc.ps1, b.ps1)"),
                        24 => pair("1.0 / b.ps0", "1.0 / b.ps1"),
                        25 => pair("a.ps0 * cc.ps0", "a.ps1 * cc.ps1"),
                        26 => pair("frsqrte(b.ps0)", "frsqrte(b.ps1)"),
                        28 => pair("madd(a.ps0, cc.ps0, -b.ps0)", "madd(a.ps1, cc.ps1, -b.ps1)"),
                        29 => pair("madd(a.ps0, cc.ps0, b.ps0)", "madd(a.ps1, cc.ps1, b.ps1)"),
                        30 => pair("-madd(a.ps0, cc.ps0, -b.ps0)", "-madd(a.ps1, cc.ps1, -b.ps1)"),
                        31 => pair("-madd(a.ps0, cc.ps0, b.ps0)", "-madd(a.ps1, cc.ps1, b.ps1)"),
                        _ => match xo10 {
                            0 | 32 => format!("c.cmp_f({}, c.f[{a}].ps0, c.f[{b}].ps0);", d >> 2),
                            64 | 96 => format!("c.cmp_f({}, c.f[{a}].ps1, c.f[{b}].ps1);", d >> 2),
                            40 => raw("-b.ps0", "-b.ps1"),
                            72 => raw("b.ps0", "b.ps1"),
                            136 => raw("-b.ps0.abs()", "-b.ps1.abs()"),
                            264 => raw("b.ps0.abs()", "b.ps1.abs()"),
                            528 => raw("a.ps0", "b.ps0"),
                            560 => raw("a.ps0", "b.ps1"),
                            592 => raw("a.ps1", "b.ps0"),
                            624 => raw("a.ps1", "b.ps1"),
                            1014 => format!("m.dcbz({});", ea_x(a, b)),
                            _ => {
                                unknown = true;
                                String::new()
                            }
                        },
                    }
                }
            }
        }
        7 => rd(&format!("c.r[{a}].wrapping_mul({:#x})", si as u32)),
        8 => format!("c.r[{d}] = {{ let (v, k) = adde(!c.r[{a}], {:#x}, true); c.ca = k; v }};", si as u32),
        10 => format!("c.cmp_u({}, c.r[{a}], {ui:#x});", d >> 2),
        11 => format!("c.cmp_s({}, c.r[{a}] as i32, {si});", d >> 2),
        12 | 13 => {
            let r = if op == 13 { format!(" c.rec(c.r[{d}]);") } else { String::new() };
            format!("c.r[{d}] = {{ let (v, k) = adde(c.r[{a}], {:#x}, false); c.ca = k; v }};{r}", si as u32)
        }
        14 | 15 | 24 | 25 | 32..=55 if reloc.is_some() => {
            let (ty, r) = reloc.unwrap();
            let v = match ty {
                R_ADDR16_HA => format!("({r}).wrapping_add(0x8000) & 0xFFFF0000"),
                R_ADDR16_HI => format!("({r}) & 0xFFFF0000"),
                _ if op == 24 => format!("({r}) & 0xFFFF"),
                _ => format!("({r}) as u16 as i16 as u32"),
            };
            let base = if a == 0 || op == 24 || op == 25 { String::new() } else { format!("c.r[{a}].wrapping_add") };
            match op {
                14 | 15 => rd(&if a == 0 { v } else { format!("{base}({v})") }),
                24 | 25 => ra_set(&format!("c.r[{d}] | ({v})")),
                _ => {
                    let ea = if a == 0 { v } else { format!("{base}({v})") };
                    let x = translate(w & 0xFFFF_0000, pc, &Ctx { start: ctx.start, end: ctx.end, ..Default::default() }).code;
                    let old = ea_d(a, 0);
                    x.replacen(&format!("let ea = {old};"), &format!("let ea = {ea};"), 1)
                }
            }
        }
        14 => rd(&ea_d(a, si)),
        15 => rd(&ea_d(a, si << 16)),
        16 => {
            let bd = (w & 0xFFFC) as u16 as i16 as i32;
            let t = if w & 2 != 0 { bd as u32 } else { pc.wrapping_add(bd as u32) };
            let (pre, cond) = bc_cond(d, a);
            if w & 1 != 0 {
                format!("{pre}if {cond} {{ c.lr = {}; {} }}", ctx.abs(&format!("{next:#010x}")), call(&ctx.abs(&format!("{t:#010x}"))))
            } else {
                if cond == "true" {
                    ends = true;
                }
                format!("{pre}if {cond} {{ {} }}", goto(ctx, pc, t, &mut targets))
            }
        }
        // sc: the OS system-call handler only syncs (DCFlushRange uses it as a
        // barrier); memory here is always coherent.
        17 => String::new(),
        18 => {
            let li = ((w & 0x03FF_FFFC) << 6) as i32 >> 6;
            let t = if w & 2 != 0 { li as u32 } else { pc.wrapping_add(li as u32) };
            if let Some((R_REL24, r)) = &reloc {
                if w & 1 == 0 {
                    ends = true;
                }
                let tail = if w & 1 == 0 { " return;" } else { "" };
                format!("c.lr = {}; {}{tail}", ctx.abs(&format!("{next:#010x}")), call(r))
            } else if w & 1 != 0 {
                format!("c.lr = {}; {}", ctx.abs(&format!("{next:#010x}")), call(&ctx.abs(&format!("{t:#010x}"))))
            } else {
                ends = true;
                goto(ctx, pc, t, &mut targets)
            }
        }
        19 => {
            let crop = |e: &str| format!("{{ let a = c.bit({a}); let b = c.bit({b}); c.set_bit({d}, {e}); }}");
            match xo10 {
                0 => format!("{{ let v = c.field({}); c.set_field({}, v); }}", a >> 2, d >> 2),
                16 => {
                    let (pre, cond) = bc_cond(d, a);
                    if w & 1 != 0 {
                        format!("{pre}if {cond} {{ let t = c.lr; c.lr = {}; rt::call(c, m, t); }}", ctx.abs(&format!("{next:#010x}")))
                    } else {
                        if cond == "true" {
                            ends = true;
                        }
                        format!("{pre}if {cond} {{ return; }}")
                    }
                }
                33 => crop("!(a | b)"),
                50 => {
                    ends = true;
                    "return;".into()
                }
                129 => crop("a & !b"),
                150 => String::new(),
                193 => crop("a ^ b"),
                225 => crop("!(a & b)"),
                257 => crop("a & b"),
                289 => crop("a == b"),
                417 => crop("a | !b"),
                449 => crop("a | b"),
                528 => {
                    let (pre, cond) = bc_cond(d | 4, a);
                    let _ = pre;
                    if w & 1 != 0 {
                        format!("if {cond} {{ let t = c.ctr; c.lr = {}; rt::call(c, m, t); }}", ctx.abs(&format!("{next:#010x}")))
                    } else {
                        if cond == "true" {
                            ends = true;
                        }
                        // Jump tables land on local blocks; anything else is a tail call.
                        format!("if {cond} {{ pc = c.ctr.wrapping_sub({}); continue; }}", ctx.base())
                    }
                }
                _ => {
                    unknown = true;
                    String::new()
                }
            }
        }
        20 => {
            let (sh, mb, me) = (b, w >> 6 & 31, w >> 1 & 31);
            let k = mask(mb, me);
            format!("c.r[{a}] = (c.r[{d}].rotate_left({sh}) & {k:#x}) | (c.r[{a}] & {:#x});{reca}", !k)
        }
        21 => {
            let (sh, mb, me) = (b, w >> 6 & 31, w >> 1 & 31);
            format!("c.r[{a}] = c.r[{d}].rotate_left({sh}) & {:#x};{reca}", mask(mb, me))
        }
        23 => {
            let (mb, me) = (w >> 6 & 31, w >> 1 & 31);
            format!("c.r[{a}] = c.r[{d}].rotate_left(c.r[{b}] & 31) & {:#x};{reca}", mask(mb, me))
        }
        24 => ra_set(&format!("c.r[{d}] | {ui:#x}")),
        25 => ra_set(&format!("c.r[{d}] | {:#x}", ui << 16)),
        26 => ra_set(&format!("c.r[{d}] ^ {ui:#x}")),
        27 => ra_set(&format!("c.r[{d}] ^ {:#x}", ui << 16)),
        28 => format!("c.r[{a}] = c.r[{d}] & {ui:#x}; c.rec(c.r[{a}]);"),
        29 => format!("c.r[{a}] = c.r[{d}] & {:#x}; c.rec(c.r[{a}]);", ui << 16),
        31 => {
            let xo9 = xo10 & 0x1FF;
            let arith = |e: &str| format!("c.r[{d}] = {e};{recd}");
            let logic = |e: &str| format!("c.r[{a}] = {e};{reca}");
            let (rs_, ra_, rb_) = (format!("c.r[{d}]"), format!("c.r[{a}]"), format!("c.r[{b}]"));
            let ld = |f: &str, upd: bool| {
                let up = if upd { format!(" c.r[{a}] = ea;") } else { String::new() };
                format!("{{ let ea = {}; c.r[{d}] = {f};{up} }}", ea_x(a, b))
            };
            let st = |f: &str, upd: bool| {
                let up = if upd { format!(" c.r[{a}] = ea;") } else { String::new() };
                format!("{{ let ea = {}; {f};{up} }}", ea_x(a, b))
            };
            match xo9 {
                8 => arith(&format!("{{ let (v, k) = adde(!{ra_}, {rb_}, true); c.ca = k; v }}")),
                10 => arith(&format!("{{ let (v, k) = adde({ra_}, {rb_}, false); c.ca = k; v }}")),
                11 => arith(&format!("(({ra_} as u64 * {rb_} as u64) >> 32) as u32")),
                40 => arith(&format!("{rb_}.wrapping_sub({ra_})")),
                75 => arith(&format!("(({ra_} as i32 as i64 * {rb_} as i32 as i64) >> 32) as u32")),
                104 => arith(&format!("{ra_}.wrapping_neg()")),
                136 => arith(&format!("{{ let k = c.ca; {{ let (v, k) = adde(!{ra_}, {rb_}, k); c.ca = k; v }} }}")),
                138 => arith(&format!("{{ let k = c.ca; {{ let (v, k) = adde({ra_}, {rb_}, k); c.ca = k; v }} }}")),
                200 => arith(&format!("{{ let k = c.ca; {{ let (v, k) = adde(!{ra_}, 0, k); c.ca = k; v }} }}")),
                202 => arith(&format!("{{ let k = c.ca; {{ let (v, k) = adde({ra_}, 0, k); c.ca = k; v }} }}")),
                232 => arith(&format!("{{ let k = c.ca; {{ let (v, k) = adde(!{ra_}, u32::MAX, k); c.ca = k; v }} }}")),
                234 => arith(&format!("{{ let k = c.ca; {{ let (v, k) = adde({ra_}, u32::MAX, k); c.ca = k; v }} }}")),
                235 => arith(&format!("{ra_}.wrapping_mul({rb_})")),
                266 => arith(&format!("{ra_}.wrapping_add({rb_})")),
                459 => arith(&format!("divwu({ra_}, {rb_})")),
                491 => arith(&format!("divw({ra_}, {rb_})")),
                _ => match xo10 {
                    0 => format!("c.cmp_s({}, {ra_} as i32, {rb_} as i32);", d >> 2),
                    32 => format!("c.cmp_u({}, {ra_}, {rb_});", d >> 2),
                    4 => {
                        if d == 31 {
                            format!("panic!(\"trap at {pc:#010x}\");")
                        } else {
                            String::new()
                        }
                    }
                    19 => rd("c.cr"),
                    20 | 23 => ld("m.r32(ea)", false),
                    55 => ld("m.r32(ea)", true),
                    87 => ld("m.r8(ea) as u32", false),
                    119 => ld("m.r8(ea) as u32", true),
                    279 => ld("m.r16(ea) as u32", false),
                    311 => ld("m.r16(ea) as u32", true),
                    343 => ld("m.r16(ea) as i16 as u32", false),
                    375 => ld("m.r16(ea) as i16 as u32", true),
                    534 => ld("m.r32(ea).swap_bytes()", false),
                    790 => ld("m.r16(ea).swap_bytes() as u32", false),
                    150 => format!("{{ let ea = {}; m.w32(ea, {rs_}); let so = c.so as u32; c.set_field(0, 2 | so); }}", ea_x(a, b)),
                    151 => st(&format!("m.w32(ea, {rs_})"), false),
                    183 => st(&format!("m.w32(ea, {rs_})"), true),
                    215 => st(&format!("m.w8(ea, {rs_} as u8)"), false),
                    247 => st(&format!("m.w8(ea, {rs_} as u8)"), true),
                    407 => st(&format!("m.w16(ea, {rs_} as u16)"), false),
                    439 => st(&format!("m.w16(ea, {rs_} as u16)"), true),
                    662 => st(&format!("m.w32(ea, {rs_}.swap_bytes())"), false),
                    918 => st(&format!("m.w16(ea, ({rs_} as u16).swap_bytes())"), false),
                    535 | 567 => {
                        let up = if xo10 == 567 { format!(" c.r[{a}] = ea;") } else { String::new() };
                        format!("{{ let ea = {}; let v = m.rf32(ea); c.f[{d}] = Fpr {{ ps0: v, ps1: v }};{up} }}", ea_x(a, b))
                    }
                    599 | 631 => {
                        let up = if xo10 == 631 { format!(" c.r[{a}] = ea;") } else { String::new() };
                        format!("{{ let ea = {}; c.f[{d}].ps0 = m.rf64(ea);{up} }}", ea_x(a, b))
                    }
                    663 => st(&format!("m.wf32(ea, c.f[{d}].ps0)"), false),
                    695 => st(&format!("m.wf32(ea, c.f[{d}].ps0)"), true),
                    727 => st(&format!("m.wf64(ea, c.f[{d}].ps0)"), false),
                    759 => st(&format!("m.wf64(ea, c.f[{d}].ps0)"), true),
                    983 => st(&format!("m.w32(ea, c.f[{d}].ps0.to_bits() as u32)"), false),
                    24 => logic(&format!("if {rb_} & 0x20 != 0 {{ 0 }} else {{ {rs_} << ({rb_} & 31) }}")),
                    536 => logic(&format!("if {rb_} & 0x20 != 0 {{ 0 }} else {{ {rs_} >> ({rb_} & 31) }}")),
                    26 => logic(&format!("{rs_}.leading_zeros()")),
                    28 => logic(&format!("{rs_} & {rb_}")),
                    60 => logic(&format!("{rs_} & !{rb_}")),
                    124 => logic(&format!("!({rs_} | {rb_})")),
                    284 => logic(&format!("!({rs_} ^ {rb_})")),
                    316 => logic(&format!("{rs_} ^ {rb_}")),
                    412 => logic(&format!("{rs_} | !{rb_}")),
                    444 => logic(&format!("{rs_} | {rb_}")),
                    476 => logic(&format!("!({rs_} & {rb_})")),
                    792 => logic(&format!("{{ let v = {rs_}; let s = {rb_}; let (r, k) = sraw(v, s); c.ca = k; r }}")),
                    824 => logic(&format!("{{ let v = {rs_}; let (r, k) = sraw(v, {b}); c.ca = k; r }}")),
                    922 => logic(&format!("{rs_} as u16 as i16 as u32")),
                    954 => logic(&format!("{rs_} as u8 as i8 as u32")),
                    83 => rd("c.msr"),
                    146 => format!("c.msr = {rs_};"),
                    144 => format!("{{ let v = {rs_}; c.mtcrf({:#x}, v); }}", w >> 12 & 0xFF),
                    339 | 371 => rd(&format!("c.mfspr({})", a | b << 5)),
                    467 => format!("{{ let v = {rs_}; c.mtspr({}, v); }}", a | b << 5),
                    512 => format!(
                        "{{ let v = (c.so as u32) << 3 | (c.ca as u32) << 1; c.set_field({}, v); c.so = false; c.ca = false; }}",
                        d >> 2
                    ),
                    210 | 242 | 595 | 659 => {
                        if xo10 >= 595 {
                            rd("0")
                        } else {
                            String::new()
                        }
                    }
                    54 | 86 | 246 | 278 | 470 | 982 | 598 | 854 | 306 | 566 | 370 => String::new(),
                    1014 => format!("m.dcbz({});", ea_x(a, b)),
                    597 => format!("{{ let ea = {}; lswi(c, m, {d}, ea, {b}); }}", ea_d(a, 0)),
                    725 => format!("{{ let ea = {}; stswi(c, m, {d}, ea, {b}); }}", ea_d(a, 0)),
                    _ => {
                        unknown = true;
                        String::new()
                    }
                },
            }
        }
        32..=47 => {
            let ea = ea_d(a, si);
            let upd = op & 1 == 1 && op != 47 && op != 46;
            let up = if upd { format!(" c.r[{a}] = ea;") } else { String::new() };
            let body = match op {
                32 | 33 => format!("c.r[{d}] = m.r32(ea);"),
                34 | 35 => format!("c.r[{d}] = m.r8(ea) as u32;"),
                36 | 37 => format!("m.w32(ea, c.r[{d}]);"),
                38 | 39 => format!("m.w8(ea, c.r[{d}] as u8);"),
                40 | 41 => format!("c.r[{d}] = m.r16(ea) as u32;"),
                42 | 43 => format!("c.r[{d}] = m.r16(ea) as i16 as u32;"),
                44 | 45 => format!("m.w16(ea, c.r[{d}] as u16);"),
                46 => format!("lmw(c, m, {d}, ea);"),
                _ => format!("stmw(c, m, {d}, ea);"),
            };
            format!("{{ let ea = {ea}; {body}{up} }}")
        }
        48..=55 => {
            let ea = ea_d(a, si);
            let up = if op & 1 == 1 { format!(" c.r[{a}] = ea;") } else { String::new() };
            let body = match op {
                48 | 49 => format!("let v = m.rf32(ea); c.f[{d}] = Fpr {{ ps0: v, ps1: v }};"),
                50 | 51 => format!("c.f[{d}].ps0 = m.rf64(ea);"),
                52 | 53 => format!("m.wf32(ea, c.f[{d}].ps0);"),
                _ => format!("m.wf64(ea, c.f[{d}].ps0);"),
            };
            format!("{{ let ea = {ea}; {body}{up} }}")
        }
        56 | 57 | 60 | 61 => {
            let off = ((w & 0xFFF) << 20) as i32 >> 20;
            let (wb, i) = (w >> 15 & 1 != 0, w >> 12 & 7);
            let up = if op & 1 == 1 { format!(" c.r[{a}] = ea;") } else { String::new() };
            let body = if op < 60 {
                format!("c.f[{d}] = psq_load(c, m, ea, {wb}, {i});")
            } else {
                format!("let f = c.f[{d}]; psq_store(c, m, ea, {wb}, {i}, f);")
            };
            format!("{{ let ea = {}; {body}{up} }}", ea_d(a, off))
        }
        59 => match xo5 {
            18 => fill(&format!("{fa} / {fb}")),
            20 => fill(&format!("{fa} - {fb}")),
            21 => fill(&format!("{fa} + {fb}")),
            24 => fill(&format!("fres({fb})")),
            25 => fill(&format!("{fa} * {fc}")),
            28 => fill(&format!("madd({fa}, {fc}, -{fb})")),
            29 => fill(&format!("madd({fa}, {fc}, {fb})")),
            30 => fill(&format!("-madd({fa}, {fc}, -{fb})")),
            31 => fill(&format!("-madd({fa}, {fc}, {fb})")),
            _ => {
                unknown = true;
                String::new()
            }
        },
        63 => match xo5 {
            18 => set0(&format!("{fa} / {fb}")),
            20 => set0(&format!("{fa} - {fb}")),
            21 => set0(&format!("{fa} + {fb}")),
            23 => set0(&format!("fsel({fa}, {fc}, {fb})")),
            25 => set0(&format!("{fa} * {fc}")),
            26 => set0(&format!("frsqrte({fb})")),
            28 => set0(&format!("madd({fa}, {fc}, -{fb})")),
            29 => set0(&format!("madd({fa}, {fc}, {fb})")),
            30 => set0(&format!("-madd({fa}, {fc}, -{fb})")),
            31 => set0(&format!("-madd({fa}, {fc}, {fb})")),
            _ => match xo10 {
                0 | 32 => format!("c.cmp_f({}, {fa}, {fb});", d >> 2),
                12 => fill(&fb),
                14 => set0(&format!("fctiw({fb}, false)")),
                15 => set0(&format!("fctiw({fb}, true)")),
                38 => format!("c.fpscr |= {:#x};", 1u32 << (31 - d)),
                70 => format!("c.fpscr &= !{:#x};", 1u32 << (31 - d)),
                40 => set0(&format!("-{fb}")),
                72 => set0(&fb),
                136 => set0(&format!("-{fb}.abs()")),
                264 => set0(&format!("{fb}.abs()")),
                64 => format!("{{ let v = c.fpscr >> (28 - 4 * {}) & 0xF; c.set_field({}, v); }}", a >> 2, d >> 2),
                134 => {
                    let f = d >> 2;
                    let sh = 28 - 4 * f;
                    format!("c.fpscr = (c.fpscr & !(0xF << {sh})) | ({} << {sh});", w >> 12 & 0xF)
                }
                583 => set0("f64::from_bits(c.fpscr as u64)"),
                711 => {
                    let fm = w >> 17 & 0xFF;
                    let mut k = 0u32;
                    for i in 0..8 {
                        if fm >> (7 - i) & 1 != 0 {
                            k |= 0xF << (28 - 4 * i);
                        }
                    }
                    format!("c.fpscr = (c.fpscr & !{k:#x}) | (c.f[{b}].ps0.to_bits() as u32 & {k:#x});")
                }
                _ => {
                    unknown = true;
                    String::new()
                }
            },
        },
        _ => {
            unknown = true;
            String::new()
        }
    };
    let code = if unknown { format!("panic!(\"rustcube-recomp: unknown instruction {w:#010x} at {pc:#010x}\");") } else { code };
    if unknown {
        ends = true;
    }
    Ins { code, targets, ends, unknown }
}

/// Emit one function. `words` are its instructions starting at `ctx.start`;
/// `extra` holds more block starts (jump table entries found in data).
pub fn function(name: &str, ctx: &Ctx, words: &[u32], extra: &BTreeSet<u32>) -> (String, usize) {
    let ins: Vec<Ins> = words
        .iter()
        .enumerate()
        .map(|(i, &w)| translate(w, ctx.start + 4 * i as u32, ctx))
        .collect();
    let mut starts: BTreeSet<u32> = extra.iter().copied().filter(|&a| ctx.local(a) && a % 4 == 0).collect();
    starts.insert(ctx.start);
    for (i, x) in ins.iter().enumerate() {
        starts.extend(x.targets.iter().copied());
        // Any branch splits the block after it, so both paths are blocks.
        if !x.targets.is_empty() || x.ends {
            starts.insert(ctx.start + 4 * (i as u32 + 1));
        }
    }
    let unknown = ins.iter().filter(|x| x.unknown).count();
    let mut s = String::new();
    let _ = writeln!(s, "/// {name}");
    let _ = writeln!(s, "pub fn {}(c: &mut Cpu, m: &mut Mem) {{", fn_name(ctx));
    let _ = writeln!(s, "    let mut pc: u32 = {:#010x};", ctx.start);
    let _ = writeln!(s, "    loop {{\n        match pc {{");
    let mut open = false;
    for (i, x) in ins.iter().enumerate() {
        let a = ctx.start + 4 * i as u32;
        if starts.contains(&a) {
            if open {
                let _ = writeln!(s, "                pc = {a:#010x};\n            }}");
            }
            let _ = writeln!(s, "            {a:#010x} => {{");
            open = true;
        }
        if !open {
            continue; // unreachable after an unconditional branch
        }
        let _ = writeln!(s, "                {}", x.code);
        if x.ends {
            let _ = writeln!(s, "            }}");
            open = false;
        }
    }
    if open {
        let _ = writeln!(s, "                pc = {:#010x};\n            }}", ctx.end);
    }
    let _ = writeln!(s, "            _ => {{ {} return; }}", call(&ctx.abs("pc")));
    let _ = writeln!(s, "        }}\n    }}\n}}");
    (s, unknown)
}

/// Function ranges from `starts` (address -> size, 0 = up to the next start),
/// clipped to the `text` ranges. Every direct branch target that is a call
/// (`bl`) or leaves its function becomes a start too, until nothing changes:
/// routines with several entry points (register save/restore ladders) have
/// a symbol only for the first, some RELs have no symbols at all, and every
/// cross-function branch must land on a function `rt::call` can find.
/// `skip` marks relocated instructions (their branch field is not the target).
pub fn layout(
    mut starts: BTreeMap<u32, u32>,
    text: &[(u32, u32)],
    word: impl Fn(u32) -> Option<u32>,
    skip: impl Fn(u32) -> bool,
) -> Vec<(u32, u32)> {
    let sec = |a: u32| text.iter().find(|(lo, hi)| (*lo..*hi).contains(&a)).copied();
    starts.retain(|&a, _| sec(a).is_some());
    loop {
        let addrs: Vec<u32> = starts.keys().copied().collect();
        let mut funcs = Vec::new();
        for (i, (&a, &size)) in starts.iter().enumerate() {
            let hi = sec(a).map_or(a, |s| s.1);
            let next = addrs.get(i + 1).copied().unwrap_or(hi).min(hi);
            funcs.push((a, if size > 0 { a.saturating_add(size).min(hi) } else { next }));
        }
        let mut new = BTreeSet::new();
        for &(a, end) in &funcs {
            for pc in (a..end).step_by(4) {
                let Some(w) = word(pc) else { continue };
                let disp = match w >> 26 {
                    18 => ((w & 0x03FF_FFFC) << 6) as i32 >> 6,
                    16 => (w & 0xFFFC) as u16 as i16 as i32,
                    _ => continue,
                };
                if w & 2 != 0 || skip(pc) {
                    continue;
                }
                let t = pc.wrapping_add(disp as u32);
                if (w & 1 == 1 || !(a..end).contains(&t)) && sec(t).is_some() && !starts.contains_key(&t) {
                    new.insert(t);
                }
            }
        }
        if new.is_empty() {
            return funcs;
        }
        starts.extend(new.into_iter().map(|t| (t, 0)));
    }
}

/// Whether a code pointer found in data looks like a function entry rather
/// than a jump-table case: a prologue at the target, or a `blr` right before.
pub fn looks_like_entry(word: impl Fn(u32) -> Option<u32>, t: u32) -> bool {
    let w = word(t).unwrap_or(0);
    w >> 16 == 0x9421 || w == 0x7C08_02A6 || (t >= 4 && word(t - 4) == Some(0x4E80_0020))
}

pub fn fn_name(ctx: &Ctx) -> String {
    match ctx.rel {
        Some((id, sec)) => format!("r{id}_{sec}_{:X}", ctx.start),
        None => format!("f_{:08X}", ctx.start),
    }
}

/// A call statement. The target is bound first: REL targets read `m`
/// (`rt::sec`), which `rt::call` borrows mutably.
fn call(target: &str) -> String {
    format!("{{ let t = {target}; rt::call(c, m, t); }}")
}
