// std::thread::scope — no external crates, so this stays comparable with the
// others. (Rayon would be the idiomatic choice and is broadly similar.)
//
// The text is ASCII, so `len()` is the character count here as well as the
// byte count, and every implementation in this set takes the O(1) answer its
// own language gives.
fn heavy(w: &str) -> String {
    let mut acc = String::from(w);
    for i in 0..200 {
        acc.push((b'0' + (i % 7) as u8) as char);
    }
    format!("{}-{}", acc.len(), w)
}

fn main() {
    const N: usize = 20_000;
    let words: Vec<String> = (0..N).map(|i| format!("w{i}")).collect();
    let mut out: Vec<String> = vec![String::new(); N];
    let workers = std::thread::available_parallelism().map(|p| p.get()).unwrap_or(1);
    let chunk = N.div_ceil(workers);
    std::thread::scope(|s| {
        for (ci, slot) in out.chunks_mut(chunk).enumerate() {
            let src = &words[ci * chunk..ci * chunk + slot.len()];
            s.spawn(move || {
                for (o, w) in slot.iter_mut().zip(src.iter()) {
                    *o = heavy(w);
                }
            });
        }
    });
    let mut h: i64 = 0;
    for s in &out {
        for b in s.bytes() {
            h = (h * 131 + b as i64) % 1_000_000_007;
        }
    }
    println!("{h}");
}
