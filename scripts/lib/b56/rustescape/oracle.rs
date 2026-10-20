// The oracle for packages/rustescape: Rust 1.99's char and str escapes
// and `{:?}`. Text travels as hex of UTF-8 ("-" when empty); the answer
// is the hex of what Rust wrote.
use std::io::{self, BufRead, Write};

fn unhex(h: &str) -> String {
    if h == "-" {
        return String::new();
    }
    let b: Vec<u8> = (0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap()).collect();
    String::from_utf8(b).unwrap()
}

fn hex(s: &str) -> String {
    if s.is_empty() {
        return "-".to_string();
    }
    s.bytes().map(|b| format!("{:02x}", b)).collect()
}

fn main() {
    let stdin = io::stdin();
    let mut out = io::BufWriter::new(io::stdout());
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        let f: Vec<&str> = line.split(' ').collect();
        if f.len() < 3 {
            continue;
        }
        let s = unhex(f[2]);
        let c = s.chars().next().unwrap_or('\0');
        let r = match f[1] {
            "cd" => c.escape_debug().to_string(),
            "cf" => c.escape_default().to_string(),
            "cu" => c.escape_unicode().to_string(),
            "sd" => s.escape_debug().to_string(),
            "sf" => s.escape_default().to_string(),
            "su" => s.escape_unicode().to_string(),
            "ds" => format!("{:?}", s),
            _ => format!("{:?}", c),
        };
        writeln!(out, "{}", hex(&r)).unwrap();
    }
}
