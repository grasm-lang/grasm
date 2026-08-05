// std::thread::scope — no external crates, so this stays comparable with the
// others. (Rayon would be the idiomatic choice and is broadly similar.)
fn mix(seed: i64) -> i64 {
    let mut acc = seed;
    for _ in 0..800 {
        acc = (acc * 1103515245 + 12345) % 2147483648;
    }
    acc % 1000
}

fn main() {
    const N: usize = 200_000;
    let xs: Vec<i64> = (0..N as i64).collect();
    let mut out = vec![0i64; N];
    let workers = std::thread::available_parallelism().map(|p| p.get()).unwrap_or(1);
    let chunk = N.div_ceil(workers);
    std::thread::scope(|s| {
        for (ci, slot) in out.chunks_mut(chunk).enumerate() {
            let src = &xs[ci * chunk..ci * chunk + slot.len()];
            s.spawn(move || {
                for (o, &x) in slot.iter_mut().zip(src.iter()) {
                    *o = mix(x);
                }
            });
        }
    });
    let total: i64 = out.iter().sum();
    println!("{total}");
}
