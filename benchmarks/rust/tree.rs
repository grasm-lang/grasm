// Allocation-shaped work: build a tree, walk it, drop it, repeat.
struct Node {
    value: i64,
    kids: Vec<Node>,
}

fn build(depth: i64, branch: i64, seed: i64) -> Node {
    if depth <= 0 {
        return Node { value: seed % 1000, kids: Vec::new() };
    }
    let mut kids = Vec::with_capacity(branch as usize);
    for i in 0..branch {
        kids.push(build(depth - 1, branch, seed * 31 + i + 1));
    }
    Node { value: seed % 1000, kids }
}

fn total(n: &Node) -> i64 {
    let mut sum = n.value;
    for k in &n.kids {
        sum += total(k);
    }
    sum
}

fn main() {
    let mut acc: i64 = 0;
    for r in 0..20i64 {
        let t = build(9, 3, r + 1);
        acc = (acc + total(&t)) % 1_000_000_007;
    }
    println!("{acc}");
}
