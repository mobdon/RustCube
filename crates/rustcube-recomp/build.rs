// Generates tests/synthetic code from hand-assembled instructions (no game
// bytes), using the same translator the tool uses.
#[path = "src/gen.rs"]
#[allow(dead_code)]
mod gen;

fn main() {
    println!("cargo:rerun-if-changed=src/gen.rs");
    let funcs: [(u32, &[u32]); 3] = [
        (
            0x8000_3000,
            &[
                0x7C0802A6, 0x90010004, 0x9421FFF0, 0x38600000, 0x3880000A, 0x7C8903A6, 0x7C632214,
                0x3884FFFF, 0x4200FFF8, 0x4800001D, 0x80010014, 0x38210010, 0x7C0803A6, 0x4E800020,
            ],
        ),
        (0x8000_3040, &[0x1C630002, 0x4E800020]),
        (0x8000_3048, &[0xC0230000, 0xEC21082A, 0xD0230004, 0x4E800020]),
    ];
    let mut out = String::from("use rustcube_recomp::rt::{self, *};\n");
    for (start, words) in funcs {
        let ctx = gen::Ctx { start, end: start + 4 * words.len() as u32, ..Default::default() };
        let (code, unknown) = gen::function("synthetic", &ctx, words, &Default::default());
        assert_eq!(unknown, 0);
        out.push_str(&code);
    }
    let dir = std::env::var("OUT_DIR").unwrap();
    std::fs::write(format!("{dir}/synthetic.rs"), out).unwrap();
}
