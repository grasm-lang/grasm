// ECS update loop, array-of-structs: Vec<Entity> updated in place.
// Usage: ecs [N] [S]   (defaults N=1000000, S=100)

#[derive(Clone, Copy)]
struct Entity {
    x: f64,
    y: f64,
    z: f64,
    vx: f64,
    vy: f64,
    vz: f64,
}

fn next(s: i64) -> i64 {
    (s * 1103515245 + 12345) % 2147483648
}

#[inline]
fn reflect(p: &mut f64, v: &mut f64) {
    if *p > 1000.0 {
        *p = 2000.0 - *p;
        *v = -*v;
    } else if *p < -1000.0 {
        *p = -2000.0 - *p;
        *v = -*v;
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1_000_000);
    let steps: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(100);
    let dt = 0.01;

    let mut s: i64 = 42;
    let mut pos = |m: i64, off: i64, div: f64| {
        s = next(s);
        (s % m - off) as f64 / div
    };
    let mut es: Vec<Entity> = Vec::with_capacity(n);
    for _ in 0..n {
        let x = pos(2000001, 1000000, 1000.0);
        let y = pos(2000001, 1000000, 1000.0);
        let z = pos(2000001, 1000000, 1000.0);
        let vx = pos(2001, 1000, 10.0);
        let vy = pos(2001, 1000, 10.0);
        let vz = pos(2001, 1000, 10.0);
        es.push(Entity { x, y, z, vx, vy, vz });
    }

    for _ in 0..steps {
        for e in es.iter_mut() {
            e.x += e.vx * dt;
            e.y += e.vy * dt;
            e.z += e.vz * dt;
            reflect(&mut e.x, &mut e.vx);
            reflect(&mut e.y, &mut e.vy);
            reflect(&mut e.z, &mut e.vz);
        }
    }

    let mut sum = 0.0f64;
    let mut isum: i64 = 0;
    for e in &es {
        sum += e.x + e.y + e.z;
        isum += (e.x * 1000.0).floor() as i64 + (e.y * 1000.0).floor() as i64 + (e.z * 1000.0).floor() as i64;
    }
    println!("checksum {:.6} {}", sum, isum);
}
