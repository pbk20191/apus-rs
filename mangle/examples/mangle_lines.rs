//! Mangles one declaration per line of stdin; prints `OK <symbol>` or `ERR <reason>` per line, in order.
//! Used by scripts/sdk-coverage.py to compare against the symbols the SDK's symbol graphs name.
use std::io::{BufRead, Write};

fn main() {
    let stdin = std::io::stdin();
    let mut out = std::io::BufWriter::new(std::io::stdout());
    for line in stdin.lock().lines() {
        let line = line.expect("stdin");
        match apus_mangle::mangle(&line) {
            Ok(symbol) => writeln!(out, "OK {symbol}"),
            Err(error) => writeln!(out, "ERR {}", error.to_string().replace('\n', " ")),
        }
        .expect("stdout");
    }
}
