//! Every builtin free function, and the effect row calling one performs.
//!
//! **This is the only place either fact is written down.** The checker's list
//! of names, the effect system's three lists of what performs what, and the
//! conformance program that proves it are all derived from this table or
//! checked against it by a test that fails the build.
//!
//! It exists because they were four separate hand-maintained lists, and two of
//! them disagreed. `read_bytes` and `write_bytes` shipped with [L-LIB-BYTES]
//! beside `read_file` and `write_file`, were added to the checker's list and
//! not to the effect system's — so a function declaring `/ {}` could read any
//! file on the disk, and a package declaring `capabilities = []` could too.
//! That is the property [T-CAPABILITY] exists to provide, and it was open from
//! the day `bytes` shipped until it was found by reading the two lists side by
//! side (ProjectGold F35).
//!
//! Nothing about that bug was subtle. It was a name missing from a list nobody
//! had a reason to look at, and no test could see it because every test was
//! written against one list or the other. The fix for *that* is not to be more
//! careful; it is to make the second list not exist.
//!
//! ## Adding a builtin
//!
//! Add it here, with its row. Everything else either follows or fails:
//!
//! * the checker's `BUILTIN_FREE_FNS` must contain exactly these names, or
//!   `builtin_names_agree_with_the_table` fails;
//! * an effectful one must be named in `spec/conformance/T-EFFECT-BUILTIN.gr`
//!   inside a function declaring that row, or
//!   `every_effectful_builtin_is_named_in_conformance` fails;
//! * and the row here is the row the checker charges, because the effect system
//!   reads this table and has no list of its own.

/// The effects a builtin performs. Empty means it reaches nothing outside the
/// program — `bytes_of` converts a list, and `panic` ends the process without
/// observing anything.
pub struct Builtin {
    pub name: &'static str,
    pub row: &'static [&'static str],
}

/// Reaching the world outside the program: the filesystem, the clock, the
/// environment, the terminal.
pub const IO: &str = "io";

/// The network.
pub const NET: &str = "net";

/// Running another program. Strictly larger than either `io` or `net`: a
/// program that can run `curl` has the network whether or not it declared
/// `net`, and one that can run `rm` has the filesystem. Folding it into `io`
/// would make every capability claim in the language weaker than it reads.
pub const PROC: &str = "proc";

/// Calling a C function. Also charged `io`, because what a C function does
/// cannot be known.
pub const UNSAFE: &str = "unsafe";

/// Concurrency.
pub const ASYNC: &str = "async";

const IO_ROW: &[&str] = &[IO];
const NET_ROW: &[&str] = &[NET];
const PROC_ROW: &[&str] = &[PROC];
const PURE: &[&str] = &[];

pub const BUILTINS: &[Builtin] = &[
    // --- the terminal ---
    Builtin { name: "print", row: IO_ROW },
    Builtin { name: "eprint", row: IO_ROW },
    Builtin { name: "input", row: IO_ROW },
    // --- the filesystem ---
    Builtin { name: "read_file", row: IO_ROW },
    Builtin { name: "write_file", row: IO_ROW },
    Builtin { name: "read_bytes", row: IO_ROW },
    Builtin { name: "write_bytes", row: IO_ROW },
    Builtin { name: "file_exists", row: IO_ROW },
    Builtin { name: "remove_file", row: IO_ROW },
    Builtin { name: "make_dir", row: IO_ROW },
    Builtin { name: "list_dir", row: IO_ROW },
    // --- the process's own surroundings ---
    Builtin { name: "args", row: IO_ROW },
    Builtin { name: "env", row: IO_ROW },
    Builtin { name: "now_millis", row: IO_ROW },
    // Reading entropy is reaching outside the program, and two calls do not
    // agree — which is the same reason `now_millis` is charged. A pure function
    // that answered differently each time would break every promise the effect
    // row makes.
    Builtin { name: "random_bytes", row: IO_ROW },
    // --- the network ---
    Builtin { name: "tcp_listen", row: NET_ROW },
    Builtin { name: "tcp_connect", row: NET_ROW },
    Builtin { name: "tcp_accept", row: NET_ROW },
    Builtin { name: "tcp_read", row: NET_ROW },
    Builtin { name: "tcp_read_bytes", row: NET_ROW },
    Builtin { name: "tcp_write", row: NET_ROW },
    Builtin { name: "tcp_write_bytes", row: NET_ROW },
    Builtin { name: "net_close", row: NET_ROW },
    Builtin { name: "net_wait", row: NET_ROW },
    Builtin { name: "net_port", row: NET_ROW },
    Builtin { name: "udp_bind", row: NET_ROW },
    Builtin { name: "udp_send", row: NET_ROW },
    Builtin { name: "udp_recv", row: NET_ROW },
    // --- other programs ---
    Builtin { name: "proc_run", row: PROC_ROW },
    Builtin { name: "proc_pipeline", row: PROC_ROW },
    // --- reaching nothing ---
    //
    // `bytes_of` converts a list. The other three end the program: they observe
    // nothing and return nothing, and charging them an effect would mean a
    // function could not assert an invariant without declaring `io`. That is a
    // deliberate exemption and it is the reason this row is written out rather
    // than left implied — a builtin with no row should be a decision, not an
    // omission, which is exactly the distinction F35 turned on.
    Builtin { name: "bytes_of", row: PURE },
    Builtin { name: "exit", row: PURE },
    Builtin { name: "panic", row: PURE },
    Builtin { name: "assert", row: PURE },
    Builtin { name: "assert_eq", row: PURE },
];

/// The row calling `name` performs, or empty if it is not a builtin.
pub fn row_of(name: &str) -> &'static [&'static str] {
    for b in BUILTINS {
        if b.name == name {
            return b.row;
        }
    }
    PURE
}

/// Whether `name` is a builtin free function.
pub fn is_builtin(name: &str) -> bool {
    BUILTINS.iter().any(|b| b.name == name)
}

/// Whether calling `name` performs `effect`.
pub fn performs(name: &str, effect: &str) -> bool {
    row_of(name).contains(&effect)
}

/// Every builtin that performs at least one effect.
pub fn effectful() -> impl Iterator<Item = &'static Builtin> {
    BUILTINS.iter().filter(|b| !b.row.is_empty())
}
