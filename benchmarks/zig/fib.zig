const std = @import("std");
fn fib(n: i64) i64 {
    return if (n < 2) n else fib(n - 1) + fib(n - 2);
}
pub fn main() void {
    std.debug.print("{d}\n", .{fib(30)});
}
