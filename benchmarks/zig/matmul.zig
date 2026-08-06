const std = @import("std");
pub fn main() !void {
    const n: usize = 200;
    const alloc = std.heap.page_allocator;
    const a = try alloc.alloc(i64, n * n);
    const b = try alloc.alloc(i64, n * n);
    const c = try alloc.alloc(i64, n * n);
    defer alloc.free(a);
    defer alloc.free(b);
    defer alloc.free(c);
    for (a, b, c, 0..) |*av, *bv, *cv, i| {
        av.* = @as(i64, @intCast(i % 7));
        bv.* = @as(i64, @intCast(i % 5));
        cv.* = 0;
    }
    var i: usize = 0;
    while (i < n) : (i += 1) {
        var k: usize = 0;
        while (k < n) : (k += 1) {
            const aik = a[i * n + k];
            if (aik != 0) {
                var j: usize = 0;
                while (j < n) : (j += 1) c[i * n + j] += aik * b[k * n + j];
            }
        }
    }
    var total: i64 = 0;
    for (c) |v| total += v;
    std.debug.print("{d}\n", .{total});
}
