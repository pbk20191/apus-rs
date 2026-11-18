//! Mangles one declaration per line of stdin; prints `OK <symbol>`, `NOCALL <symbol>\t<reason>` (mangled, but
//! `swift!` refuses to call it) or `ERR <reason>` per line, in order.
//! Used by scripts/sdk-coverage.py to compare against the symbols the SDK's symbol graphs name.
use std::io::{BufRead, Write};

fn main() {
    let stdin = std::io::stdin();
    let mut out = std::io::BufWriter::new(std::io::stdout());
    for line in stdin.lock().lines() {
        let line = line.expect("stdin");
        match apus_mangle::mangle(&line) {
            Ok(symbol) => match apus_mangle::check_callable(&line) {
                // An enum case is not called: its tag symbol is read.
                Err(error) if !line.starts_with("case ") => {
                    writeln!(out, "NOCALL {symbol}\t{}", error.to_string().replace('\n', " "))
                }
                _ => writeln!(out, "OK {symbol}"),
            },
            Err(error) => writeln!(out, "ERR {}", error.to_string().replace('\n', " ")),
        }
        .expect("stdout");
    }
}
