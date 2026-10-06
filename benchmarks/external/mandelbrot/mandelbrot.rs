// The same kernel as benchmarks/c/mandelbrot.c, single-threaded.
#[inline]
fn escapes(cx: f64, cy: f64) -> i32 {
    let (mut zx, mut zy) = (0.0f64, 0.0f64);
    for i in 0..200 {
        if zx * zx + zy * zy > 4.0 {
            return i;
        }
        let tx = zx * zx - zy * zy + cx;
        zy = 2.0 * zx * zy + cy;
        zx = tx;
    }
    200
}

fn main() {
    let mut total: i64 = 0;
    for row in 0..2000 {
        let cy = row as f64 / 2000.0 * 2.4 - 1.2;
        for col in 0..3000 {
            let cx = col as f64 / 3000.0 * 3.0 - 2.0;
            if escapes(cx, cy) == 200 {
                total += 1;
            }
        }
    }
    println!("{}", total);
}
