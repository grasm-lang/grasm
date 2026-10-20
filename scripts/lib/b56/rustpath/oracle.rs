// The oracle for packages/rustpath: Rust 1.99's std::path::Path on Unix.
// Text travels as hex ("-" when empty); an answer is s:<hex>, "none",
// true/false, a list of s:<hex> joined by ",", or P: and a panic message.
use std::io::{self, BufRead, Write};
use std::path::Path;

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

fn opt(o: Option<&std::ffi::OsStr>) -> String {
    match o {
        Some(x) => format!("s:{}", hex(x.to_str().unwrap())),
        None => "none".to_string(),
    }
}

fn path_s(p: &Path) -> String {
    format!("s:{}", hex(p.to_str().unwrap()))
}

fn answer(f: &[&str]) -> String {
    let a = unhex(f[1]);
    let p = Path::new(&a);
    match f[0] {
        "comp" => p.components().map(|c| format!("s:{}", hex(c.as_os_str().to_str().unwrap()))).collect::<Vec<_>>().join(","),
        "parent" => p.parent().map(path_s).unwrap_or("none".to_string()),
        "name" => opt(p.file_name()),
        "stem" => opt(p.file_stem()),
        "prefix" => opt(p.file_prefix()),
        "ext" => opt(p.extension()),
        "abs" => p.is_absolute().to_string(),
        "join" => path_s(&p.join(unhex(f[2]))),
        "wfn" => path_s(&p.with_file_name(unhex(f[2]))),
        "wext" => {
            let e = unhex(f[2]);
            match std::panic::catch_unwind(|| p.with_extension(&e)) {
                Ok(q) => path_s(&q),
                Err(m) => format!("P:{}", m.downcast_ref::<String>().cloned().unwrap_or_default()),
            }
        }
        "strip" => p.strip_prefix(unhex(f[2])).map(path_s).unwrap_or("none".to_string()),
        "starts" => p.starts_with(unhex(f[2])).to_string(),
        _ => p.ends_with(unhex(f[2])).to_string(),
    }
}

fn main() {
    std::panic::set_hook(Box::new(|_| {}));
    let stdin = io::stdin();
    let mut out = String::new();
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        if line.is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split(' ').skip(1).collect();
        out.push_str(&answer(&f));
        out.push('\n');
    }
    io::stdout().write_all(out.as_bytes()).unwrap();
}
