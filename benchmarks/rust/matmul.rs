fn main() {
    let n = 200usize;
    let a: Vec<i64> = (0..n * n).map(|i| (i % 7) as i64).collect();
    let b: Vec<i64> = (0..n * n).map(|i| (i % 5) as i64).collect();
    let mut c = vec![0i64; n * n];
    for i in 0..n {
        for k in 0..n {
            let aik = a[i * n + k];
            if aik != 0 {
                for j in 0..n {
                    c[i * n + j] += aik * b[k * n + j];
                }
            }
        }
    }
    let total: i64 = c.iter().sum();
    println!("{total}");
}
