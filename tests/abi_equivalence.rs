//! The Rust side calls these Swift-CC runtime functions as `extern "C"` (src/swift/rt.rs). That is only sound
//! while clang lowers a `swiftcall` call and a C call to the same machine code. Check it for every signature,
//! on arm64 and x86_64. Swift-CC `bool` returns differ (only bit 0 is defined); rt.rs reads them as `u8 & 1`.
use std::process::Command;

const SIGS: &str = include_str!("../src/swift/rt_sigs.txt");
const PRELUDE: &str = "#include <stddef.h>\n#include <stdint.h>\n\
typedef struct { void *task; void *initial_context; } AsyncTaskAndContext;\n\
typedef struct { const void *identity; uintptr_t implementation; } SerialExecutorRef;\n\
typedef struct { const void *metadata; size_t state; } MetadataResponse;\n\
typedef struct { const char *data; size_t length; } TypeNamePair;\n\
typedef struct { uint64_t a, b; } SwiftString;\n";

struct Sig { name: String, ret: String, params: String, args: String }

impl Sig {
    /// A C identifier for the (possibly mangled, `$`-containing) symbol.
    fn ident(&self) -> String {
        self.name.replace('$', "_S_")
    }
    /// Function-pointer shapes (value witnesses) have no symbol to bind in rt.rs.
    fn is_symbol(&self) -> bool {
        !self.name.starts_with("vwt_") && !self.name.starts_with("shape_")
    }
}

fn sigs() -> Vec<Sig> {
    SIGS.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|l| {
            let f: Vec<_> = l.split('|').map(str::trim).collect();
            assert_eq!(f.len(), 4, "bad line in rt_sigs.txt: {l}");
            Sig { name: f[0].into(), ret: f[1].into(), params: f[2].into(), args: f[3].into() }
        })
        .collect()
}

fn source(sigs: &[Sig], swiftcall: bool) -> String {
    let attr = if swiftcall { "__attribute__((swiftcall))" } else { "" };
    let mut s = String::from(PRELUDE);
    s.push_str("void zz_first(void) {}\n");
    for g in sigs {
        let params = if g.params.is_empty() { "void" } else { &g.params };
        s.push_str(&format!("extern {attr} {} ext_{}({params}) __asm__(\"_{}\");\n", g.ret, g.ident(), g.name));
        let call = format!("ext_{}({})", g.ident(), g.args);
        let body = if g.ret == "void" { format!("{call};") } else { format!("return {call};") };
        s.push_str(&format!("{} call_{}({params}) {{ {body} }}\n", g.ret, g.ident()));
    }
    s
}

fn disassemble(src: &str, arch: &str, tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!("async-swift-rs-abi-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (c, o) = (dir.join(format!("{tag}-{arch}.c")), dir.join(format!("{tag}-{arch}.o")));
    std::fs::write(&c, src).unwrap();
    let st = Command::new("clang").args(["-O2", "-arch", arch, "-c"]).arg(&c).arg("-o").arg(&o).status().unwrap();
    assert!(st.success(), "clang failed for {tag} {arch}");
    let out = Command::new("objdump").args(["-d", "--no-show-raw-insn", "--no-leading-addr"]).arg(&o).output().unwrap();
    String::from_utf8(out.stdout).unwrap()
}

/// The body of `call_<name>`, with branch targets and symbol annotations normalized away.
fn body(dis: &str, name: &str) -> String {
    let header = format!("<_call_{name}>:");
    let mut lines = dis.lines().skip_while(|l| l.trim() != header).skip(1);
    let mut out = Vec::new();
    for l in lines.by_ref() {
        if l.starts_with('<') || l.trim().is_empty() { break }
        let l = l.split('<').next().unwrap();
        let l: String = l.split_whitespace().map(|t| if t.starts_with("0x") { "ADDR" } else { t }).collect::<Vec<_>>().join(" ");
        out.push(l);
    }
    assert!(!out.is_empty(), "no code found for call_{name}");
    out.join("\n")
}

fn main() {
    let sigs = sigs();
    let rt = include_str!("../src/swift/rt.rs");
    let mut failures = Vec::new();
    for g in sigs.iter().filter(|g| g.is_symbol()) {
        let declared = rt.contains(&format!("fn {}(", g.name)) || rt.contains(&format!("link_name = \"{}\"", g.name));
        if !declared {
            failures.push(format!("{} is in rt_sigs.txt but not declared in rt.rs", g.name));
        }
    }
    // Every Swift-CC function rt.rs declares must be checked; plain-C runtime functions carry a `C-CC` marker.
    for line in rt.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("pub fn swift_") {
            let name = format!("swift_{}", rest.split('(').next().unwrap());
            if !line.contains("C-CC") && !sigs.iter().any(|g| g.name == name) {
                failures.push(format!("{name} is declared in rt.rs but has no rt_sigs.txt entry (or C-CC marker)"));
            }
        }
    }
    for arch in ["arm64", "x86_64"] {
        let (swift, c) = (disassemble(&source(&sigs, true), arch, "swift"), disassemble(&source(&sigs, false), arch, "c"));
        for g in &sigs {
            let (a, b) = (body(&swift, &g.ident()), body(&c, &g.ident()));
            let expected_bool_mask = g.ret == "_Bool"; // bit 0 only: rt.rs reads u8 & 1
            if a != b && !expected_bool_mask {
                failures.push(format!("{arch} {}: swiftcall and C lower differently\n{a}\n---\n{b}", g.name));
            }
        }
    }
    if failures.is_empty() {
        println!("runtime_calls_lower_identically ... ok ({} functions × 2 archs)", sigs.len());
    } else {
        eprintln!("{}", failures.join("\n\n"));
        std::process::exit(1);
    }
}
