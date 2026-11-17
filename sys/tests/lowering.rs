//! Rust calls the header's `APUS_DIRECT` Swift-CC functions as `extern "C"` (the bindings are generated with the
//! Swift attributes defined away). That is only sound while clang lowers a `swiftcall` call and a C call to the same
//! machine code. For each such declaration, compile a caller twice (header as is, and with `-DAPUS_BINDGEN`) on arm64
//! and x86_64 and compare. Also check that every Swift-CC declaration carries a marker the rules allow.
#[path = "support/ast.rs"]
mod ast;

use ast::{Decl, Kind};
use std::process::Command;

fn caller(decl: &Decl) -> String {
    let params: Vec<String> = decl.params.iter().enumerate().map(|(i, t)| format!("{t} a{i}")).collect();
    let args: Vec<String> = (0..decl.params.len()).map(|i| format!("a{i}")).collect();
    let (target, params) = match decl.kind {
        Kind::Function => (decl.name.clone(), params),
        _ => ("fp".to_string(), std::iter::once(format!("{} fp", decl.name)).chain(params).collect()),
    };
    let params = if params.is_empty() { "void".to_string() } else { params.join(", ") };
    let call = format!("{target}({})", args.join(", "));
    let body = if decl.ret == "void" { format!("{call};") } else { format!("return {call};") };
    format!("{} call_{}({params}) {{ {body} }}\n", decl.ret, decl.name)
}

/// This run's scratch directory (removed when the test ends).
fn scratch() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("apus-lowering-{}", std::process::id()))
}

fn disassemble(source: &str, arch: &str, tag: &str, bindgen: bool) -> String {
    let dir = scratch();
    std::fs::create_dir_all(&dir).unwrap();
    let (c, o) = (dir.join(format!("{tag}-{arch}.c")), dir.join(format!("{tag}-{arch}.o")));
    std::fs::write(&c, source).unwrap();
    let mut clang = Command::new("xcrun");
    clang.args(["clang", "-O2", "-arch", arch, "-c", "-DAPUS_SYS_BUILD"]).arg(format!("-I{}", ast::include_dir().display()));
    if bindgen {
        clang.arg("-DAPUS_BINDGEN");
    }
    let status = clang.arg(&c).arg("-o").arg(&o).status().unwrap();
    assert!(status.success(), "clang failed for {tag} {arch}");
    let out = Command::new("xcrun").args(["objdump", "-d", "--no-show-raw-insn", "--no-leading-addr"]).arg(&o).output().unwrap();
    String::from_utf8(out.stdout).unwrap()
}

/// The body of `call_<name>`, with branch targets and symbol annotations normalized away.
fn body(dis: &str, name: &str) -> String {
    let header = format!("<_call_{name}>:");
    let mut out = Vec::new();
    for l in dis.lines().skip_while(|l| l.trim() != header).skip(1) {
        if l.starts_with('<') || l.trim().is_empty() {
            break;
        }
        let l = l.split('<').next().unwrap();
        out.push(l.split_whitespace().map(|t| if t.starts_with("0x") { "ADDR" } else { t }).collect::<Vec<_>>().join(" "));
    }
    assert!(!out.is_empty(), "no code found for call_{name}");
    out.join("\n")
}

fn rule_failures(decls: &[Decl]) -> Vec<String> {
    let mut failures = Vec::new();
    for d in decls.iter().filter(|d| d.kind != Kind::Data) {
        match d.marker.as_str() {
            "" if d.swift_cc => failures.push(format!("{}: Swift-CC declaration without a marker", d.name)),
            "c" if d.swift_cc => failures.push(format!("{}: APUS_C but Swift-CC", d.name)),
            "direct" | "thunk" if !d.swift_cc => failures.push(format!("{}: marked {} but not Swift-CC", d.name, d.marker)),
            _ => {}
        }
        if (d.marker == "direct" || d.marker == "thunk") && (d.ret == "_Bool" || d.ret == "bool") {
            failures.push(format!("{}: Swift-CC Bool return; declare uint8_t", d.name));
        }
        // The thunk's C name may carry the boundary namespace (apus_<tag>_apus_<name>).
        if d.marker == "thunk" && !decls.iter().any(|t| t.name.ends_with(&format!("apus_{}", d.name))) {
            failures.push(format!("{0}: APUS_THUNK without apus_{0}", d.name));
        }
    }
    failures
}

fn main() {
    let decls = ast::declarations(&[]);
    let mut failures = rule_failures(&decls);
    let direct: Vec<&Decl> = decls.iter().filter(|d| d.marker == "direct" && d.kind != Kind::Data).collect();
    let source = std::iter::once("#include \"apus/abi.h\"\n// objdump labels the first function ltmp0: keep the callers off that spot.\nvoid zz_first(void) {}\n".to_string()).chain(direct.iter().map(|d| caller(d))).collect::<String>();
    for arch in ["arm64", "x86_64"] {
        let (swift, c) = (disassemble(&source, arch, "swift", false), disassemble(&source, arch, "c", true));
        for d in &direct {
            let (a, b) = (body(&swift, &d.name), body(&c, &d.name));
            if a != b {
                failures.push(format!("{arch} {}: lowers differently from C; mark it APUS_THUNK\n{a}\n---\n{b}", d.name));
            }
        }
    }
    std::fs::remove_dir_all(scratch()).ok();
    if direct.is_empty() {
        failures.push("no APUS_DIRECT declarations found".to_string());
    }
    if failures.is_empty() {
        println!("lowering ... ok ({} direct declarations × 2 archs)", direct.len());
    } else {
        eprintln!("{}", failures.join("\n\n"));
        std::process::exit(1);
    }
}
