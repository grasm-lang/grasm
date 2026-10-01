// Dirty heap, Rust arena: Vec<Node> with usize edge indices. The drop is the
// Vec going away, which frees every URL String and every edge Vec.
use std::collections::VecDeque;
use std::time::Instant;

struct Node {
    url: String,
    edges: Vec<usize>,
}

fn lcg(x: i64) -> i64 {
    (x * 1103515245 + 12345) % 2147483648
}

fn build(n: usize) -> Vec<Node> {
    let mut nodes = Vec::with_capacity(n);
    let mut s: i64 = 42;
    for i in 0..n {
        s = lcg(s);
        let deg = 1 + s % 9;
        let mut edges = Vec::with_capacity(deg as usize);
        for _ in 0..deg {
            s = lcg(s);
            edges.push((s % n as i64) as usize);
        }
        nodes.push(Node { url: format!("https://site{}.example/page/{}", i % 1000, i), edges });
    }
    nodes
}

fn bfs(nodes: &[Node]) -> (usize, usize) {
    let mut seen = vec![false; nodes.len()];
    let mut queue = VecDeque::new();
    seen[0] = true;
    queue.push_back(0);
    let (mut reached, mut bytes) = (0, 0);
    while let Some(v) = queue.pop_front() {
        reached += 1;
        bytes += nodes[v].url.len();
        for &w in &nodes[v].edges {
            if !seen[w] {
                seen[w] = true;
                queue.push_back(w);
            }
        }
    }
    (reached, bytes)
}

fn main() {
    let t_start = Instant::now();
    let n: usize = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(5_000_000);
    let graph = build(n);
    let built = t_start.elapsed();
    let (reached, bytes) = bfs(&graph);
    println!("checksum nodes={n} reachable={reached} url_bytes={bytes}");
    let t0 = Instant::now();
    drop(graph);
    let drop_t = t0.elapsed();
    println!("build_ms {}", built.as_millis());
    println!("drop_ms {:.3}", drop_t.as_secs_f64() * 1e3);
    println!("total_ms {}", t_start.elapsed().as_millis());
}
