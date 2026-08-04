fn main() {
    let limit = 2_000_000usize;
    let mut flags = vec![true; limit + 1];
    let mut count = 0i64;
    for i in 2..=limit {
        if flags[i] {
            count += 1;
            let mut j = i * i;
            while j <= limit {
                flags[j] = false;
                j += i;
            }
        }
    }
    println!("{count}");
}
