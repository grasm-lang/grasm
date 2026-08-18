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

fn fib(n: i64) i64 {
    return if (n < 2) n else fib(n - 1) + fib(n - 2);
}
pub fn main() !void {
    try emit("{d}\n", .{fib(30)});
}
