// Dirty heap, Rust Rc<RefCell<Node>> variant: edges are strong Rc pointers,
// which is the shape a "graph of objects" takes without an arena. The graph is
// cyclic, so dropping the Vec of roots alone would leak every cycle; the
// teardown is clearing each node's edges (breaking the cycles) and then
// dropping the Vec — the same "release every node" Grasm's refs variant does.
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Instant;

struct Node {
    url: String,
    edges: Vec<Rc<RefCell<Node>>>,
    seen: bool,
}

fn lcg(x: i64) -> i64 {
    (x * 1103515245 + 12345) % 2147483648
}

fn build(n: usize) -> Vec<Rc<RefCell<Node>>> {
    let nodes: Vec<_> = (0..n)
        .map(|i| {
            Rc::new(RefCell::new(Node {
                url: format!("https://site{}.example/page/{}", i % 1000, i),
                edges: Vec::new(),
                seen: false,
            }))
        })
        .collect();
    let mut s: i64 = 42;
    for node in &nodes {
        s = lcg(s);
        let deg = 1 + s % 9;
        let mut node = node.borrow_mut();
        node.edges.reserve(deg as usize);
        for _ in 0..deg {
            s = lcg(s);
            node.edges.push(Rc::clone(&nodes[(s % n as i64) as usize]));
        }
    }
    nodes
}

fn bfs(root: &Rc<RefCell<Node>>) -> (usize, usize) {
    let mut queue = VecDeque::new();
    root.borrow_mut().seen = true;
    queue.push_back(Rc::clone(root));
    let (mut reached, mut bytes) = (0, 0);
    while let Some(v) = queue.pop_front() {
        reached += 1;
        let v = v.borrow();
        bytes += v.url.len();
        for w in &v.edges {
            // a self-loop is already marked, so it is never borrowed mutably here
            if !w.borrow().seen {
                w.borrow_mut().seen = true;
                queue.push_back(Rc::clone(w));
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
    let (reached, bytes) = bfs(&graph[0]);
    println!("checksum nodes={n} reachable={reached} url_bytes={bytes}");
    let t0 = Instant::now();
    for node in &graph {
        node.borrow_mut().edges.clear();
    }
    drop(graph);
    let drop_t = t0.elapsed();
    println!("build_ms {}", built.as_millis());
    println!("drop_ms {:.3}", drop_t.as_secs_f64() * 1e3);
    println!("total_ms {}", t_start.elapsed().as_millis());
}
