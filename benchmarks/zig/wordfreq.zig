const std = @import("std");
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
    std.debug.print("{d} {d}\n", .{ counts.count(), total });
}
