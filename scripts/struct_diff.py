"""Fixed binary records against Python's own `struct`."""
import os, random, struct, subprocess, sys
random.seed(401)
CODES = ["b", "B", "h", "H", "i", "I", "q", "Q", "?", "x"]
PREFIXES = ["<", ">", "!", "=", "@", ""]
cases = []
for prefix in PREFIXES:
    for code in CODES:
        cases.append(prefix + code)
        cases.append(prefix + "2" + code)
    cases += [prefix + "HHI", prefix + "BH", prefix + "HB", prefix + "iq",
              prefix + "BQ", prefix + "cB", prefix + "4sB", prefix + "B4s",
              prefix + "hxi", prefix + "qB", prefix + "BBBB", prefix + "3sH"]
for _ in range(300):
    n = random.randint(1, 5)
    cases.append(random.choice(PREFIXES) +
                 "".join(random.choice([c for c in CODES if c != "x"])
                         for _ in range(n)))

def values_for(fmt):
    numbers, texts = [], []
    for code in struct._clearcache.__self__ if False else []:
        pass
    # Walk the format the same way the package does.
    i = 1 if fmt[0] in "<>!=@" else 0
    while i < len(fmt):
        count = ""
        while i < len(fmt) and fmt[i].isdigit():
            count += fmt[i]
            i += 1
        code = fmt[i]
        i += 1
        n = int(count) if count else 1
        if code == "x":
            continue
        if code == "s":
            texts.append("ab")
            continue
        if code == "c":
            for _ in range(n):
                texts.append("Z")
            continue
        for _ in range(n):
            if code == "?":
                numbers.append(random.choice([0, 1]))
            elif code in "bB":
                numbers.append(random.randint(0, 127))
            elif code in "hH":
                numbers.append(random.randint(0, 32000))
            else:
                numbers.append(random.randint(0, 2_000_000))
    return numbers, texts

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
grasm = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
harness = os.path.join(ROOT, "packages/sptest/src/main.gr")
prepared = []
for fmt in cases:
    numbers, texts = values_for(fmt)
    prepared.append((fmt, numbers, texts))
def run_chunks(requests, per_request):
    """Whole requests per call: a chunk that splits one desynchronises all."""
    out = []
    step = 100
    for at in range(0, len(requests), step):
        flat = []
        for one in requests[at:at + step]:
            flat += one
        got = subprocess.run([grasm, "run", harness, "--"] + flat,
                             capture_output=True, text=True)
        if got.returncode != 0:
            print(got.stderr[-2000:])
            sys.exit(1)
        out += got.stdout.split("\n")[:len(requests[at:at + step])]
    return out

pack_requests = [["pack", fmt, ",".join(str(n) for n in numbers),
                  "|".join(texts)] for fmt, numbers, texts in prepared]
size_requests = [["size", fmt] for fmt, _n, _t in prepared]
lines = run_chunks(pack_requests, 4) + run_chunks(size_requests, 2)
agree = differ = shown = 0
half = len(prepared)
for i, (fmt, numbers, texts) in enumerate(prepared):
    args_for_struct = []
    numbers_left = list(numbers)
    texts_left = list(texts)
    j = 1 if fmt[0] in "<>!=@" else 0
    while j < len(fmt):
        count = ""
        while j < len(fmt) and fmt[j].isdigit():
            count += fmt[j]; j += 1
        code = fmt[j]; j += 1
        n = int(count) if count else 1
        if code == "x":
            continue
        if code == "s":
            args_for_struct.append(texts_left.pop(0).encode())
            continue
        if code == "c":
            for _ in range(n):
                args_for_struct.append(texts_left.pop(0).encode())
            continue
        for _ in range(n):
            value = numbers_left.pop(0)
            if code == "?":
                args_for_struct.append(bool(value))
            else:
                args_for_struct.append(value)
    try:
        packed = struct.pack(fmt, *args_for_struct).hex()
        unpacked = struct.unpack(fmt, bytes.fromhex(packed))
        plain = ",".join(str(int(v) if isinstance(v, (bool, int)) else 0)
                         for v in unpacked if not isinstance(v, bytes))
        want = packed + "\x02" + plain
        want_size = str(struct.calcsize(fmt))
    except Exception:
        want = "error"
        want_size = "error"
    if lines[i] == want:
        agree += 1
    else:
        differ += 1; shown += 1
        if shown <= 8:
            print(f"  pack {fmt!r} {args_for_struct}\n    python {want}\n"
                  f"    grasm  {lines[i]}")
    if lines[half + i] == want_size:
        agree += 1
    else:
        differ += 1; shown += 1
        if shown <= 8:
            print(f"  size {fmt!r}: python {want_size}, grasm {lines[half + i]}")
print(f"formats: {len(prepared)}   checks: {agree + differ}   "
      f"agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
