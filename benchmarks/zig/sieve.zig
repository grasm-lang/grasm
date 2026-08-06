const std = @import("std");
pub fn main() !void {
    const limit: usize = 2_000_000;
    const alloc = std.heap.page_allocator;
    const flags = try alloc.alloc(bool, limit + 1);
    defer alloc.free(flags);
    for (flags) |*f| f.* = true;
    var count: i64 = 0;
    var i: usize = 2;
    while (i <= limit) : (i += 1) {
        if (flags[i]) {
            count += 1;
            var j = i * i;
            while (j <= limit) : (j += i) flags[j] = false;
        }
    }
    std.debug.print("{d}\n", .{count});
}
