//! Recording a run's effects, and replaying them.
//!
//! A program is a function of its code and everything it read from outside
//! itself. The code is in the binary; the rest is what this records — so a
//! failure that happened once can be made to happen again, on a different
//! machine, under a debugger, as many times as it takes.
//!
//! **The effect system is what makes this cheap.** Knowing which calls are
//! non-deterministic is normally the hard part of building a record-and-replay
//! tool, and here it is already written down: a builtin either declares an
//! effect row or it does not, and one that does not cannot read anything.
//! `crates/grasm_sema/src/builtins.rs` is the list, and the same test that
//! keeps it honest for capabilities keeps it honest for this.
//!
//! ## What is recorded
//!
//! Every call to a builtin that **reads** from outside the program: the file
//! surface, the clock, the environment, arguments, stdin, the network, other
//! processes, entropy. Each gets one journal record, in call order.
//!
//! Calls that only **write** — `print`, `write_file` — are recorded as having
//! happened and are *not* replayed against the world a second time. Replaying a
//! `write_file` would damage a disk to reproduce a bug, and replaying a `print`
//! would hide the output the person is reading.
//!
//! ## Divergence is an error, not a guess
//!
//! On replay the recorded call name is checked against the call actually made.
//! If they differ the program has taken a different path from the one recorded
//! — usually because the code changed — and the run stops and says so.
//!
//! The alternative is to hand back the next record anyway, which produces a
//! run that looks like a replay and is not. A tool whose whole promise is *this
//! is what happened* must not be able to lie about it.
//!
//! ## Format
//!
//! `GRASMJRNL2` then one record each:
//!
//! ```text
//! u32 name_len | name | u32 value_len | value
//! ```
//!
//! The value is a [`Wire`] — one portable encoding of a Grasm value, shared by
//! every engine. That sharing is the point: the interpreter and the compiled
//! runtime have entirely separate value representations, and a journal that
//! spoke either one would only replay under the engine that wrote it. A
//! recording is most useful on the engine you did *not* record it with.
//!
//! Length-prefixed rather than a text format because a recorded value is
//! arbitrary bytes — a `read_bytes` of a JPEG is exactly the case this exists
//! to reproduce, and it is the one a text format would corrupt.

#![no_std]
extern crate alloc;
#[macro_use]
extern crate grasm_sys;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

/// `eprintln!` without `std`, over `grasm_sys`'s buffered stderr.
macro_rules! eprint_line {
    ($($arg:tt)*) => {{
        use grasm_sys::io::Write;
        let mut err = grasm_sys::io::stderr().lock();
        let _ = writeln!(err, $($arg)*);
    }};
}

use core::cell::RefCell;

const MAGIC: &[u8; 10] = b"GRASMJRNL2";

/// A Grasm value in the one shape both engines can read.
///
/// Only the shapes a builtin can answer with. Anything else reaching here is a
/// mistake in a hook rather than a value the journal should try to guess at,
/// and [`Wire::of_unsupported`] says so by name rather than recording something
/// that will decode into the wrong thing later.
#[derive(Debug, Clone, PartialEq)]
pub enum Wire {
    Unit,
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Bytes(Vec<u8>),
    List(Vec<Wire>),
    Tuple(Vec<Wire>),
    /// `Ok`/`Err`/`Some`/`None`/`IoError` — every fallible builtin answers with
    /// one of these.
    Variant {
        ty: String,
        name: String,
        fields: Vec<Wire>,
    },
}

impl Wire {
    /// The name a hook should pass when it meets a value this cannot carry.
    /// Recording it would produce a journal that decodes into something else.
    pub fn of_unsupported(what: &str) -> ! {
        eprint_line!(
            "grasm: cannot record a {what} — the journal has no encoding for it, \
             and recording something else in its place would make the replay a \
             lie about what happened"
        );
        grasm_sys::sys::exit(7);
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.write_into(&mut out);
        out
    }

    fn write_into(&self, out: &mut Vec<u8>) {
        fn s(out: &mut Vec<u8>, text: &str) {
            out.extend((text.len() as u32).to_le_bytes());
            out.extend(text.as_bytes());
        }
        fn many(out: &mut Vec<u8>, items: &[Wire]) {
            out.extend((items.len() as u32).to_le_bytes());
            for i in items {
                i.write_into(out);
            }
        }
        match self {
            Wire::Unit => out.push(0),
            Wire::Int(v) => {
                out.push(1);
                out.extend(v.to_le_bytes());
            }
            Wire::Float(v) => {
                out.push(2);
                out.extend(v.to_bits().to_le_bytes());
            }
            Wire::Bool(v) => {
                out.push(3);
                out.push(u8::from(*v));
            }
            Wire::Str(v) => {
                out.push(4);
                s(out, v);
            }
            Wire::Bytes(v) => {
                out.push(5);
                out.extend((v.len() as u32).to_le_bytes());
                out.extend(v);
            }
            Wire::List(v) => {
                out.push(6);
                many(out, v);
            }
            Wire::Tuple(v) => {
                out.push(7);
                many(out, v);
            }
            Wire::Variant { ty, name, fields } => {
                out.push(8);
                s(out, ty);
                s(out, name);
                many(out, fields);
            }
        }
    }

    pub fn decode(raw: &[u8]) -> Result<Wire, String> {
        let mut at = 0;
        let w = Wire::read_from(raw, &mut at)?;
        if at != raw.len() {
            return Err("trailing bytes after a recorded value".to_string());
        }
        Ok(w)
    }

    fn read_from(raw: &[u8], at: &mut usize) -> Result<Wire, String> {
        fn take<'a>(raw: &'a [u8], at: &mut usize, n: usize) -> Result<&'a [u8], String> {
            if *at + n > raw.len() {
                return Err("a recorded value ends early".to_string());
            }
            let v = &raw[*at..*at + n];
            *at += n;
            Ok(v)
        }
        fn u32_of(raw: &[u8], at: &mut usize) -> Result<usize, String> {
            let b: [u8; 4] = take(raw, at, 4)?.try_into().map_err(|_| "bad length")?;
            Ok(u32::from_le_bytes(b) as usize)
        }
        fn text(raw: &[u8], at: &mut usize) -> Result<String, String> {
            let n = u32_of(raw, at)?;
            Ok(String::from_utf8_lossy(take(raw, at, n)?).into_owned())
        }
        fn many(raw: &[u8], at: &mut usize) -> Result<Vec<Wire>, String> {
            let n = u32_of(raw, at)?;
            let mut out = Vec::with_capacity(n.min(4096));
            for _ in 0..n {
                out.push(Wire::read_from(raw, at)?);
            }
            Ok(out)
        }
        let tag = take(raw, at, 1)?[0];
        Ok(match tag {
            0 => Wire::Unit,
            1 => {
                let b: [u8; 8] = take(raw, at, 8)?.try_into().map_err(|_| "bad int")?;
                Wire::Int(i64::from_le_bytes(b))
            }
            2 => {
                let b: [u8; 8] = take(raw, at, 8)?.try_into().map_err(|_| "bad float")?;
                Wire::Float(f64::from_bits(u64::from_le_bytes(b)))
            }
            3 => Wire::Bool(take(raw, at, 1)?[0] != 0),
            4 => Wire::Str(text(raw, at)?),
            5 => {
                let n = u32_of(raw, at)?;
                Wire::Bytes(take(raw, at, n)?.to_vec())
            }
            6 => Wire::List(many(raw, at)?),
            7 => Wire::Tuple(many(raw, at)?),
            8 => Wire::Variant {
                ty: text(raw, at)?,
                name: text(raw, at)?,
                fields: many(raw, at)?,
            },
            other => return Err(format!("unknown value tag {other} in the journal")),
        })
    }

    /// `Ok(v)` / `Err(v)` / `Some(v)` / `None`, built the way every engine does.
    pub fn ok(v: Wire) -> Wire {
        Wire::Variant {
            ty: "!".into(),
            name: "Ok".into(),
            fields: vec![v],
        }
    }
    pub fn err(v: Wire) -> Wire {
        Wire::Variant {
            ty: "!".into(),
            name: "Err".into(),
            fields: vec![v],
        }
    }
    pub fn some(v: Wire) -> Wire {
        Wire::Variant {
            ty: "Option".into(),
            name: "Some".into(),
            fields: vec![v],
        }
    }
    pub fn none() -> Wire {
        Wire::Variant {
            ty: "Option".into(),
            name: "None".into(),
            fields: vec![],
        }
    }
    /// The one error type every `io` builtin fails with.
    pub fn io_error(kind: &str, message: &str) -> Wire {
        Wire::Variant {
            ty: "IoError".into(),
            name: kind.into(),
            fields: vec![Wire::Str(message.to_string())],
        }
    }
}

impl Wire {
    /// One line of it, for a person reading a recording.
    ///
    /// Long payloads are cut and say how much was cut, because the whole point
    /// of looking at a journal is usually to see *which* call went wrong, and a
    /// recorded file of half a megabyte would bury the other twenty.
    pub fn render(&self, budget: usize) -> String {
        match self {
            Wire::Unit => "()".to_string(),
            Wire::Int(v) => v.to_string(),
            Wire::Float(v) => format!("{v}"),
            Wire::Bool(v) => v.to_string(),
            Wire::Str(s) => quoted(s, budget),
            Wire::Bytes(b) => {
                let shown = b.len().min(budget / 3);
                let hex: Vec<String> = b[..shown].iter().map(|x| format!("{x:02x}")).collect();
                let tail = if shown < b.len() {
                    format!(" … {} more", b.len() - shown)
                } else {
                    String::new()
                };
                format!("{} byte(s) {}{}", b.len(), hex.join(" "), tail)
            }
            Wire::List(items) => bracketed(items, budget, '[', ']'),
            Wire::Tuple(items) => bracketed(items, budget, '(', ')'),
            Wire::Variant { name, fields, .. } if fields.is_empty() => name.clone(),
            Wire::Variant { name, fields, .. } => {
                let inner: Vec<String> = fields
                    .iter()
                    .map(|f| f.render(budget.saturating_sub(name.len() + 2)))
                    .collect();
                format!("{name}({})", inner.join(", "))
            }
        }
    }
}

fn quoted(s: &str, budget: usize) -> String {
    let mut out = String::from("\"");
    for (n, c) in s.chars().enumerate() {
        if n >= budget {
            let left = s.chars().count() - n;
            return format!("{out}\" … {left} more char(s)");
        }
        match c {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '"' => out.push_str("\\\""),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn bracketed(items: &[Wire], budget: usize, open: char, close: char) -> String {
    // Two elements and a count, rather than a hundred names on one line: a
    // `list_dir` of a large directory is exactly what a recording holds.
    let shown = items.len().min(3);
    let mut parts: Vec<String> = items[..shown]
        .iter()
        .map(|i| i.render(budget / 3))
        .collect();
    if shown < items.len() {
        parts.push(format!("… {} more", items.len() - shown));
    }
    format!("{open}{}{close}", parts.join(", "))
}

enum Mode {
    Off,
    Recording(grasm_sys::fs::File),
    Replaying {
        records: Vec<(String, Wire)>,
        at: usize,
    },
}

thread_local! {
    static MODE: RefCell<Mode> = RefCell::new(open());
}

/// `GRASM_RECORD=path` writes a journal; `GRASM_REPLAY=path` reads one.
///
/// Both set means recording, because a replay that also rewrote the journal it
/// was reading would destroy the evidence it exists to preserve.
fn open() -> Mode {
    if let Some(path) = grasm_sys::sys::env_var("GRASM_RECORD") {
        return match grasm_sys::fs::File::create(&path) {
            Ok(mut f) => {
                let _ = f.write_all(MAGIC);
                Mode::Recording(f)
            }
            Err(e) => {
                eprint_line!("grasm: cannot record to {}: {e}", path);
                grasm_sys::sys::exit(7);
            }
        };
    }
    if let Some(path) = grasm_sys::sys::env_var("GRASM_REPLAY") {
        return match read_journal(&path) {
            Ok(records) => Mode::Replaying { records, at: 0 },
            Err(e) => {
                eprint_line!("grasm: cannot replay {}: {e}", path);
                grasm_sys::sys::exit(7);
            }
        };
    }
    Mode::Off
}

/// Builds a journal from records, in the format [`read`] reads.
///
/// A journal does not have to have been recorded. Written by hand it is a
/// *script of what the world will say* — `read_file` fails, the disk is full,
/// the socket answers this exact byte sequence — and the program has no way to
/// tell the difference, because there is none: the effect system says those are
/// the only places the world reaches it.
///
/// That makes the failure paths reachable without a broken machine to reach
/// them from, which is otherwise the hardest kind of test to write.
pub fn framed(records: &[(String, Wire)]) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    for (name, value) in records {
        let payload = value.encode();
        out.extend((name.len() as u32).to_le_bytes());
        out.extend(name.as_bytes());
        out.extend((payload.len() as u32).to_le_bytes());
        out.extend(payload);
    }
    out
}

/// Reads a journal from disk. Public so a person can be shown what a recording
/// holds — a file of framed bytes is evidence nobody can examine.
pub fn read(path: &str) -> Result<Vec<(String, Wire)>, String> {
    read_journal(path)
}

fn read_journal(path: &str) -> Result<Vec<(String, Wire)>, String> {
    let mut raw = Vec::new();
    grasm_sys::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .read_to_end(&mut raw)
        .map_err(|e| e.to_string())?;
    parse(&raw)
}

fn parse(raw: &[u8]) -> Result<Vec<(String, Wire)>, String> {
    if raw.len() < MAGIC.len() || &raw[..MAGIC.len()] != MAGIC {
        return Err("not a Grasm journal".to_string());
    }
    let mut at = MAGIC.len();
    let mut out = Vec::new();
    let take = |raw: &[u8], at: &mut usize, n: usize| -> Result<Vec<u8>, String> {
        if *at + n > raw.len() {
            return Err("journal ends mid-record".to_string());
        }
        let v = raw[*at..*at + n].to_vec();
        *at += n;
        Ok(v)
    };
    while at < raw.len() {
        let n = u32::from_le_bytes(
            take(raw, &mut at, 4)?
                .try_into()
                .map_err(|_| "bad length".to_string())?,
        ) as usize;
        let name = String::from_utf8_lossy(&take(raw, &mut at, n)?).into_owned();
        let n = u32::from_le_bytes(
            take(raw, &mut at, 4)?
                .try_into()
                .map_err(|_| "bad length".to_string())?,
        ) as usize;
        out.push((name, Wire::decode(&take(raw, &mut at, n)?)?));
    }
    Ok(out)
}

/// Replays a journal handed over as bytes rather than named by a path.
///
/// wasm32 has no filesystem to read a journal from, which is the only reason
/// this exists — and it is the reason worth having, because a browser tab is
/// somewhere a person can be handed a failure and step through it without
/// installing anything. The bytes are the same format a recording writes.
///
/// Refuses if a mode is already chosen: a run that silently swapped one journal
/// for another would be neither replay.
pub fn install(bytes: &[u8]) -> Result<(), String> {
    let records = parse(bytes)?;
    MODE.with(|m| {
        let mut m = m.borrow_mut();
        if !matches!(&*m, Mode::Off) {
            return Err("this run already has a journal".to_string());
        }
        *m = Mode::Replaying { records, at: 0 };
        Ok(())
    })
}

/// Whether this run is replaying. Cheap: the common case is one load.
pub fn replaying() -> bool {
    MODE.with(|m| matches!(&*m.borrow(), Mode::Replaying { .. }))
}

pub fn recording() -> bool {
    MODE.with(|m| matches!(&*m.borrow(), Mode::Recording(_)))
}

/// Whether the journal is doing anything at all. A run with neither variable
/// set pays one thread-local read per effectful call and nothing else.
pub fn active() -> bool {
    MODE.with(|m| !matches!(&*m.borrow(), Mode::Off))
}

/// Writes one record. Called after an effect actually happened.
pub fn put(name: &str, value: &Wire) {
    MODE.with(|m| {
        if let Mode::Recording(f) = &mut *m.borrow_mut() {
            let payload = value.encode();
            let mut buf = Vec::with_capacity(8 + name.len() + payload.len());
            buf.extend((name.len() as u32).to_le_bytes());
            buf.extend(name.as_bytes());
            buf.extend((payload.len() as u32).to_le_bytes());
            buf.extend(payload);
            let _ = f.write_all(&buf);
        }
    });
}

/// The next recorded answer for `name`, or `None` when not replaying.
///
/// Stops the run if the journal has run out or names a different call: the
/// program has diverged from the recording, and continuing would produce
/// something that looks like a replay and is not.
pub fn gate(name: &str) -> Option<Wire> {
    MODE.with(|m| {
        let mut m = m.borrow_mut();
        let Mode::Replaying { records, at } = &mut *m else {
            return None;
        };
        if *at >= records.len() {
            eprint_line!(
                "grasm: the journal ran out at call {} (`{name}`) — the program \
                 reached outside itself more often than the recording holds, so \
                 it has diverged from the run being replayed",
                *at + 1
            );
            grasm_sys::sys::exit(7);
        }
        let (recorded, value) = &records[*at];
        if recorded != name {
            eprint_line!(
                "grasm: at call {} the recording says `{recorded}` and this run \
                 asked for `{name}` — the program has taken a different path \
                 from the one recorded, so this is not a replay of it",
                *at + 1
            );
            grasm_sys::sys::exit(7);
        }
        *at += 1;
        Some(value.clone())
    })
}

/// Says how much of the journal a replay consumed, if it is short.
///
/// A replay that ends with records left over did *not* reproduce the recorded
/// run — it stopped early — and saying nothing would let that pass for success.
pub fn check_exhausted() {
    MODE.with(|m| {
        if let Mode::Replaying { records, at } = &*m.borrow() {
            if *at < records.len() {
                let left = records.len() - *at;
                eprint_line!(
                    "grasm: the replay ended with {left} record(s) unread — the \
                     program stopped earlier than the run being replayed"
                );
            }
        }
    });
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::Wire;

    /// Every shape survives a round trip, including the awkward ones: an empty
    /// list is not a missing list, and a byte string is not text.
    #[test]
    fn every_shape_survives_the_round_trip() {
        let cases = vec![
            Wire::Unit,
            Wire::Int(i64::MIN),
            Wire::Int(0),
            Wire::Float(-0.0),
            Wire::Bool(false),
            Wire::Str(String::new()),
            Wire::Str("héllo\0wörld".into()),
            Wire::Bytes(vec![]),
            Wire::Bytes(vec![0, 255, 128]),
            Wire::List(vec![]),
            Wire::List(vec![Wire::Str("a".into()), Wire::Int(2)]),
            Wire::Tuple(vec![Wire::Int(1), Wire::Str("out".into())]),
            Wire::none(),
            Wire::some(Wire::Bytes(vec![7])),
            Wire::ok(Wire::List(vec![Wire::Str("x".into())])),
            Wire::err(Wire::io_error("NotFound", "/nope")),
        ];
        for c in cases {
            let round = Wire::decode(&c.encode()).expect("decodes");
            assert_eq!(round, c, "{c:?} did not survive the journal");
        }
    }

    /// A journal written by hand reads back as what was written.
    ///
    /// This is what makes fault injection work: a `read_file` that fails, with
    /// no broken disk anywhere.
    #[test]
    fn a_hand_written_journal_reads_back() {
        let records = vec![
            ("now_millis".to_string(), Wire::Int(1_700_000_000_000)),
            (
                "read_file".to_string(),
                Wire::err(Wire::io_error("IoFailed", "disk quota exceeded")),
            ),
        ];
        let raw = super::framed(&records);
        let dir = std::env::temp_dir().join(format!("grasm_journal_{}", std::process::id()));
        grasm_sys::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("j");
        grasm_sys::fs::write(&path, &raw).unwrap();
        let back = super::read(path.as_os_str()).expect("reads");
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].0, "now_millis");
        assert_eq!(
            back[1].1.render(usize::MAX),
            "Err(IoFailed(\"disk quota exceeded\"))"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A truncated journal is refused rather than read as a shorter value.
    #[test]
    fn a_truncated_value_is_refused() {
        let raw = Wire::Str("abcdef".into()).encode();
        for cut in 1..raw.len() {
            assert!(
                Wire::decode(&raw[..cut]).is_err(),
                "accepted a {cut}-byte prefix"
            );
        }
    }
}
