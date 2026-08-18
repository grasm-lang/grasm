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
    const words = [_][]const u8{ "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta" };
    var arena = std.heap.ArenaAllocator.init(std.heap.page_allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var counts = std.StringHashMap(i64).init(alloc);
    defer counts.deinit();
    var buf: [64]u8 = undefined;
    var i: usize = 0;
    while (i < 400_000) : (i += 1) {
        const key = try std.fmt.bufPrint(&buf, "{s}{d}", .{ words[i % 8], i % 97 });
        const gop = try counts.getOrPut(key);
        if (!gop.found_existing) {
            gop.key_ptr.* = try alloc.dupe(u8, key);
            gop.value_ptr.* = 0;
        }
        gop.value_ptr.* += 1;
    }
    var total: i64 = 0;
    var it = counts.valueIterator();
    while (it.next()) |v| total += v.*;
    try emit("{d} {d}\n", .{ counts.count(), total });
}
