// The playground: the reference interpreter, built for wasm32, run in the tab.
//
// The module and the examples are files beside this one, written by
// `scripts/site.py`: `grasm.wasm` is the exact module `scripts/wasmtest.sh`
// holds to the other engines, and `playground.json` is read from `examples/`.
// Nothing is fetched from anywhere else.

const $ = (id) => document.getElementById(id);
const els = {
  pick: $("pg-example"),
  engine: $("pg-engine"),
  run: $("pg-run"),
  src: $("pg-src"),
  out: $("pg-out"),
  status: $("pg-status"),
};

// The host facilities the module imports. The same four the corpus host in
// `ProjectGold/web/` provides, for the same reasons: `std::time` panics on
// this target, and a panic with no unwinding takes the module down.
const imports = {
  env: {
    grasm_now_millis: () => Date.now(),
    grasm_monotonic_millis: () => performance.now(),
    // A call from wasm into the host cannot yield, so a sleep is a spin: the
    // thread really is unavailable for that long, which is what was asked.
    grasm_sleep_millis: (ms) => {
      const until = performance.now() + ms;
      while (performance.now() < until) {}
    },
    dlsym: () => 0,
  },
};

let compiled = null;

// A fresh instance per run. A trap -- a stack exhausted, say -- leaves its
// instance unusable, so reusing one would let one bad program break every run
// after it. Instantiating costs about a millisecond.
function run(source) {
  const ex = new WebAssembly.Instance(compiled, imports).exports;
  const mem = () => new Uint8Array(ex.memory.buffer);
  const bytes = new TextEncoder().encode(source);
  const sp = ex.grasm_alloc(bytes.length);
  mem().set(bytes, sp);
  const res = ex.grasm_run_with_stdin(sp, bytes.length, 0, 0);
  ex.grasm_free(sp, bytes.length);

  // The frame: total length, exit code, stdout, then diagnostics.
  const v = new DataView(ex.memory.buffer);
  const total = v.getUint32(res, true);
  const code = v.getInt32(res + 4, true);
  const outLen = v.getUint32(res + 8, true);
  const dec = new TextDecoder();
  const out = dec.decode(mem().slice(res + 12, res + 12 + outLen));
  const eo = res + 12 + outLen;
  const errLen = v.getUint32(eo, true);
  const err = dec.decode(mem().slice(eo + 4, eo + 4 + errLen));
  ex.grasm_free(res, total);
  return { code, out, err };
}

function setStatus(kind, text) {
  els.status.className = "pg-status " + kind;
  els.status.textContent = text;
}

function onRun() {
  if (!compiled) return;
  els.run.disabled = true;
  setStatus("busy", "Running…");
  // Yield once so the label paints before a long program holds the thread.
  setTimeout(() => {
    const t = performance.now();
    try {
      const r = run(els.src.value);
      const ms = (performance.now() - t).toFixed(1);
      const text = r.out + (r.err ? (r.out ? "\n" : "") + r.err : "");
      els.out.textContent = text || "(no output)";
      if (r.code === 0) setStatus("ok", `Exit 0 · ${ms} ms`);
      else setStatus("bad", `Exit ${r.code} · ${ms} ms`);
    } catch (e) {
      els.out.textContent = String(e);
      setStatus("bad", "The engine trapped. Edit the program and run it again.");
    }
    els.run.disabled = false;
  }, 0);
}

// Tab indents rather than leaving the editor, as it does in every code
// editor; Escape then Tab still leaves, so the keyboard is never trapped.
let escaped = false;
els.src.addEventListener("keydown", (e) => {
  if (e.key === "Escape") { escaped = true; return; }
  if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
    e.preventDefault();
    onRun();
    return;
  }
  if (e.key === "Tab" && !e.shiftKey && !escaped) {
    e.preventDefault();
    const { selectionStart: a, selectionEnd: b, value } = els.src;
    els.src.value = value.slice(0, a) + "    " + value.slice(b);
    els.src.selectionStart = els.src.selectionEnd = a + 4;
  }
  escaped = false;
});
els.run.addEventListener("click", onRun);

async function boot() {
  const [bytes, examples] = await Promise.all([
    fetch("grasm.wasm").then((r) => {
      if (!r.ok) throw new Error(`grasm.wasm: ${r.status}`);
      return r.arrayBuffer();
    }),
    fetch("playground.json").then((r) => r.json()),
  ]);
  compiled = await WebAssembly.compile(bytes);

  for (const ex of examples) {
    const o = document.createElement("option");
    o.value = ex.id;
    o.textContent = ex.title;
    els.pick.appendChild(o);
  }
  const show = (id) => {
    const ex = examples.find((x) => x.id === id);
    els.src.value = ex.src;
    els.out.textContent = "Press Run, or Cmd/Ctrl + Enter.";
    setStatus("", "");
  };
  els.pick.addEventListener("change", () => show(els.pick.value));
  show(examples[0].id);

  els.engine.textContent =
    `Interpreter · ${(bytes.byteLength / 1048576).toFixed(1)} MB wasm`;
  els.pick.disabled = false;
  els.run.disabled = false;
}

boot().catch((e) => {
  els.engine.textContent = "The engine did not load.";
  els.out.textContent =
    `${e}\n\nThe playground needs WebAssembly and has to be served over ` +
    "HTTP; opening the file directly blocks the fetch. " +
    "`python3 -m http.server -d site` serves it.";
});
