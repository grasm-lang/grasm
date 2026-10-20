// The oracle for packages/rustnum: Rust's own float formatting and str::parse.
// Doubles travel as their bits in hex; text as hex, '-' when empty.
use std::io::{self, BufRead, Write};

fn unhex(h: &str) -> String {
    if h == "-" {
        return String::new();
    }
    let b: Vec<u8> = (0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap()).collect();
    String::from_utf8_lossy(&b).into_owned()
}

fn hx(s: &str) -> String {
    if s.is_empty() {
        return "-".to_string();
    }
    s.bytes().map(|b| format!("{:02x}", b)).collect()
}

fn int<T: std::str::FromStr<Err = std::num::ParseIntError> + std::fmt::Display>(s: &str) -> String {
    match s.parse::<T>() {
        Ok(v) => format!("{}", v),
        Err(e) => format!("E:{:?}:{}", e.kind(), hx(&e.to_string())),
    }
}

fn main() {
    let stdin = io::stdin();
    let out = io::stdout();
    let mut out = io::BufWriter::new(out.lock());
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        if line.is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split(' ').collect();
        let x = || f64::from_bits(u64::from_str_radix(f[1], 16).unwrap());
        let r = match f[0] {
            "d" => hx(&format!("{}", x())),
            "g" => hx(&format!("{:?}", x())),
            "f" => hx(&format!("{:.*}", f[2].parse::<usize>().unwrap(), x())),
            "e" => hx(&if f[2] == "1" { format!("{:E}", x()) } else { format!("{:e}", x()) }),
            "x" => {
                let n = f[2].parse::<usize>().unwrap();
                hx(&if f[3] == "1" { format!("{:.*E}", n, x()) } else { format!("{:.*e}", n, x()) })
            }
            "p" => match unhex(f[1]).parse::<f64>() {
                Ok(v) => format!("{:016x}", if v.is_nan() { 0x7ff8000000000000u64 } else { v.to_bits() }),
                Err(e) => format!("E:{}", hx(&e.to_string())),
            },
            _ => {
                let s = unhex(f[1]);
                match f[2] {
                    "i8" => int::<i8>(&s),
                    "i16" => int::<i16>(&s),
                    "i32" => int::<i32>(&s),
                    "i64" => int::<i64>(&s),
                    "u8" => int::<u8>(&s),
                    "u16" => int::<u16>(&s),
                    "u32" => int::<u32>(&s),
                    _ => int::<u64>(&s),
                }
            }
        };
        writeln!(out, "{}", r).unwrap();
    }
}
