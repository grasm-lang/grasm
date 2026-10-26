#!/usr/bin/env python3
"""Thread-local cells, against a Python model of each thread's cells.

[T-TLS] says each thread has its own cell of a layout type, zero until
written, reached through `mem_tls`. What that means is which bits a store
leaves for a later load, at what width, and -- the part only threads can
show -- that one thread's writes never reach another's cell. Two engines
that share a mistake agree with each other, so every answer comes from the
model instead.

Four parts, each its own way of being wrong:

1. Random programs, in both engines: several cells of every layout type,
   stored, loaded, updated atomically and compared, through `mem_tls`
   directly and through an address kept in a struct field, a list and a
   function's parameter, against one value per cell. The native build leaks
   nothing.
2. Threads, natively only: threads C starts (`pthread_create` with a Grasm
   entry, [T-FN-ADDR]) each find every cell zero, fill their cells with
   values of their own while the others do the same, and find their own
   values back; one shared counter is added to atomically by all of them
   and must come to the exact total -- the first time a Grasm atomic runs
   under contention -- and the main thread's cells are untouched.
   `grasm run` refuses to hand C its first sandbox address, in its words.
3. Misuse, under `grasm run` only: freeing a cell, and a load or store wider
   than the cell, each a `test` whose panic the sandbox words exactly.
4. The checker: a cell of a type with no layout, a `pub` one, two of one
   name, `mem_tls` outside `unsafe`, of something that is not a cell, and
   in a row without `mem` -- each refused with its sentence.

    scripts/fuzz_tls.py [--programs N] [--seed S] [--keep]
"""

import argparse
import os
import random
import shutil
import struct
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM") or os.path.join(ROOT, "target", "release", "grasm")

import importlib.util as _ilu


def _lib(name):
    spec = _ilu.spec_from_file_location(
        name, os.path.join(os.path.dirname(os.path.abspath(__file__)), "lib", f"{name}.py"))
    mod = _ilu.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


leaked = _lib("leaks").leaked
build_native = _lib("inplace").build
# The Grasm backend (compiler/s3), held to the Cranelift build on each program.
S3Engine = _lib("s3engine").Engine
_lib("fresh").fresh_or_stop(GRASM)

INTS = {
    "i8": (8, True), "i16": (16, True), "i32": (32, True), "i64": (64, True),
    "u8": (8, False), "u16": (16, False), "u32": (32, False), "u64": (64, False),
}
LAYOUT = list(INTS) + ["f64", "addr"]
SIZE = {**{k: v[0] // 8 for k, v in INTS.items()}, "f64": 8, "addr": 8}
FLOATS = [0.5, -2.25, 1024.0, 1.5, 100.125, -0.75, 3.0, -65536.5, 7.0e5]
ORDERS = {"load": ["Relaxed", "Acquire", "SeqCst"], "add": ["Relaxed", "Acquire", "Release", "AcqRel", "SeqCst"]}


def wrap(t, v):
    bits, signed = INTS[t]
    v &= (1 << bits) - 1
    if signed and v >= 1 << (bits - 1):
        v -= 1 << bits
    return v


def rng_int(rng, t):
    bits, signed = INTS[t]
    lo, hi = (-(1 << (bits - 1)), (1 << (bits - 1)) - 1) if signed else (0, (1 << bits) - 1)
    edges = [lo, hi, 0, 1, lo + 1, hi - 1]
    v = rng.choice(edges) if rng.random() < 0.4 else rng.randint(lo, hi)
    return max(lo, min(hi, v))


def bits_of(x):
    return struct.unpack("<q", struct.pack("<d", x))[0]


def lit(t, v):
    if t == "f64":
        return repr(float(v))
    if t == "addr":
        return f"mem_addr({v})"
    return str(v)


def show_load(t, at):
    """A load of a `t` at `at`, as printed, which the model can word."""
    if t == "f64":
        return f"{at}.load_f64().to_bits()"
    if t == "addr":
        return f"{at}.load_addr().as_i64()"
    return f"{at}.load_{t}()"


def model_shown(t, v):
    return bits_of(v) if t == "f64" else v


def program(rng, idx):
    cells = [(f"C{k}", rng.choice(LAYOUT)) for k in range(rng.randint(2, 7))]
    lines = [f"thread_local {n}: {t}" for n, t in cells]
    lines += [
        "",
        "type Holder = { at: addr }",
        "",
        "fn via(p: addr) -> addr {",
        "    p",
        "}",
        "",
        "fn main() / {io, unsafe, mem} {",
        "    unsafe {",
        "        let held = [" + ", ".join(f"mem_tls({n})" for n, _ in cells) + "]",
        f"        let box0 = Holder{{ at: mem_tls({cells[0][0]}) }}",
    ]
    value = {n: (0.0 if t == "f64" else 0) for n, t in cells}
    want = []
    printed = [0]

    def say(expr, v):
        printed[0] += 1
        lines.append(f'        print("{idx}.{printed[0]} {{{expr}}}")')
        want.append(f"{idx}.{printed[0]} {v}")

    def at(k):
        n = cells[k][0]
        r = rng.random()
        if r < 0.4:
            return f"mem_tls({n})"
        if r < 0.6:
            return f"held[{k}]"
        if r < 0.8:
            return f"via(mem_tls({n}))"
        return "box0.at" if k == 0 else f"held[{k}]"

    # Every cell starts at zero.
    for k, (n, t) in enumerate(cells):
        say(show_load(t, at(k)), model_shown(t, value[n]))
    for _ in range(rng.randint(15, 45)):
        k = rng.randrange(len(cells))
        n, t = cells[k]
        r = rng.random()
        if r < 0.35:
            if t == "f64":
                v = rng.choice(FLOATS)
            elif t == "addr":
                v = rng.randrange(1 << 40)
            else:
                v = rng_int(rng, t)
            lines.append(f"        {at(k)}.store_{t}({lit(t, v)})")
            value[n] = v
        elif r < 0.6:
            say(show_load(t, at(k)), model_shown(t, value[n]))
        elif r < 0.8 and t in INTS:
            d = rng_int(rng, t)
            o = rng.choice(ORDERS["add"])
            say(f"{at(k)}.atomic_add_{t}({lit(t, d)}, {o})", value[n])
            value[n] = wrap(t, value[n] + d)
        elif r < 0.9 and t in INTS:
            o = rng.choice(ORDERS["load"])
            say(f"{at(k)}.atomic_load_{t}({o})", value[n])
        else:
            j = rng.randrange(len(cells))
            say(f"{at(k)} == {at(j)}", "true" if j == k else "false")
    lines += ["    }", "}"]
    return "\n".join(lines) + "\n", "\n".join(want) + "\n"


def threads_program(rng, idx):
    """Part 2. Returns the source and what the native build prints."""
    cells = [(f"T{k}", rng.choice([t for t in LAYOUT if t != "f64"])) for k in range(rng.randint(1, 4))]
    nthreads = rng.randint(2, 6)
    iters = rng.choice([2000, 10000, 40000])
    lines = [
        "extern fn pthread_create(thread: addr, attr: addr, start: addr, arg: addr) -> i32",
        "extern fn pthread_join(thread: addr, ret: addr) -> i32",
        "extern fn sched_yield() -> i32",
        "",
        "thread_local WHO: i64",
    ]
    lines += [f"thread_local {n}: {t}" for n, t in cells]
    # A thread's own value for cell k: its number, times something, at the
    # cell's width -- different in every thread.
    def own(t, k, tid):
        v = (tid + 1) * 1000003 + k * 7919
        return v if t == "addr" else wrap(t, v)

    lines += [
        "",
        "// arg: [shared counter, thread number, verdict]",
        "fn worker(arg: addr) -> addr / {io, unsafe, mem} {",
        "    unsafe {",
        "        let tid = arg.offset(8).load_i64()",
        "        let shared = arg.load_addr()",
        "        var ok = mem_tls(WHO).load_i64() == 0",
    ]
    for k, (n, t) in enumerate(cells):
        zero = "mem_addr(0)" if t == "addr" else "0"
        cmp = f"mem_tls({n}).load_{t}() == {zero}"
        lines.append(f"        ok = ok and {cmp}")
    lines.append("        mem_tls(WHO).store_i64(tid + 1)")
    for k, (n, t) in enumerate(cells):
        v = f"(tid + 1) * 1000003 + {k * 7919}"
        val = f"mem_addr({v})" if t == "addr" else (f"({v})" if t == "i64" else f"({v}).as_{t}()")
        lines.append(f"        mem_tls({n}).store_{t}({val})")
    lines += [
        f"        for i in 0..{iters} {{",
        "            let _old = shared.atomic_add_i64(1, AcqRel)",
        "            if i % 997 == 0 {",
        "                let _y = sched_yield()",
        "            }",
        "        }",
    ]
    for k, (n, t) in enumerate(cells):
        v = f"(tid + 1) * 1000003 + {k * 7919}"
        val = f"mem_addr({v})" if t == "addr" else (f"({v})" if t == "i64" else f"({v}).as_{t}()")
        lines.append(f"        ok = ok and mem_tls({n}).load_{t}() == {val}")
    lines += [
        "        ok = ok and mem_tls(WHO).load_i64() == tid + 1",
        "        arg.offset(16).store_i64(if ok { 1 } else { 0 })",
        "        mem_addr(0)",
        "    }",
        "}",
        "",
        "fn main() / {io, unsafe, mem} {",
        "    unsafe {",
        "        mem_tls(WHO).store_i64(-5)",
    ]
    main_vals = {}
    for k, (n, t) in enumerate(cells):
        v = rng.randrange(1, 1 << 30) if t == "addr" else rng_int(rng, t)
        main_vals[n] = v
        lines.append(f"        mem_tls({n}).store_{t}({lit(t, v)})")
    lines += [
        "        let shared = mem_alloc(8, 8)",
        f"        let ts = mem_alloc({8 * nthreads}, 8)",
        f"        let args = mem_alloc({24 * nthreads}, 8)",
        f"        for k in 0..{nthreads} {{",
        "            let a = args.offset(24 * k)",
        "            a.store_addr(shared)",
        "            a.offset(8).store_i64(k)",
        "            let rc = pthread_create(ts.offset(8 * k), mem_addr(0), mem_fn_addr(worker), a)",
        "            if rc != 0 {",
        '                print("pthread_create {rc}")',
        "            }",
        "        }",
        f"        for k in 0..{nthreads} {{",
        "            let rc = pthread_join(ts.offset(8 * k).load_addr(), mem_addr(0))",
        "        }",
        "        var verdicts: [i64] = []",
        f"        for k in 0..{nthreads} {{",
        "            verdicts.push(args.offset(24 * k + 16).load_i64())",
        "        }",
        '        print("{verdicts}")',
        '        print("{shared.load_i64()}")',
        '        print("{mem_tls(WHO).load_i64()}")',
    ]
    want = [str([1] * nthreads), str(nthreads * iters), "-5"]
    for n, t in cells:
        lines.append(f'        print("{{{show_load(t, f"mem_tls({n})")}}}")')
        want.append(str(main_vals[n]))
    lines += ["        mem_free(shared)", "        mem_free(ts)", "        mem_free(args)", "    }", "}"]
    # The arguments block is the first sandbox address handed to C.
    panic = "`grasm run` cannot hand C an address from its own sandbox (argument 1 of `pthread_create`)"
    return "\n".join(lines) + "\n", "\n".join(want) + "\n", panic


def misuse_file(rng, count):
    lines, want = [], []
    for i in range(count):
        t = rng.choice(LAYOUT)
        lines.append(f"thread_local M{i}: {t}")
    lines.append("")
    for i in range(count):
        t = lines[i].split(": ")[1]
        name = f"m{i}"
        wider = [w for w in LAYOUT if SIZE[w] > SIZE[t]]
        if wider and rng.random() < 0.6:
            w = rng.choice(wider)
            if rng.random() < 0.5:
                body = f"let _x = mem_tls(M{i}).load_{w}()"
                op = f"load_{w}"
            else:
                v = "mem_addr(8)" if w == "addr" else ("1.5" if w == "f64" else "1")
                body = f"mem_tls(M{i}).store_{w}({v})"
                op = f"store_{w}"
            msg = f"{op} of {SIZE[w]} byte(s) at offset 0 of a {SIZE[t]}-byte allocation"
        else:
            body = f"mem_free(mem_tls(M{i}))"
            msg = "mem_free of a thread-local cell"
        lines += [f'test "{name}" {{', "    unsafe {", f"        {body}", "    }", "}", ""]
        want.append((name, msg))
    lines.append("fn main() {\n}")
    return "\n".join(lines) + "\n", want


BAD = [
    ("thread_local A: str\n", "a thread-local cell cannot hold `str`"),
    ("thread_local A: bool\n", "a thread-local cell cannot hold `bool`"),
    ("thread_local A: [i64]\n", "a thread-local cell cannot hold `[i64]`"),
    ("thread_local A: Option[i64]\n", "a thread-local cell cannot hold `Option[i64]`"),
    ("type P = { x: i64 }\n\nthread_local A: P\n", "a thread-local cell cannot hold `P`"),
    ("pub thread_local A: i64\n", "thread-local cell `A` cannot be `pub`"),
    ("thread_local A: i64\nthread_local A: i64\n", "duplicate name `A`"),
    ("fn A() {\n}\n\nthread_local A: u8\n", "duplicate name `A`"),
    ("thread_local A: i64\n\nfn f() -> addr {\n    mem_tls(A)\n}\n",
     "`mem_tls` reaches raw memory, so it can only be used inside `unsafe`"),
    ("fn f() -> addr / {unsafe, mem} {\n    unsafe {\n        mem_tls(Nope)\n    }\n}\n",
     "`mem_tls` takes the name of a `thread_local` cell this module declares"),
    ("thread_local A: i64\n\nfn f() -> addr / {unsafe, mem} {\n    unsafe {\n        mem_tls(mem_tls(A))\n    }\n}\n",
     "`mem_tls` takes the name of a `thread_local` cell this module declares"),
    ("thread_local A: i64\n\nfn f() -> addr / {unsafe} {\n    unsafe {\n        mem_tls(A)\n    }\n}\n",
     "performs effect(s) `mem` not in its declared row"),
]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--programs", type=int, default=int(os.environ.get("TLS_PROGRAMS", "40")))
    ap.add_argument("--seed", type=int, default=int(os.environ.get("SEED", "0")) or random.randrange(1 << 30))
    ap.add_argument("--keep", action="store_true")
    o = ap.parse_args()
    rng = random.Random(o.seed)
    work = tempfile.mkdtemp(prefix="grasm_tls_")
    bad = 0
    checked = 0
    s3 = S3Engine(GRASM, work)
    try:
        path = os.path.join(work, "p.gr")
        exe = os.path.join(work, "p.bin")

        def keep(src, tag):
            if o.keep:
                with open(os.path.join(ROOT, f"tls_fail_{tag}.gr"), "w") as fh:
                    fh.write(src)

        def compare(engine, got, want, label, src):
            nonlocal bad
            if got.returncode != 0 or got.stdout != want:
                bad += 1
                print(f"{label}: {engine} disagrees with the model (exit {got.returncode})")
                wl, gl = want.splitlines(), got.stdout.splitlines()
                shown = 0
                for w, g in zip(wl, gl + [""] * len(wl)):
                    if w != g and shown < 4:
                        print(f"    want {w}\n    got  {g}")
                        shown += 1
                if got.stderr:
                    print("    stderr: " + got.stderr.strip()[:400])
                keep(src, label.replace(" ", "_"))

        for idx in range(o.programs):
            # 1. Both engines, against the model.
            src, want = program(rng, idx)
            checked += 2 * len(want.splitlines())
            with open(path, "w") as fh:
                fh.write(src)
            r = subprocess.run([GRASM, "run", path], capture_output=True, text=True, timeout=600)
            compare("interpreter", r, want, f"program {idx}", src)
            bp = build_native(GRASM, path, exe, "--native")
            if bp.returncode != 0:
                print(f"program {idx}: the native build failed:\n{(bp.stdout + bp.stderr)[:2000]}")
                bad += 1
                keep(src, idx)
                continue
            n = subprocess.run([exe], capture_output=True, text=True, timeout=600)
            s3.check(path, exe, f"program {idx} (SEED={o.seed})", interp=(r.returncode, r.stdout))
            compare("native", n, want, f"program {idx}", src)
            held = leaked(exe)
            if held:
                bad += 1
                print(f"program {idx}: LEAKS {held}")

            # 2. Threads, natively; refused by `grasm run`.
            if idx % 4 == 0:
                src, want, panic = threads_program(rng, idx)
                checked += len(want.splitlines()) + 1
                with open(path, "w") as fh:
                    fh.write(src)
                bp = build_native(GRASM, path, exe, "--native")
                if bp.returncode != 0:
                    print(f"threads {idx}: the build failed:\n{(bp.stdout + bp.stderr)[:2000]}")
                    bad += 1
                    keep(src, f"threads{idx}")
                    continue
                n = subprocess.run([exe], capture_output=True, text=True, timeout=600)
                s3.check(path, exe, f"threads {idx} (SEED={o.seed})")
                compare("native", n, want, f"threads {idx}", src)
                r = subprocess.run([GRASM, "run", path], capture_output=True, text=True, timeout=600)
                first = (r.stderr.strip().splitlines() or [""])[0]
                if r.returncode != 101 or panic not in first:
                    bad += 1
                    print(f"threads {idx}: `grasm run` gave exit {r.returncode}, `{first}`; want `{panic}`")

        # 3. Misuse, in the interpreter.
        src, cases = misuse_file(rng, max(20, o.programs))
        with open(path, "w") as fh:
            fh.write(src)
        t = subprocess.run([GRASM, "test", path], capture_output=True, text=True, timeout=600)
        report = t.stdout + t.stderr
        for name, msg in cases:
            checked += 1
            line = next((l for l in report.splitlines() if l.startswith(f'test "{name}" ...')), None)
            if line is None or not line.startswith(f'test "{name}" ... FAILED: {msg}'):
                bad += 1
                print(f"misuse {name}: want `{msg}`\n               got  `{line}`")
                keep(src, "misuse")

        # 4. The checker.
        for decl, need in BAD:
            with open(path, "w") as fh:
                fh.write(decl + "\nfn main() {\n}\n")
            c = subprocess.run([GRASM, "check", path], capture_output=True, text=True, timeout=120)
            checked += 1
            if c.returncode == 0 or need not in c.stdout + c.stderr:
                bad += 1
                print(f"not refused with `{need}`:\n{decl}\n{(c.stdout + c.stderr)[:600]}")

        if not s3.report():
            bad += 1
        print(f"programs: {o.programs}   checked lines: {checked}   disagreed: {bad}   (SEED={o.seed})")
    finally:
        if not o.keep:
            shutil.rmtree(work, ignore_errors=True)
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
