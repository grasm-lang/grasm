use std::collections::HashMap;
fn main() {
    let words = ["alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta"];
    let mut counts: HashMap<String, i64> = HashMap::new();
    for i in 0..400_000i64 {
        let w = format!("{}{}", words[(i % 8) as usize], i % 97);
        *counts.entry(w).or_insert(0) += 1;
    }
    let total: i64 = counts.values().sum();
    println!("{} {}", counts.len(), total);
}
