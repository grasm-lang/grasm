// Dumps the two character sets packages/rustescape needs, as ranges, from
// what Rust's escape_debug does: N (escaped wherever it stands: not
// printable) and G (printable, escaped only as a string's first char or
// in a char: Grapheme_Extend).
fn main() {
    let mut n: Vec<(u32, u32)> = Vec::new();
    let mut g: Vec<(u32, u32)> = Vec::new();
    for cp in 0u32..=0x10FFFF {
        let c = match char::from_u32(cp) { Some(c) => c, None => continue };
        if matches!(c, '\0' | '\t' | '\r' | '\n' | '\\' | '\'' | '"') { continue; }
        let lit = c.to_string();
        let rest: String = format!("a{}", c).escape_debug().to_string();
        let printable = rest == format!("a{}", c);
        let first = c.escape_debug().to_string() != lit;
        let (set, on) = if !printable { (&mut n, true) } else { (&mut g, first) };
        if on {
            match set.last_mut() { Some(r) if r.1 + 1 == cp => r.1 = cp, _ => set.push((cp, cp)) }
        }
    }
    for (name, set) in [("N", &n), ("G", &g)] {
        println!("{} {}", name, set.iter().map(|r| format!("{} {}", r.0, r.1)).collect::<Vec<_>>().join(" "));
    }
}
