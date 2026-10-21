//! Raw memory and atomics ([T-ADDR], [T-MEM-UNSAFE], [L-LIB-MEM],
//! [L-LIB-ATOMIC]): what each operation is, decided once by the checker.
//!
//! The checker writes a [`MemOp`] for every raw-memory call into
//! `Analysis::mem_ops`, keyed by the call's span, and every engine reads that
//! instead of looking at the method's name. Natively an address is held as an
//! `i64`, so going by name would turn a user's UFCS `fn offset(n: i64, ..)`
//! into pointer arithmetic; and two arguments here are not values at all -- a
//! layout's type and an atomic's ordering -- so an engine that evaluated them
//! would be looking up a variable called `u16` or `Acquire`.

use crate::types::{IntKind, Type};

/// A *layout type*: the only types a load or a store moves. Each is as large
/// as it is aligned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemTy {
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F64,
    Addr,
}

impl MemTy {
    pub const ALL: [MemTy; 10] = [
        MemTy::I8,
        MemTy::I16,
        MemTy::I32,
        MemTy::I64,
        MemTy::U8,
        MemTy::U16,
        MemTy::U32,
        MemTy::U64,
        MemTy::F64,
        MemTy::Addr,
    ];

    pub fn name(self) -> &'static str {
        match self {
            MemTy::I8 => "i8",
            MemTy::I16 => "i16",
            MemTy::I32 => "i32",
            MemTy::I64 => "i64",
            MemTy::U8 => "u8",
            MemTy::U16 => "u16",
            MemTy::U32 => "u32",
            MemTy::U64 => "u64",
            MemTy::F64 => "f64",
            MemTy::Addr => "addr",
        }
    }

    pub fn from_name(s: &str) -> Option<MemTy> {
        MemTy::ALL.into_iter().find(|t| t.name() == s)
    }

    /// The layout type a checked type is, if it has a layout.
    pub fn of(t: &Type) -> Option<MemTy> {
        Some(match t {
            Type::I64 => MemTy::I64,
            Type::Int(k) => MemTy::from_name(k.name())?,
            Type::F64 => MemTy::F64,
            Type::Addr => MemTy::Addr,
            _ => return None,
        })
    }

    /// Bytes, which is also the alignment.
    pub fn size(self) -> i64 {
        match self {
            MemTy::I8 | MemTy::U8 => 1,
            MemTy::I16 | MemTy::U16 => 2,
            MemTy::I32 | MemTy::U32 => 4,
            _ => 8,
        }
    }

    /// An integer type: what an atomic operates on.
    pub fn is_int(self) -> bool {
        !matches!(self, MemTy::F64 | MemTy::Addr)
    }

    /// Whether a narrow value read back is sign-extended.
    pub fn signed(self) -> bool {
        matches!(self, MemTy::I8 | MemTy::I16 | MemTy::I32 | MemTy::I64)
    }

    /// The checker's type for a value of this layout.
    pub fn ty(self) -> Type {
        match self {
            MemTy::I64 => Type::I64,
            MemTy::F64 => Type::F64,
            MemTy::Addr => Type::Addr,
            t => Type::Int(IntKind::from_name(t.name()).expect("a narrow integer")),
        }
    }
}

/// A memory ordering, written at the call ([L-LIB-ATOMIC]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    Relaxed,
    Acquire,
    Release,
    AcqRel,
    SeqCst,
}

impl Order {
    pub const ALL: [Order; 5] = [
        Order::Relaxed,
        Order::Acquire,
        Order::Release,
        Order::AcqRel,
        Order::SeqCst,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Order::Relaxed => "Relaxed",
            Order::Acquire => "Acquire",
            Order::Release => "Release",
            Order::AcqRel => "AcqRel",
            Order::SeqCst => "SeqCst",
        }
    }

    pub fn from_name(s: &str) -> Option<Order> {
        Order::ALL.into_iter().find(|o| o.name() == s)
    }
}

/// What an atomic does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Atomic {
    Load,
    Store,
    /// Compare-exchange: stores the new value if the expected one was there,
    /// and answers what was there.
    Cas,
    /// Fetch-add: adds, wrapping at the width, and answers what was there.
    Add,
}

impl Atomic {
    pub fn name(self) -> &'static str {
        match self {
            Atomic::Load => "load",
            Atomic::Store => "store",
            Atomic::Cas => "cas",
            Atomic::Add => "add",
        }
    }

    /// The orderings this operation can have, as C++ and Rust allow them: a
    /// load does not release and a store does not acquire.
    pub fn allows(self, o: Order) -> bool {
        match self {
            Atomic::Load => matches!(o, Order::Relaxed | Order::Acquire | Order::SeqCst),
            Atomic::Store => matches!(o, Order::Relaxed | Order::Release | Order::SeqCst),
            Atomic::Cas | Atomic::Add => true,
        }
    }

    /// How many value arguments come before the ordering.
    pub fn values(self) -> usize {
        match self {
            Atomic::Load => 0,
            Atomic::Store | Atomic::Add => 1,
            Atomic::Cas => 2,
        }
    }
}

/// One raw-memory operation, resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemOp {
    /// `mem_alloc(size, align)`.
    Alloc,
    /// `mem_free(a)`.
    Free,
    /// `mem_addr(n)`: the address whose integer is `n`.
    FromInt,
    /// `mem_size_of(T)`; its argument is a type and is not evaluated.
    Size(MemTy),
    /// `mem_align_of(T)`, likewise.
    Align(MemTy),
    /// `a.offset(n)`.
    Offset,
    /// `a.diff(b)`.
    Diff,
    /// `a.as_i64()`.
    ToInt,
    Load(MemTy),
    Store(MemTy),
    /// The ordering argument is a name, not a value, and is not evaluated.
    Atomic(Atomic, MemTy, Order),
}

/// What a method name on an `addr` asks for, before its ordering (if any) is
/// read: `None` for a name that is not a raw-memory method.
pub fn method(name: &str) -> Option<MethodKind> {
    match name {
        "offset" => return Some(MethodKind::Plain(MemOp::Offset)),
        "diff" => return Some(MethodKind::Plain(MemOp::Diff)),
        "as_i64" => return Some(MethodKind::Plain(MemOp::ToInt)),
        _ => {}
    }
    if let Some(rest) = name.strip_prefix("atomic_") {
        let (op, t) = rest.split_once('_')?;
        let op = match op {
            "load" => Atomic::Load,
            "store" => Atomic::Store,
            "cas" => Atomic::Cas,
            "add" => Atomic::Add,
            _ => return None,
        };
        let t = MemTy::from_name(t).filter(|t| t.is_int())?;
        return Some(MethodKind::Atomic(op, t));
    }
    if let Some(t) = name.strip_prefix("load_").and_then(MemTy::from_name) {
        return Some(MethodKind::Plain(MemOp::Load(t)));
    }
    if let Some(t) = name.strip_prefix("store_").and_then(MemTy::from_name) {
        return Some(MethodKind::Plain(MemOp::Store(t)));
    }
    None
}

/// A raw-memory method, by its name alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MethodKind {
    Plain(MemOp),
    /// The ordering comes from the call's last argument.
    Atomic(Atomic, MemTy),
}

/// The built-in free functions of raw memory.
pub const FREE_FNS: &[&str] = &[
    "mem_alloc",
    "mem_free",
    "mem_addr",
    "mem_size_of",
    "mem_align_of",
];

/// The largest alignment `mem_alloc` takes: a page, which is the most any
/// allocator the runtime will be built from needs.
pub const MAX_ALIGN: i64 = 4096;
