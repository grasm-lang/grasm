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
//! `GRASMJRNL1` then one record each:
//!
//! ```text
//! u32 name_len | name | u8 outcome | u32 payload_len | payload
//! ```
//!
//! `outcome` is 0 for a value and 1 for a failure; the payload is the bytes of
//! the answer, or of the error's message. An `i64` answer is eight
//! little-endian bytes. Length-prefixed rather than a text format because a
//! recorded value is arbitrary bytes — a `read_bytes` of a JPEG is exactly the
//! case this exists to reproduce, and it is the one a text format would
//! corrupt.

use std::cell::RefCell;
use std::io::{Read, Write};

const MAGIC: &[u8; 10] = b"GRASMJRNL1";

/// What a recorded call answered.
pub enum Outcome {
    /// A value, as bytes.
    Value(Vec<u8>),
    /// A failure, and the message it carried.
    Failed(String),
}

enum Mode {
    Off,
    Recording(std::fs::File),
    Replaying { records: Vec<(String, Outcome)>, at: usize },
}

thread_local! {
    static MODE: RefCell<Mode> = RefCell::new(open());
}

/// `GRASM_RECORD=path` writes a journal; `GRASM_REPLAY=path` reads one.
///
/// Both set means recording, because a replay that also rewrote the journal it
/// was reading would destroy the evidence it exists to preserve.
fn open() -> Mode {
    if let Some(path) = std::env::var_os("GRASM_RECORD") {
        return match std::fs::File::create(&path) {
            Ok(mut f) => {
                let _ = f.write_all(MAGIC);
                Mode::Recording(f)
            }
            Err(e) => {
                eprintln!("grasm: cannot record to {}: {e}", path.to_string_lossy());
                std::process::exit(7);
            }
        };
    }
    if let Some(path) = std::env::var_os("GRASM_REPLAY") {
        return match read_journal(&path) {
            Ok(records) => Mode::Replaying { records, at: 0 },
            Err(e) => {
                eprintln!("grasm: cannot replay {}: {e}", path.to_string_lossy());
                std::process::exit(7);
            }
        };
    }
    Mode::Off
}

fn read_journal(path: &std::ffi::OsStr) -> Result<Vec<(String, Outcome)>, String> {
    let mut raw = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .read_to_end(&mut raw)
        .map_err(|e| e.to_string())?;
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
            take(&raw, &mut at, 4)?
                .try_into()
                .map_err(|_| "bad length".to_string())?,
        ) as usize;
        let name = String::from_utf8_lossy(&take(&raw, &mut at, n)?).into_owned();
        let kind = take(&raw, &mut at, 1)?[0];
        let n = u32::from_le_bytes(
            take(&raw, &mut at, 4)?
                .try_into()
                .map_err(|_| "bad length".to_string())?,
        ) as usize;
        let payload = take(&raw, &mut at, n)?;
        out.push((
            name,
            if kind == 0 {
                Outcome::Value(payload)
            } else {
                Outcome::Failed(String::from_utf8_lossy(&payload).into_owned())
            },
        ));
    }
    Ok(out)
}

/// Whether this run is replaying. Cheap: the common case is one load.
pub fn replaying() -> bool {
    MODE.with(|m| matches!(&*m.borrow(), Mode::Replaying { .. }))
}

pub fn recording() -> bool {
    MODE.with(|m| matches!(&*m.borrow(), Mode::Recording(_)))
}

/// Writes one record. Called after an effect actually happened.
pub fn record(name: &str, outcome: &Outcome) {
    MODE.with(|m| {
        if let Mode::Recording(f) = &mut *m.borrow_mut() {
            let (kind, payload): (u8, &[u8]) = match outcome {
                Outcome::Value(v) => (0, v),
                Outcome::Failed(s) => (1, s.as_bytes()),
            };
            let mut buf = Vec::with_capacity(9 + name.len() + payload.len());
            buf.extend((name.len() as u32).to_le_bytes());
            buf.extend(name.as_bytes());
            buf.push(kind);
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
pub fn next(name: &str) -> Option<Outcome> {
    MODE.with(|m| {
        let mut m = m.borrow_mut();
        let Mode::Replaying { records, at } = &mut *m else {
            return None;
        };
        if *at >= records.len() {
            eprintln!(
                "grasm: the journal ran out at call {} (`{name}`) — the program \
                 read more from outside itself than the recording holds, so it \
                 has diverged from the run being replayed",
                *at + 1
            );
            std::process::exit(7);
        }
        let (recorded, outcome) = &records[*at];
        if recorded != name {
            eprintln!(
                "grasm: at call {} the recording says `{recorded}` and this run \
                 asked for `{name}` — the program has taken a different path \
                 from the one recorded, so this is not a replay of it",
                *at + 1
            );
            std::process::exit(7);
        }
        *at += 1;
        Some(match outcome {
            Outcome::Value(v) => Outcome::Value(v.clone()),
            Outcome::Failed(s) => Outcome::Failed(s.clone()),
        })
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
                eprintln!(
                    "grasm: the replay ended with {left} record(s) unread — the \
                     program stopped earlier than the run being replayed"
                );
            }
        }
    });
}

pub fn i64_bytes(v: i64) -> Vec<u8> {
    v.to_le_bytes().to_vec()
}

pub fn bytes_i64(b: &[u8]) -> i64 {
    let mut a = [0u8; 8];
    let n = b.len().min(8);
    a[..n].copy_from_slice(&b[..n]);
    i64::from_le_bytes(a)
}
