fn main() {
    let n = 400_000i64;
    let mut xs: Vec<i64> = Vec::with_capacity(n as usize);
    let mut seed: i64 = 12345;
    for _ in 0..n {
        seed = (seed * 1103515245 + 12345) % 2147483648;
        xs.push(seed % 1000000);
    }
    xs.sort();
    let mut checksum: i64 = 0;
    for (k, v) in xs.iter().enumerate() {
        checksum = (checksum + v * ((k as i64) % 7 + 1)) % 1000000007;
    }
    println!("{} {}", n, checksum);
}
