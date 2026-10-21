//! Raw memory under `grasm run` ([L-LIB-MEM], [L-LIB-ATOMIC],
//! [E-MEM-UNCHECKED]).
//!
//! This engine has no real addresses to give out, so it keeps raw memory in a
//! sandbox of its own: one byte array per allocation, at addresses it makes up
//! and **never reuses**. Every access is checked against the allocation it
//! falls in, which is how `grasm run` catches what a compiled build cannot: a
//! null, wild, freed, out-of-bounds or misaligned access, and a `mem_free` of
//! anything but a live allocation's start. Because nothing is reused, a stale
//! address finds its own allocation's tombstone, or nothing -- never somebody
//! else's bytes.
//!
//! An address is the engine's ordinary integer value, so a list, a field and
//! printing need nothing new. One thread runs at a time ([`crate::tasks`]), so
//! an atomic is a plain access here, and every ordering is trivially kept.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use grasm_sema::mem::{Atomic, CSig, MemOp, MemTy, MAX_ALIGN};

use crate::value::Value;

/// Where the first allocation goes: far enough from zero that no small
/// integer is ever a valid address.
const BASE: u64 = 0x1_0000;
/// Bytes left empty after each allocation, so that running off the end lands
/// in a gap that names the allocation, and never in the next one.
const GAP: u64 = 64;
/// Freed allocations remembered by name. Past this many, the oldest is
/// forgotten -- and an access through it is still refused, as through an
/// address no allocation holds, because its addresses are never handed out
/// again.
const TOMBSTONES: usize = 1 << 16;
/// The largest allocation either engine attempts.
pub const MAX_SIZE: i64 = 1 << 40;

struct Block {
    len: u64,
    /// `None` once freed.
    bytes: Option<Vec<u8>>,
}

#[derive(Default)]
pub struct Heap {
    blocks: BTreeMap<u64, Block>,
    /// The next free address, past every allocation and its gap.
    next: u64,
    /// Freed allocations, oldest first, so tombstones can be forgotten.
    freed: VecDeque<u64>,
    /// The address given to each Grasm function `mem_fn_addr` was asked for
    /// ([T-FN-ADDR]), reserved like an allocation and never reused: what it
    /// names, and the C signature it has.
    funcs: BTreeMap<u64, (String, Option<CSig>)>,
    by_name: BTreeMap<String, u64>,
    /// Addresses C handed over -- an `extern` result, or a C function's own
    /// address -- which `call_c` may call and C may be given back.
    from_c: BTreeSet<u64>,
    /// Each `thread_local` cell, by name, and where it was put ([T-TLS]).
    /// One thread runs here, so there is one of each.
    cells: BTreeMap<String, u64>,
    cell_at: BTreeSet<u64>,
}

impl Heap {
    /// Runs one operation. `at` is the receiver's address, for a method; `vs`
    /// are the value arguments, evaluated.
    pub fn run(&mut self, op: MemOp, at: u64, vs: &[Value]) -> Result<Value, String> {
        match op {
            MemOp::Alloc => self.alloc(int(&vs[0]), int(&vs[1])),
            MemOp::Free => self.free(addr(&vs[0])).map(|_| Value::Unit),
            MemOp::FromInt => Ok(Value::Int(int(&vs[0]))),
            MemOp::Size(t) | MemOp::Align(t) => Ok(Value::Int(t.size())),
            MemOp::Offset => Ok(Value::Int(at.wrapping_add(int(&vs[0]) as u64) as i64)),
            MemOp::Diff => Ok(Value::Int(at.wrapping_sub(addr(&vs[0])) as i64)),
            MemOp::ToInt => Ok(Value::Int(at as i64)),
            MemOp::Load(t) => self.load(t, at, &format!("load_{}", t.name())),
            MemOp::Store(t) => {
                let what = format!("store_{}", t.name());
                self.store(t, at, &vs[0], &what).map(|_| Value::Unit)
            }
            MemOp::FnAddr | MemOp::CallC | MemOp::Tls(_) => {
                unreachable!("run by the interpreter itself")
            }
            MemOp::Atomic(op, t, _) => {
                let what = format!("atomic_{}_{}", op.name(), t.name());
                match op {
                    Atomic::Load => self.load(t, at, &what),
                    Atomic::Store => self.store(t, at, &vs[0], &what).map(|_| Value::Unit),
                    Atomic::Cas => {
                        let old = self.load(t, at, &what)?;
                        // At the width: a sign-extended `i8` and the literal
                        // it was compared with agree in their low byte.
                        if bits(&old) & mask(t) == bits(&vs[0]) & mask(t) {
                            self.store(t, at, &vs[1], &what)?;
                        }
                        Ok(old)
                    }
                    Atomic::Add => {
                        let old = self.load(t, at, &what)?;
                        let sum = Value::U64(bits(&old).wrapping_add(bits(&vs[0])));
                        self.store(t, at, &sum, &what)?;
                        Ok(old)
                    }
                }
            }
        }
    }

    fn alloc(&mut self, size: i64, align: i64) -> Result<Value, String> {
        check_alloc(size, align)?;
        // A size this host cannot even count -- past 4 GiB on wasm32 -- is
        // one it cannot meet.
        let oom = || "mem_alloc: out of memory".to_string();
        let n = usize::try_from(size).map_err(|_| oom())?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(n).map_err(|_| oom())?;
        bytes.resize(n, 0);
        let step = (align as u64).max(16);
        let at = self.next.max(BASE).div_ceil(step) * step;
        self.next = at + size as u64 + GAP;
        self.blocks.insert(
            at,
            Block {
                len: size as u64,
                bytes: Some(bytes),
            },
        );
        Ok(Value::Int(at as i64))
    }

    /// A Grasm function's address: the same one every time it is asked for,
    /// from the sandbox's own range, so it can never be an allocation's.
    pub fn fn_addr(&mut self, name: &str, sig: Option<CSig>) -> u64 {
        if let Some(at) = self.by_name.get(name) {
            return *at;
        }
        let at = self.next.max(BASE).div_ceil(16) * 16;
        self.next = at + 16 + GAP;
        self.funcs.insert(at, (name.to_string(), sig));
        self.by_name.insert(name.to_string(), at);
        at
    }

    /// The function at `at`, if `mem_fn_addr` gave out that address.
    pub fn func_at(&self, at: u64) -> Option<&(String, Option<CSig>)> {
        self.funcs.get(&at)
    }

    /// The cell `name`: an allocation of its type's size and alignment, zero
    /// until written, made the first time it is asked for and never freed.
    pub fn tls_cell(&mut self, name: &str, t: MemTy) -> Result<u64, String> {
        if let Some(at) = self.cells.get(name) {
            return Ok(*at);
        }
        let at = match self.alloc(t.size(), t.size())? {
            Value::Int(i) => i as u64,
            _ => unreachable!("an allocation is an address"),
        };
        self.cells.insert(name.to_string(), at);
        self.cell_at.insert(at);
        Ok(at)
    }

    /// Remembers an address C handed over.
    pub fn note_from_c(&mut self, at: u64) {
        if at != 0 {
            self.from_c.insert(at);
        }
    }

    pub fn is_from_c(&self, at: u64) -> bool {
        self.from_c.contains(&at)
    }

    /// Whether the sandbox made this address, for memory or for a function:
    /// one C must not be given, since it would read made-up numbers.
    pub fn made(&self, at: u64) -> bool {
        at >= BASE && at < self.next && !self.from_c.contains(&at)
    }

    fn free(&mut self, at: u64) -> Result<(), String> {
        if self.funcs.contains_key(&at) {
            return Err("mem_free of the address of a function".into());
        }
        if self.cell_at.contains(&at) {
            return Err("mem_free of a thread-local cell".into());
        }
        let inside = self.find(at).is_some();
        match self.blocks.get_mut(&at) {
            Some(Block {
                bytes: b @ Some(_), ..
            }) => {
                *b = None;
                self.freed.push_back(at);
                if self.freed.len() > TOMBSTONES {
                    let old = self.freed.pop_front().expect("not empty");
                    self.blocks.remove(&old);
                }
                Ok(())
            }
            Some(_) => Err("mem_free of an allocation already freed".into()),
            None if at == 0 => Err("mem_free of the null address".into()),
            None if inside => {
                Err("mem_free of an address inside an allocation, not its start".into())
            }
            None => Err("mem_free of an address no allocation holds".into()),
        }
    }

    /// The allocation `at` belongs to: the nearest at or below it, if `at` is
    /// inside it or in the gap after it.
    fn find(&self, at: u64) -> Option<(u64, &Block)> {
        let (&base, b) = self.blocks.range(..=at).next_back()?;
        (at - base < b.len + GAP).then_some((base, b))
    }

    /// The bytes `n` long at `at`, checked: live, in bounds, aligned.
    fn span(&mut self, at: u64, n: u64, what: &str) -> Result<&mut [u8], String> {
        if at == 0 {
            return Err(format!("{what} through the null address"));
        }
        if self.funcs.contains_key(&at) {
            return Err(format!("{what} through the address of a function"));
        }
        let Some((base, b)) = self.find(at) else {
            return Err(format!("{what} through an address no allocation holds"));
        };
        let len = b.len;
        if b.bytes.is_none() {
            return Err(format!("{what} after its allocation was freed"));
        }
        let off = at - base;
        if off + n > len {
            return Err(format!(
                "{what} of {n} byte(s) at offset {off} of a {len}-byte allocation"
            ));
        }
        if !at.is_multiple_of(n) {
            return Err(format!("{what} at an address not aligned to {n}"));
        }
        let bytes = self
            .blocks
            .get_mut(&base)
            .and_then(|b| b.bytes.as_mut())
            .expect("checked live above");
        Ok(&mut bytes[off as usize..(off + n) as usize])
    }

    fn load(&mut self, t: MemTy, at: u64, what: &str) -> Result<Value, String> {
        let n = t.size() as usize;
        let src = self.span(at, n as u64, what)?;
        let mut le = [0u8; 8];
        le[..n].copy_from_slice(src);
        let raw = u64::from_le_bytes(le);
        Ok(match t {
            MemTy::I8 => Value::Int(raw as u8 as i8 as i64),
            MemTy::I16 => Value::Int(raw as u16 as i16 as i64),
            MemTy::I32 => Value::Int(raw as u32 as i32 as i64),
            MemTy::U64 => Value::U64(raw),
            MemTy::F64 => Value::Float(f64::from_bits(raw)),
            _ => Value::Int(raw as i64),
        })
    }

    fn store(&mut self, t: MemTy, at: u64, v: &Value, what: &str) -> Result<(), String> {
        let n = t.size() as usize;
        let dst = self.span(at, n as u64, what)?;
        dst.copy_from_slice(&bits(v).to_le_bytes()[..n]);
        Ok(())
    }
}

/// The panics both engines share for a request `mem_alloc` cannot take.
pub fn check_alloc(size: i64, align: i64) -> Result<(), String> {
    if size < 0 {
        return Err(format!("mem_alloc: negative size {size}"));
    }
    if !(1..=MAX_ALIGN).contains(&align) || align & (align - 1) != 0 {
        return Err(format!(
            "mem_alloc: alignment {align} is not a power of two from 1 to {MAX_ALIGN}"
        ));
    }
    if size > MAX_SIZE {
        return Err(format!(
            "mem_alloc: {size} bytes is more than an allocation can hold"
        ));
    }
    Ok(())
}

/// A value's bits, as a load or store moves them.
fn bits(v: &Value) -> u64 {
    match v {
        Value::Int(i) => *i as u64,
        Value::U64(u) => *u,
        Value::Float(f) => f.to_bits(),
        _ => 0,
    }
}

fn mask(t: MemTy) -> u64 {
    match t.size() {
        8 => u64::MAX,
        n => (1u64 << (8 * n)) - 1,
    }
}

fn int(v: &Value) -> i64 {
    bits(v) as i64
}

fn addr(v: &Value) -> u64 {
    bits(v)
}
