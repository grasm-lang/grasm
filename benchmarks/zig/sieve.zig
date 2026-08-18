const std = @import("std");
/// The benchmark harness compares stdout, as it does for every other language
/// here. `std.debug.print` writes to *stderr*, so these programs reported no
/// output at all and the table recorded them as wrong answers — an accusation
/// against Zig caused entirely by the wrong stream on our side.
fn emit(comptime fmt: []const u8, args: anytype) !void {
    var threaded: std.Io.Threaded = .init(std.heap.page_allocator, .{});
    defer threaded.deinit();
    var buf: [64]u8 = undefined;
    var w = std.Io.File.stdout().writer(threaded.io(), &buf);
    try w.interface.print(fmt, args);
    try w.interface.flush();
}

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
    try emit("{d}\n", .{count});
}
