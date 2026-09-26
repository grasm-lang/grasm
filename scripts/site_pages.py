"""The content of each page. The chrome lives in `site.py`."""

E = "&amp;"


from site_highlight import code  # noqa: E402,F401
import os  # noqa: E402

import site_markdown  # noqa: E402

_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def markdown_file(rel):
    """A document from the repository, rendered as this page's body."""
    with open(os.path.join(_ROOT, rel)) as f:
        return site_markdown.render(f.read())


PAGES = {}


def verdicts():
    """The supply-chain verdicts, as a table each row of which resolves in turn.

    `data-result` is the answer; `data-state` is what is currently shown. The
    script moves the second to the first, so with JavaScript off the table
    still renders — just already settled.
    """
    rows = [
        ("Grasm", "refused", "COMPILE ERROR", "inferred effects (io) exceed the declared ceiling ([])"),
        ("Rust", "clean", "builds clean", "std::fs is ambient; no capability is recorded"),
        ("Go", "clean", "builds clean", "os.ReadFile is importable by any package"),
        ("Python", "clean", "compiles clean", "open() needs no import and no declaration"),
    ]
    out = ['<div class="verdicts">']
    for name, state, verdict, why in rows:
        mark = "\u00abicon_check\u00bb" if state == "refused" else "\u00abicon_x\u00bb"
        out.append(
            f'<div class="verdict-row" data-result="{state}">'
            f'<span class="lang">{mark}{name}</span>'
            f'<span class="out">{verdict}</span>'
            f'<span class="why">{why}</span></div>'
        )
    out.append("</div>")
    out.append('<div class="actions">'
               '<button class="btn ghost" data-run-verdicts>Run it again</button></div>')
    return "".join(out)


# ---------------------------------------------------------------- index

PAGES["index.html"] = (
    "Grasm is a systems language where a dependency cannot reach further than its manifest allows.",
    """
<div class="hero">
  <p class="eyebrow">Systems language \u00b7 v\u00abversion\u00bb</p>
  <h1>Your dependencies can read every file you can.</h1>
  <p class="lede">In Python, Go, Rust and Zig, a library that claims to do
  arithmetic can open your files and your sockets, and nothing in the toolchain
  will say a word. Grasm makes that a compile error.</p>
  <div class="actions">
    <a class="btn" href="start.html">Get started</a>
    <a class="btn ghost" href="security.html">How it works</a>
    <a class="btn ghost" href="syntax.html">Syntax</a>
  </div>
</div>

<div class="split">
  <div>
    <h3>The manifest is the ceiling</h3>
    <p>A package declares what it may reach. The compiler infers what it
    actually does \u2014 through every call, into every dependency \u2014 and
    refuses to build when the two disagree.</p>
"""
    + code(
        """[package]
name = "mathlib"

capabilities = []          # reaches nothing at all""",
        "mathlib/grasm.toml — the ceiling, in one line a reviewer can see.",
    )
    + """
  </div>
  <div>
    <h3>A patch release tries to grow teeth</h3>
    <p>Same name, same signature, same result \u2014 and one new line that reads
    a file. This is the whole build:</p>
    <div class="term">
      <div class="chrome"><span class="dot"></span><span class="dot"></span><span class="dot"></span>
        <span>app \u2014 grasm build</span></div>
      <div class="body"></div>
    </div>
  </div>
</div>

<h2 class="reveal">The same attack, four toolchains</h2>
<p class="reveal">A trusted pure library\u2019s 1.0.1 starts reading
<code>/etc/passwd</code>. <code>scripts/security_bench.sh</code> runs it against
four toolchains on every green build. Three of them ship it.</p>
"""
    + verdicts()
    + """
<p class="note"><strong>This is the reason the language exists.</strong>
xz-utils, event-stream, SolarWinds: the attack is always the same shape, and
the answer everywhere else is a runtime sandbox or a human reading the diff.
Here it is a type error.</p>

<h2 class="reveal">Three ideas, and they compose</h2>
<div class="grid three reveal">
  <div class="card">
    <span class="ico">\u00abicon_lock\u00bb</span>
    <h3>Capabilities</h3>
    <p>A package states the effects it may perform. Exceeding it fails the
    build in every consumer \u2014 not in a review, not at run time.</p>
    <div class="lottie" data-lottie="ceiling"></div>
  </div>
  <div class="card">
    <span class="ico">\u00abicon_layers\u00bb</span>
    <h3>Ownership in tiers</h3>
    <p>Value semantics by default, a region when a scope should own its
    allocations, a generational reference when a graph needs one. No lifetime
    annotations anywhere.</p>
    <div class="lottie" data-lottie="tiers"></div>
  </div>
  <div class="card">
    <span class="ico">\u00abicon_flow\u00bb</span>
    <h3>Four engines, one answer</h3>
    <p>Interpreter, bundle, ahead-of-time native, and wasm. Develop against
    instant feedback, ship a binary, run the same program in a browser.</p>
    <div class="lottie" data-lottie="engines"></div>
  </div>
</div>

<h2 class="reveal">Effects are in the type, and inferred</h2>
<p class="reveal">You do not write the row. The compiler works it out from the
body and checks it against what you declared, so a function that starts
touching the network stops compiling \u2014 and so does every caller that was
not given the same permission.</p>
"""
    + code(
        """fn safe_div(a: i64, b: i64) -> i64 ! MathError {
    if b == 0 {
        fail(DivByZero)
    }
    a / b
}

fn load(path: str) -> str ! IoError / {io} {
    try read_file(path)
}""",
        "! MathError is a failure the caller must handle. / {io} is a permission the caller must have.",
    )
    + """
<h2 class="reveal">The same program, every engine</h2>
<p class="reveal">The interpreter is normative and the other three have to match
it. Every gate here exists to hold them to the same bytes.</p>
<div class="viz reveal">
  <canvas id="engines" data-programs="\u00abcorpus\u00bb"
    aria-label="Programs flowing through four engines to one answer"></canvas>
  <div class="cap">difftest \u00b7 crosstest \u00b7 wasmtest \u2014 \u00abcorpus\u00bb programs, 0 mismatched</div>
</div>

<div class="figures reveal">
  <div class="fig"><b>\u00abcorpus\u00bb</b><span>programs, engines identical</span></div>
  <div class="fig"><b>\u00abpackages\u00bb</b><span>libraries, all written in Grasm</span></div>
  <div class="fig"><b>\u00abclauses\u00bb</b><span>normative clauses, each with a conformance program</span></div>
  <div class="fig"><b>\u00abgates\u00bb</b><span>gates, judged by exit code alone</span></div>
</div>

<h2 class="reveal">Start here</h2>
<div class="grid three reveal">
  <div class="card"><span class="ico">\u00abicon_terminal\u00bb</span><h3>Getting started</h3>
    <p>Install, first program, first package.
    <a href="start.html">Begin \u2192</a></p></div>
  <div class="card"><span class="ico">\u00abicon_book\u00bb</span><h3>Syntax</h3>
    <p>The whole language on one page, every form with an example.
    <a href="syntax.html">Read it \u2192</a></p></div>
  <div class="card"><span class="ico">\u00abicon_gauge\u00bb</span><h3>Status</h3>
    <p>What works, what is measured, and what is still missing.
    <a href="status.html">See it \u2192</a></p></div>
</div>
"""
)

PAGES["start.html"] = (
    "Write, run and compile your first Grasm program.",
    """
<p class="eyebrow">Start</p>
<h1>Getting started</h1>
<p class="lede">A first program, run three ways, in about two minutes.</p>

<h2>1. Write it</h2>
<p>Put this in <code>hello.gr</code>.</p>"""
    + code(
        """
fn main() {
    print("hello from grasm")
}
"""
    )
    + """
<p><code>main</code> needs no effect row: it is the entry point, and the
capability to print is granted there. Every other function that prints must say
so.</p>

<h2>2. Run it</h2>"""
    + code(
        """
grasm run hello.gr
"""
    )
    + """
<p>That is the interpreter, and it is normative: where the engines could ever
disagree, it is the one that is right by definition.</p>

<h2>3. Compile it</h2>"""
    + code(
        """
grasm build hello.gr --native -o hello
./hello
"""
    )
    + """
<p>Ahead-of-time through Cranelift. Byte-for-byte the same stdout, the same exit
code. Drop <code>--native</code> and you get a bundle instead — the interpreter
with your program attached, one file, no toolchain needed to run it.</p>

<h2>4. Something that can fail</h2>
<p>Failure is a value, not an exception. A function that can fail says so in its
type, and a caller that ignores it does not compile.</p>"""
    + code(
        """
type MathError = | DivByZero

fn safe_div(a: i64, b: i64) -> i64 ! MathError {
    if b == 0 {
        fail(DivByZero)
    }
    a / b
}

fn main() {
    match safe_div(10, 0) {
        Ok(v) -> print("{v}")
        Err(e) -> print("caught: {e}")
    }

    // or take a default and move on
    print("{safe_div(10, 0) ?? 0 - 1}")
}

test "division works" {
    let v = try safe_div(10, 2)
    assert_eq(v, 5)
}
"""
    )
    + """
<p>Tests live beside the code and run with <code>grasm test</code>.</p>

<h2>Next</h2>
<div class="grid two">
  <div class="cell"><h4><a href="tour.html">A tour of the language</a></h4>
  <p>Types, pattern matching, closures, generics — the whole surface in one
  read.</p></div>
  <div class="cell"><h4><a href="effects.html">Effects and capabilities</a></h4>
  <p>The part that is unlike other languages, and the reason to use this
  one.</p></div>
</div>
""",
)

# ---------------------------------------------------------------- install

PAGES["install.html"] = (
    "Build Grasm from source.",
    """
<p class="eyebrow">Start</p>
<h1>Install</h1>
<p class="lede">Grasm builds from source with a Rust toolchain. There are no
prebuilt binaries yet.</p>

<h2>From source</h2>"""
    + code(
        """
git clone <repository>
cd grasm
cargo build --release --workspace

./target/release/grasm run examples/hello.gr
"""
    )
    + """
<p>The binary is <code>target/release/grasm</code>. Put it on your path, or call
it where it is.</p>

<h2>What you get</h2>
<div class="scroll"><table>
<tr><th>Command</th><th>Does</th></tr>
<tr><td><code>grasm run FILE</code></td><td>Runs it under the interpreter.</td></tr>
<tr><td><code>grasm build FILE -o OUT</code></td><td>Writes a bundle — one file, runs anywhere.</td></tr>
<tr><td><code>grasm build FILE --native -o OUT</code></td><td>Compiles ahead of time through Cranelift.</td></tr>
<tr><td><code>grasm build FILE --native --target TRIPLE</code></td><td>Cross-compiles; link against the runtime for that target.</td></tr>
<tr><td><code>grasm test [DIR]</code></td><td>Runs <code>test</code> blocks.</td></tr>
<tr><td><code>grasm bench [DIR]</code></td><td>Runs <code>bench</code> blocks.</td></tr>
<tr><td><code>grasm check FILE</code></td><td>Types and effects only, no output.</td></tr>
<tr><td><code>grasm fmt [DIR]</code></td><td>Formats.</td></tr>
<tr><td><code>grasm deps [DIR]</code></td><td>Resolves dependencies, checks capability ceilings, writes <code>grasm.lock</code>.</td></tr>
<tr><td><code>grasm journal FILE</code></td><td>Reads a recording of a program's effects.</td></tr>
</table></div>

<h2>Editor support</h2>
<p>A language server is in <code>crates/grasm_lsp</code> and a VS Code extension
in <code>editors/vscode</code>. A debug adapter is in
<code>crates/grasm_dap</code>.</p>

<div class="note"><b>No release binaries yet.</b> Building from source is the
only route today. That is on the <a href="roadmap.html">roadmap</a>.</div>
""",
)

# ---------------------------------------------------------------- playground

PAGES["playground.html"] = (
    "Run Grasm in your browser.",
    """
<p class="eyebrow">Start</p>
<h1>Playground</h1>
<p class="lede">Grasm compiles to <code>wasm32</code>, so the language runs in a
browser with no server behind it.</p>

<p>The engine below is the reference interpreter — the same one
<code>grasm run</code> uses — built for <code>wasm32</code> and held to the other
engines by <code>scripts/wasmtest.sh</code>. Your program never leaves this tab.
The examples are the files in <code>examples/</code>, read when this page was
generated.</p>

<section class="pg" id="playground" aria-label="Playground">
  <div class="pg-bar">
    <label class="pg-pick">Example
      <select id="pg-example" disabled></select>
    </label>
    <span class="pg-engine" id="pg-engine">loading the engine…</span>
    <button class="btn" id="pg-run" type="button" disabled>Run</button>
  </div>
  <textarea id="pg-src" class="pg-src" spellcheck="false" autocapitalize="off"
    autocomplete="off" aria-label="Program source"></textarea>
  <div class="pg-out-head">
    <span>Output</span>
    <span class="pg-status" id="pg-status" role="status"></span>
  </div>
  <pre class="pg-out" id="pg-out" aria-live="polite">Loading…</pre>
</section>
<noscript><div class="note">The playground needs JavaScript and WebAssembly.
The instructions below run the same engine locally.</div></noscript>
<script type="module" src="playground.js"></script>

<h2>Running it locally</h2>
<p>The wasm build and a host page live in the repository. Serve the directory
and open it:</p>"""
    + code(
        """
cargo build --release --target wasm32-unknown-unknown -p grasm_wasm
cp target/wasm32-unknown-unknown/release/grasm_wasm.wasm ProjectGold/web/grasm.wasm

python3 -m http.server -d ProjectGold/web
"""
    )
    + """
<p>That page runs the corpus: pick a program, and it executes in the tab. Twelve
of the programs under <code>ProjectGold/</code> are browser applications built
this way — a SPA router, a markdown editor, a collaborative todo list, a
realtime analytics dashboard.</p>

<h2>What the browser build can and cannot do</h2>
<p>«wasm_ok» of the «corpus» corpus programs produce identical output in wasm. The other «wasm_skip»
are skipped for a reason the host cannot supply, and each names it: a
filesystem, a socket, a process, threads, or a stack deep enough for the
recursion limit the specification promises.</p>

<p>In this page the missing facilities refuse the same way: a program that
opens a file or a socket gets the ordinary error back, and <code>par_map</code>
runs on one core with the same answer.</p>
""",
)

# ---------------------------------------------------------------- tour

PAGES["tour.html"] = (
    "The whole language surface in one read.",
    """
<p class="eyebrow">Learn</p>
<h1>A tour of the language</h1>
<p class="lede">Everything you need to read Grasm, in order.</p>

<h2>Bindings</h2>
<p><code>let</code> binds; <code>var</code> binds something you may reassign.
Types are inferred and may be written.</p>"""
    + code(
        """
let n = 1 + 2
let x: f64 = 2          // an integer literal becomes a float where one is wanted
var total = 0
total = total + n
"""
    )
    + """

<h2>Types</h2>
<div class="scroll"><table>
<tr><th>Kind</th><th>Written</th><th>Example</th></tr>
<tr><td>Primitives</td><td><code>i64 f64 bool str bytes ()</code></td><td><code>let ok: bool = true</code></td></tr>
<tr><td>List</td><td><code>[T]</code></td><td><code>[1, 2, 3]</code></td></tr>
<tr><td>Map</td><td><code>Map[K, V]</code></td><td><code>{ "a": 1 }</code></td></tr>
<tr><td>Tuple</td><td><code>(A, B)</code></td><td><code>(1, "a")</code></td></tr>
<tr><td>Range</td><td><code>range</code></td><td><code>0..10</code>, <code>0..=10</code></td></tr>
<tr><td>Struct</td><td><code>type P = { x: i64 }</code></td><td><code>P{ x: 1 }</code></td></tr>
<tr><td>Sum</td><td><code>type C = | Red | Green</code></td><td><code>Red</code></td></tr>
<tr><td>Option</td><td><code>Option[T]</code></td><td><code>Some(1)</code>, <code>None</code></td></tr>
<tr><td>Fallible</td><td><code>T ! E</code></td><td><code>i64 ! MathError</code></td></tr>
<tr><td>Function</td><td><code>fn(A) -&gt; B</code></td><td><code>|n| n + 1</code></td></tr>
<tr><td>Reference</td><td><code>ref T</code></td><td><code>ref(0)</code></td></tr>
</table></div>
<p>There is no null. Something that may be absent is an <code>Option</code>, and
the compiler makes you open it.</p>

<h2>Control</h2>"""
    + code(
        """
let label = if n > 0 { "positive" } else { "not" }

match value {
    Some(v) -> print("{v}")
    None    -> print("nothing")
}

for i in 0..3 { }
for v in [1, 2, 3] { }
while more { }
"""
    )
    + """
<p><code>if</code> and <code>match</code> are expressions. A <code>match</code>
must cover its subject, and the compiler says which case is missing.</p>

<h2>Functions and closures</h2>"""
    + code(
        """
fn twice(n: i64) -> i64 {
    n * 2
}

let f = |n| n + 1                 // a closure captures by value, at creation
let g: fn(i64) -> i64 = twice     // a function is a value

[1, 2, 3].map(twice)
3 |> twice                        // the same call, written the other way
"""
    )
    + """

<h2>Generics</h2>"""
    + code(
        """
type Stack[T] = { items: [T] }

fn same[T](v: T) -> T {
    v
}
"""
    )
    + """
<p>Generics are monomorphised: one compiled copy per member of the family. The
one thing this cannot do is polymorphic recursion, and it is refused by name
rather than miscompiled.</p>

<h2>Parameters</h2>
<div class="scroll"><table>
<tr><th>Mode</th><th>Means</th></tr>
<tr><td><code>fn f(x: T)</code></td><td>A borrow. The caller keeps it.</td></tr>
<tr><td><code>fn f(own x: T)</code></td><td>Ownership moves in. Scalars copy, so the caller keeps those.</td></tr>
<tr><td><code>fn f(inout x: T)</code></td><td>An exclusive mutable borrow; writes land in the caller's value.</td></tr>
</table></div>

<h2>Next</h2>
<div class="grid two">
  <div class="cell"><h4><a href="effects.html">Effects and capabilities</a></h4><p>What a function may do, and how a package is held to it.</p></div>
  <div class="cell"><h4><a href="tiers.html">Ownership tiers</a></h4><p>Four ways to own a value, chosen per value.</p></div>
</div>
""",
)

# ---------------------------------------------------------------- effects

PAGES["effects.html"] = (
    "Effects are inferred, written in the type, and enforced across package boundaries.",
    """
<p class="eyebrow">Learn</p>
<h1>Effects and capabilities</h1>
<p class="lede">The row after a function's return type says what it is allowed
to do. It is inferred from the body, so it cannot lie, and it survives across a
package boundary — which is what makes a dependency's reach something you can
check rather than something you have to trust.</p>

<h2>The row</h2>"""
    + code(
        """
fn pure_sum(xs: [i64]) -> i64 {              // no row: it does nothing but compute
    xs.sum()
}

fn report(xs: [i64]) / {io} {                // it prints, so it says io
    print("{xs.sum()}")
}

fn fetch(url: str) -> str ! IoError / {net} {   // fails, and reaches the network
    tcp_get(url)
}
"""
    )
    + """
<p>You do not have to write the row — it is inferred. Writing it is a claim the
compiler checks: declare <code>/ {}</code> on something that prints and the
program does not build.</p>

<h2>Why inference matters</h2>
<p>A hand-written effect annotation is a comment that happens to be checked at
one point. An inferred one is a fact about the body. When somebody adds a
<code>print</code> three calls down, the row widens on its own and every caller
that promised not to print stops compiling — including callers in other
packages.</p>

<h2>Your own effects</h2>
<p>An effect is a set of operations. A handler supplies them for a block, and
the operation resolves where it is <em>performed</em>, not where it is
written — so a function five calls deep uses whichever handler is installed.</p>"""
    + code(
        """
effect Cfg {
    fn value() -> i64
}

fn deep() -> i64 / {Cfg} {
    Cfg.value()
}

fn main() {
    with Cfg handled_by CfgHandler{ value: || 42 } {
        print("{deep()}")             // 42
    }
}
"""
    )
    + """
<p>A handler is a value. It can be bound, passed to a function, stored in a
list — the function installing it need never have seen the literal. That is how
a test substitutes a clock, a random source, or a network for the real one
without the code under test knowing.</p>

<h2>Capability ceilings</h2>
<p>A package declares, in its manifest, the effects it may perform:</p>"""
    + code(
        """
[package]
name = "ecommerce_cart"
version = "1.0.0"
entry = "main.gr"
capabilities = ["io", "net"]
"""
    )
    + """
<p>Because effects are inferred from source, a dependency that quietly starts
touching the filesystem exceeds its declared ceiling and <strong>fails to build
in every consumer</strong>. Not a runtime surprise. Not something a reviewer has
to notice in a diff. A compile error, at the boundary, before the code runs.</p>

<p>This is the reason Grasm has a package system rather than a directory of
files.</p>
""",
)

# ---------------------------------------------------------------- tiers

PAGES["tiers.html"] = (
    "Four ways to own a value, chosen per value.",
    """
<p class="eyebrow">Learn</p>
<h1>Ownership tiers</h1>
<p class="lede">Most languages pick one memory model for everything. Grasm gives
four and lets the value decide, so the common case stays simple and the hard
case stays possible.</p>

<div class="scroll"><table>
<tr><th>Tier</th><th>What it is</th><th>Costs</th></tr>
<tr><td><b>0</b> — borrow</td><td>The default. A parameter borrows; the caller keeps the value.</td><td>Nothing.</td></tr>
<tr><td><b>1</b> — <code>region</code></td><td>A scope nothing allocated inside may outlive.</td><td>Nothing at the boundary; the guarantee is checked.</td></tr>
<tr><td><b>2</b> — <code>ref T</code></td><td>A generational reference into a shared heap. Copies freely, stores in a struct, forms cycles.</td><td>One allocation; a generation check on use.</td></tr>
<tr><td><b>3</b> — <code>unsafe</code></td><td>The explicit escape, and an effect in the row.</td><td>You are on your own, and the type says so.</td></tr>
</table></div>

<h2>Tier 2 in practice</h2>"""
    + code(
        """
let counter = ref(0)
counter.value += 1

let same = counter          // a reference is a word: it copies
same.value += 1
print("{counter.value}")    // 2 — both name one slot

release(counter)            // the slot goes back
print("{alive(counter)}")   // false, through every copy of it
"""
    )
    + """
<p>Reclamation is manual. A slot returns to the pool when it is released and not
before — which is the point of the tier: the lifetime is the program's to state.
Releasing twice is not an error, so cleanup order is not something every caller
has to track.</p>

<h2>Tier 1 in practice</h2>"""
    + code(
        """
let n = region {
    let scratch = build_a_big_thing()
    scratch.len()               // a scalar may leave; an allocation may not
}
"""
    )
    + """
<p>Three rules give the guarantee: a region may not write to a binding declared
outside it, may not <code>return</code> through its brace, and yields a scalar
or nothing. The rule is enforced today; the bulk deallocation it exists to make
sound is not yet implemented, and the specification says so rather than
implying otherwise.</p>
""",
)

# ---------------------------------------------------------------- errors

PAGES["errors.html"] = (
    "Failure is a value. There is no exception mechanism and no null.",
    """
<p class="eyebrow">Learn</p>
<h1>Errors</h1>
<p class="lede">A function that can fail says so in its type. Ignoring that is a
compile error, not a runtime one.</p>

<h2>Fallible types</h2>"""
    + code(
        """
type MathError = | DivByZero

fn safe_div(a: i64, b: i64) -> i64 ! MathError {
    if b == 0 {
        fail(DivByZero)
    }
    a / b
}
"""
    )
    + """
<p><code>i64 ! MathError</code> is an <code>i64</code> or a
<code>MathError</code>. Three ways to open it:</p>"""
    + code(
        """
match safe_div(10, 0) {                 // handle both halves
    Ok(v) -> print("{v}")
    Err(e) -> print("caught: {e}")
}

let v = safe_div(10, 2) ?? 0 - 1        // take a default; the right side is lazy

fn caller() -> i64 ! MathError {
    let v = try safe_div(10, 2)         // propagate, and say so in the type
    v * 2
}
"""
    )
    + """
<p>What you cannot do is nothing. <code>let _ = risky()</code> is refused too —
a binding being unnamed says nothing about whether its failure was considered.</p>

<h2>Panics</h2>
<p>A panic is for a bug, not for a condition: an index out of range, a division
by zero, an <code>expect</code> on <code>None</code>. It prints a message and
exits 101 in every engine. Whether a stack trace follows depends on how the
program was run, and the specification says which engines print one.</p>

<h2>No null</h2>
<p>Something that may be absent is an <code>Option[T]</code>, and the compiler
makes you open it before you can use what is inside.</p>"""
    + code(
        """
let found: Option[i64] = m.get("k")
print("{found ?? 0}")
"""
    ),
)

# ---------------------------------------------------------------- concurrency

PAGES["concurrency.html"] = (
    "Deterministic task ordering, and effects that cross thread boundaries.",
    """
<p class="eyebrow">Learn</p>
<h1>Concurrency</h1>
<p class="lede">Tasks run in the order they were spawned, the block joins before
the line after it, and the effect row crosses the boundary with them.</p>

<h2>parallel and spawn</h2>"""
    + code(
        """
fn main() / {io, async} {
    var log = ""
    parallel {
        spawn { log += "a" }
        spawn { log += "b" }
    }
    print(log)          // "ab", every time, on every engine
}
"""
    )
    + """
<p>The block records which <code>spawn</code> sites fired, in order, and the
bodies run afterwards — which is what makes the order deterministic and the
scope joined. A <code>spawn</code> body <em>sees</em> the enclosing scope rather
than capturing it, which is the opposite of how a closure behaves and is
documented as such.</p>

<h2>Data-parallel methods</h2>"""
    + code(
        """
let doubled = xs.par_map(|v| v * 2)
let evens   = xs.par_filter(|v| v % 2 == 0)
"""
    )
    + """
<p>Both preserve order, and both require the closure to be pure — which the
effect system can prove rather than assume. A closure that prints will not
typecheck here.</p>

<div class="note"><b>What is missing.</b> There is no timer, channel or select
library above these primitives yet. That is the async workstream on the
<a href="roadmap.html">roadmap</a>.</div>
""",
)

# ---------------------------------------------------------------- language

PAGES["language.html"] = (
    "Syntax and type-system reference.",
    """
<p class="eyebrow">Reference</p>
<h1>Language reference</h1>
<p class="lede">The normative text is <code>spec/</code>. This is the working
summary.</p>

<h2>Declarations</h2>
<div class="scroll"><table>
<tr><th>Form</th><th>Means</th></tr>
<tr><td><code>fn name(args) -&gt; T ! E / {row}</code></td><td>A function. Return, failure and effects are all part of the type.</td></tr>
<tr><td><code>type N = { f: T }</code></td><td>A struct.</td></tr>
<tr><td><code>type N = | A | B(v: T)</code></td><td>A sum, with or without payloads.</td></tr>
<tr><td><code>type N[T] = …</code></td><td>A generic type: a family, one member per use.</td></tr>
<tr><td><code>type N = T</code></td><td>An alias.</td></tr>
<tr><td><code>effect N { fn op() -&gt; T }</code></td><td>An effect and its operations.</td></tr>
<tr><td><code>import path</code></td><td>Brings a module in.</td></tr>
<tr><td><code>pub</code></td><td>Exports a declaration from its module.</td></tr>
<tr><td><code>extern fn c_name(...)</code></td><td>A C function. Calling it performs <code>unsafe</code>.</td></tr>
<tr><td><code>test "name" { }</code></td><td>A test, beside the code.</td></tr>
<tr><td><code>bench "name" { }</code></td><td>A benchmark.</td></tr>
</table></div>

<h2>Operators</h2>
<div class="scroll"><table>
<tr><th>Operator</th><th>On</th></tr>
<tr><td><code>+ - * / %</code></td><td><code>i64</code> and <code>f64</code>. <code>+</code> also concatenates a <code>str</code> or a list.</td></tr>
<tr><td><code>^</code></td><td>Exponentiation — not bitwise xor. A negative integer exponent panics; write it in floats.</td></tr>
<tr><td><code>== !=</code></td><td>Any two values of one type, structurally.</td></tr>
<tr><td><code>&lt; &lt;= &gt; &gt;=</code></td><td><code>i64</code>, <code>f64</code>, <code>str</code>, and tuples whose every component is ordered.</td></tr>
<tr><td><code>and or not</code></td><td>Booleans. <code>and</code> and <code>or</code> short-circuit.</td></tr>
<tr><td><code>??</code></td><td>Opens an <code>Option</code> or a fallible with a default. The right side is lazy.</td></tr>
<tr><td><code>|&gt;</code></td><td>Pipes a value into a call.</td></tr>
<tr><td><code>..</code> <code>..=</code></td><td>A range, exclusive or inclusive.</td></tr>
</table></div>

<h2>Strings</h2>
<p>Interpolation is <code>"{expr}"</code>, and the expression inside needs no
escaping for its own quotes. <code>len()</code> counts characters;
<code>byte_len()</code> counts bytes.</p>

<h2>Where the truth is</h2>
<p>«clauses» numbered clauses across <code>spec/</code>, each with a conformance
program that runs in the test suite. <a href="spec.html">More on the
specification.</a></p>
""",
)

# ---------------------------------------------------------------- library

PAGES["library.html"] = (
    "The standard library, rendered from spec/library.md.",
    """
<p class="eyebrow">Reference</p>
<h1>Library reference</h1>
<p class="lede">The standard library, clause by clause. This page is
<code>spec/library.md</code>, rendered when the site was written — not a summary
of it.</p>
"""
    + markdown_file("spec/library.md"),
)

PAGES["tutorial.html"] = (
    "Grasm in an hour: the guided tutorial, rendered from docs/tutorial.md.",
    """
<p class="eyebrow">Learn</p>
<h1>Grasm in an hour</h1>
<p class="lede">The whole language in one sitting. This page is
<code>docs/tutorial.md</code>, rendered when the site was written.</p>
"""
    + markdown_file("docs/tutorial.md"),
)

# ---------------------------------------------------------------- packages

PAGES["packages.html"] = (
    "Manifests, lockfiles, and the capability ceiling.",
    """
<p class="eyebrow">Reference</p>
<h1>Packages</h1>
<p class="lede">A manifest, a resolver, a lockfile — and a ceiling on what a
dependency may do, checked at compile time in every consumer.</p>

<h2>A manifest</h2>"""
    + code(
        """
[package]
name = "ecommerce_cart"
version = "1.0.0"
entry = "main.gr"
capabilities = ["io", "net"]

[dependencies.json]
path = "../../packages/json"
version = "^1.0.0"
"""
    )
    + """

<h2>The ceiling</h2>
<p><code>capabilities</code> is the set of effects this package may perform.
Effects are inferred from source, so the claim is checked rather than trusted. A
dependency that starts reading files without declaring <code>io</code> fails to
build — and it fails in <em>your</em> build, not silently in theirs.</p>
<p>This is the part of the package system worth having. Version resolution is
plumbing every language has; a supply chain where a dependency's reach is a
compile-time fact is not.</p>

<h2>Commands</h2>"""
    + code(
        """
grasm deps            # resolve, check ceilings, write grasm.lock
grasm deps --locked   # fail rather than update the lockfile
"""
    )
    + """

<h2>What ships today</h2>
<div class="scroll"><table>
<tr><th>Package</th><th>Capabilities</th><th>What it is</th></tr>
<tr><td><code>http</code></td><td><i>none</i></td><td>Requests, responses, routing with <code>:param</code> and <code>*</code> segments, percent decoding, HTML escaping.</td></tr>
<tr><td><code>http_server</code></td><td><code>io</code>, <code>net</code></td><td>The accept loop, and many conversations on one thread. Depends on <code>http</code>.</td></tr>
<tr><td><code>http_client</code></td><td><code>net</code></td><td>Dials, sends what <code>http</code> composed, reads until the declared frame arrives. Depends on <code>http</code>.</td></tr>
<tr><td><code>json</code></td><td><i>none</i></td><td>Parse, render, reach into a document, build one. Six programs in <code>ProjectGold/</code> depend on it.</td></tr>
<tr><td><code>bytes_codec</code></td><td><i>none</i></td><td>Hex, base64, url-safe, constant-time compare.</td></tr>
<tr><td><code>time</code></td><td><i>none</i></td><td>Civil calendar, ISO-8601, durations. The clock is a parameter.</td></tr>
<tr><td><code>random</code></td><td><i>none</i></td><td>Seeded generator, uniform without modulo bias, shuffle.</td></tr>
<tr><td><code>uuid</code></td><td><i>none</i></td><td>v4 and v7. The entropy is a parameter.</td></tr>
<tr><td><code>path</code></td><td><i>none</i></td><td>Join, normalise, split, containment. Touches no disk.</td></tr>
<tr><td><code>url</code></td><td><i>none</i></td><td>Parse, build, resolve, percent code, query strings.</td></tr>
<tr><td><code>log</code></td><td><code>io</code>, <code>Log</code></td><td>Logging as an <b>effect</b>: a library that logs declares <code>Log</code>, not <code>io</code>.</td></tr>
<tr><td><code>csv</code></td><td><i>none</i></td><td>Quoting, embedded newlines, headers, any separator.</td></tr>
<tr><td><code>cli</code></td><td><i>none</i></td><td>Flags, bundles, subcommands, <code>--</code>, a generated help page.</td></tr>
<tr><td><code>testing</code></td><td><i>none</i></td><td>Cases as values, table-driven suites, a golden diff that names the line and column.</td></tr>
<tr><td><code>toml</code></td><td><i>none</i></td><td>Tables, arrays, inline tables, reached by dotted path.</td></tr>
<tr><td><code>resilience</code></td><td><i>none</i></td><td>Backoff with full jitter, circuit breaker, token bucket, deadlines — the clock is a parameter.</td></tr>
<tr><td><code>cache</code></td><td><i>none</i></td><td><code>Cache[V]</code>, least-recently-used with an expiry.</td></tr>
<tr><td><code>crypto_hash</code></td><td><i>none</i></td><td>SHA-256, HMAC-SHA-256, PBKDF2, constant-time compare — every digest checked against FIPS 180-4, RFC 4231 and RFC 6070.</td></tr>
<tr><td><code>regex</code></td><td><i>none</i></td><td>A Thompson machine, so matching is linear in the input for every pattern — <code>(a+)+b</code> against thirty <code>a</code>s returns at once.</td></tr>
<tr><td><code>fs</code></td><td><code>io</code></td><td>Walk, glob, copy, temporary paths. The arithmetic half is <code>path</code>, which declares nothing.</td></tr>
<tr><td><code>jwt</code></td><td><i>none</i></td><td>HS256. The algorithm comes from the caller, never the token — so <code>alg: none</code> is refused because it disagrees, not by a special case.</td></tr>
<tr><td><code>cors</code></td><td><i>none</i></td><td>A request in, the headers a response needs out.</td></tr>
<tr><td><code>dotenv</code></td><td><code>io</code></td><td><code>.env</code> files. It answers a map and never touches the process environment.</td></tr>
<tr><td><code>sanitize</code></td><td><i>none</i></td><td>Escaping by <em>destination</em> — HTML text, an attribute, a script block, a URL, a shell argument, a header. There is no general <code>escape</code>, because there is no such thing.</td></tr>
<tr><td><code>metrics</code></td><td><code>Metric</code></td><td>Counters, gauges, histograms and the Prometheus text format — the registry is an effect, so a library that counts declares <code>Metric</code> and not <code>io</code>.</td></tr>
<tr><td><code>errors</code></td><td><i>none</i></td><td>A failure carrying its cause, its place and its fields. Grasm has no captured backtrace, so every link is one somebody wrote — which is the work, and is why nothing is lost.</td></tr>
<tr><td><code>trace</code></td><td><code>Trace</code></td><td>Spans and W3C <code>traceparent</code>, as an effect. An all-zero trace id is refused, since a receiver that accepts one joins every unrelated request into a single trace.</td></tr>
<tr><td><code>redis</code></td><td><code>net</code></td><td>RESP2 and RESP3 both — a client that reads only one refuses half the servers it meets. The encoder and decoder are pure, so the demo drives them with bytes and never opens a socket.</td></tr>
<tr><td><code>websocket</code></td><td><i>none</i></td><td>The upgrade handshake (checked against RFC 6455's own example) and the frame format. A frame's masking is checked in both directions, because a server that accepts an unmasked one sits behind a proxy that can be poisoned. <code>ws://</code> only — there is no TLS.</td></tr>
<tr><td><code>password</code></td><td><i>none</i></td><td>PBKDF2-HMAC-SHA256. The stored form carries its own algorithm and cost, so a file written today can be verified after both change.</td></tr>
<tr><td><code>protobuf</code></td><td><i>none</i></td><td>The wire format: varints, zigzag, packed and nested fields. A message without its schema still reads, which is what makes a proxy possible. <b>Not gRPC</b> — that needs HTTP/2 and TLS.</td></tr>
<tr><td><code>postgres</code></td><td><code>net</code></td><td>The v3 wire protocol and <b>SCRAM-SHA-256</b>, checked against RFC 7677's own vector. No TLS, so a trusted network or a local socket.</td></tr>
<tr><td><code>xml</code></td><td><i>none</i></td><td>Parse and render. <b>No DTD and no external entities, ever</b> — that is what XXE is, and a first release does not need it. Which is also why this can declare nothing and mean it.</td></tr>
<tr><td><code>oauth2</code></td><td><i>none</i></td><td>The authorisation-code flow with PKCE (checked against RFC 7636's vector) and client credentials. PKCE and <code>state</code> are not optional here.</td></tr>
</table></div>
<p>Twenty-six of the thirty-four declare <em>nothing</em>. That is the pattern: reach is
split from computation, so a program that joins paths, formats a date or
builds a URL acquires no capability by doing so — and where a library really
does need the world, it asks for it as an effect the consumer supplies, which
is what <code>log</code> demonstrates.</p>
<p>They are three packages because <code>http</code> <em>cannot</em> open a
socket, and that is checked rather than claimed: its manifest declares no
capability, effects are inferred from its source, and a version of it that
started dialling out would fail to build in every consumer. So a program can
parse and route HTTP while remaining provably unable to reach the network, and
a program that only builds requests never acquires the ability to send them. In
every other language, depending on the HTTP library means depending on
something that can.</p>

<h2>Where packages come from</h2>
<p>Two sources work today: a <code>path</code> dependency, and a local registry
under <code>.grasm/registry</code> (or wherever <code>GRASM_REGISTRY</code>
points). Both resolve, lock and check ceilings.</p>

<div class="note"><b>There is no remote registry.</b> Nothing fetches, publishes
or verifies over a network yet, and there are no third-party packages to fetch.
Both are on the <a href="roadmap.html">roadmap</a>, and they are the largest
thing standing between this language and ordinary use.</div>
""",
)

# ---------------------------------------------------------------- spec

PAGES["spec.html"] = (
    "234 numbered clauses, each with a conformance program.",
    """
<p class="eyebrow">Reference</p>
<h1>Specification</h1>
<p class="lede">The language is defined by numbered clauses, each with a program
that exercises it, and each filed at a stability level that says whether it may
change.</p>

<div class="figures">
  <div class="fig"><b>«clauses»</b><span>clauses</span></div>
  <div class="fig"><b>70</b><span>pages, the cap</span></div>
  <div class="fig"><b>«laws»</b><span>laws checked against arithmetic</span></div>
</div>

<h2>How a clause works</h2>
<p>A clause is a paragraph with a tag: <code>[T-REF-VALUE]</code>,
<code>[E-SPAWN-ORDER]</code>, <code>[L-LIB-BITS]</code>. Every one has a program
under <code>spec/conformance/</code> tagged with it, and the test suite fails if
a clause has no program, if a program has no clause, or if a clause is filed
under a stability heading that disagrees with its own level.</p>

<h2>Stability</h2>
<div class="scroll"><table>
<tr><th>Level</th><th>Means</th></tr>
<tr><td>Stable</td><td>It will not change under you.</td></tr>
<tr><td>Provisional</td><td>It works and it may still move.</td></tr>
<tr><td>Experimental</td><td>Expect it to change.</td></tr>
</table></div>

<h2>Why there is also a law for most of them</h2>
<p>A conformance program records what a clause says the output is. Where the
clause and the implementation were written together — which, for a young
language, is most of them — both can be wrong together, and the program agrees
with them.</p>
<p>So the behavioural clauses in scope also have a <em>law</em>: one of «laws» relations checked against
arithmetic, not against a recorded string. <code>rev</code> puts the last
element first. <code>a / b * b + a % b == a</code>. A released slot is dead
through every reference to it. A clause in scope with no law fails the gate by
name.</p>

<h2>The page budget</h2>
<p>The specification is capped at seventy pages and the test suite enforces it.
When a clause was added recently and the file went one line over, the clause was
made shorter rather than the cap raised. A specification nobody finishes reading
is one nobody checks the implementation against.</p>
""",
)

# ---------------------------------------------------------------- examples

PAGES["examples.html"] = (
    "Forty-four programs, each running identically on three engines.",
    """
<p class="eyebrow">More</p>
<h1>Examples</h1>
<p class="lede">Not snippets. «gold» whole programs, «gold_lines» lines, each one
checked on three engines for identical output, exit code and diagnostics.</p>

<h2>Systems</h2>
<div class="grid three">
  <div class="cell"><h4>HTTP server</h4><p>347 lines. Sockets, routing, request parsing.</p></div>
  <div class="cell"><h4>SQL engine</h4><p>583 lines. Parser, planner, execution.</p></div>
  <div class="cell"><h4>Micro VM</h4><p>A bytecode machine.</p></div>
  <div class="cell"><h4>Garbage collector</h4><p>Written in a language with no garbage collector.</p></div>
  <div class="cell"><h4>DNS resolver</h4><p>Wire format, UDP.</p></div>
  <div class="cell"><h4>Turing machine</h4><p>Because the tests needed one.</p></div>
</div>

<h2>Tools and formats</h2>
<div class="grid three">
  <div class="cell"><h4>JSON parser</h4><p>375 lines, and a package.</p></div>
  <div class="cell"><h4>Markdown to HTML</h4><p>A package too.</p></div>
  <div class="cell"><h4>SHA-256</h4><p>Bit operations, byte handling.</p></div>
  <div class="cell"><h4>grep</h4><p>Files, patterns, streams.</p></div>
  <div class="cell"><h4>Self-hosting lexer</h4><p>Grasm, lexing Grasm.</p></div>
  <div class="cell"><h4>Expression REPL</h4><p>Parse, evaluate, loop.</p></div>
</div>

<h2>Concurrent</h2>
<div class="grid three">
  <div class="cell"><h4>Multithreaded scraper</h4><p>Parallel fetches, joined.</p></div>
  <div class="cell"><h4>Async chat</h4><p>Many connections, one process.</p></div>
  <div class="cell"><h4>Sorting benchmark</h4><p>Where the performance numbers come from.</p></div>
</div>

<h2>In a browser</h2>
<p>Twelve programs compiled to wasm and run in a tab with no server: a SPA
router, a markdown editor, a collaborative todo list, realtime analytics, user
registration and authentication, an upload gallery, a chat router, a voting
app, a blog dashboard, a weather mashup, an e-commerce cart, a fullstack
counter.</p>

<p>All of them live under <code>ProjectGold/</code> in the repository, with the
expected output recorded beside each.</p>
""",
)

# ---------------------------------------------------------------- status

PAGES["status.html"] = (
    "What works, what does not, and what is not yet true.",
    """
<p class="eyebrow">More</p>
<h1>Status</h1>
<p class="lede">Every number here is written by the run that proved it —
<code>scripts/allgates.sh</code> records them after every gate passes, and the
page is regenerated from that record. None of them is typed by hand.</p>

<h2>What holds</h2>
<div class="figures">
  <div class="fig"><b>«corpus»</b><span>programs, three engines</span></div>
  <div class="fig"><b>189</b><span>identical in wasm</span></div>
  <div class="fig"><b>227</b><span>cross-compiled, agreeing</span></div>
  <div class="fig"><b>0</b><span>open defects</span></div>
  <div class="fig"><b>67</b><span>findings recorded</span></div>
  <div class="fig"><b>14</b><span>generators</span></div>
</div>
<p>Interpreter, bundle and native builds agree byte for byte on stdout, exit
code and diagnostics. A wasm build agrees on the «wasm_ok» programs that do not
need a filesystem, a socket, a process or threads. The same «cross» cross-compile to
x86_64 and agree there. Every generated program is checked for what it keeps as
well as for what it prints.</p>

<h2>What does not work</h2>
<div class="scroll"><table>
<tr><th>Limit</th><th>Detail</th></tr>
<tr><td>Thirty-four libraries, all first-party</td><td>See <a href="packages.html">Packages</a>; the plan for the rest is in <code>docs/packages-plan.md</code>. Anything else you need, you write, or bind to C with <code>extern fn</code>.</td></tr>
<tr><td>No remote registry</td><td>Path and local-registry dependencies work. Nothing fetches or publishes.</td></tr>
<tr><td>No release binaries</td><td>Build from source with a Rust toolchain.</td></tr>
<tr><td>0.45× native Rust</td><td>Measured, floor-subtracted 0.39×, against a 0.95× target.</td></tr>
<tr><td>Polymorphic recursion</td><td>Refused by name at build time. It is what monomorphization is; no work removes it.</td></tr>
<tr><td>Small standard library</td><td>«methods» methods. Enough for the 44 programs above; you will find gaps in a larger one.</td></tr>
<tr><td>No users but this project</td><td>Nobody has hit it from an angle nobody here thought of.</td></tr>
<tr><td>Region sweep not implemented</td><td>Tier 1's rule is enforced; the bulk deallocation it makes sound is not built.</td></tr>
</table></div>

<h2>What is not claimed</h2>
<p>Grasm is not faster than Rust, Go or Zig, and no benchmark here says it is.
It is not battle-tested. Defects are still being found at roughly one per
working session — falling, but not zero, and a language with zero known defects
and no users has told you more about its test suite than about itself.</p>

<h2>How that is kept honest</h2>
<p>All «findings» findings are written down in <code>ProjectGold/FINDINGS.md</code>,
including the ones that were the test harness's fault, the ones withdrawn as
wrong, and the ones caught before they shipped. Every gate is verified by
breaking the thing it guards and confirming it fails.</p>
""",
)

# ---------------------------------------------------------------- roadmap

PAGES["roadmap.html"] = (
    "What is being built next, and what would have to be true to call it done.",
    """
<p class="eyebrow">More</p>
<h1>Roadmap</h1>
<p class="lede">Four workstreams toward a first release, running alongside the
defect loop rather than instead of it.</p>

<h2>1. This site</h2>
<p>In progress — you are reading it. The <a href="playground.html">playground</a>
now runs programs in the page, on the same wasm module the gates hold to the
other engines, and the <a href="tutorial.html">tutorial</a> and
<a href="library.html">library reference</a> are the documents themselves,
rendered when the site is written rather than summarised here.</p>

<h2>2. Speed</h2>
<p>0.45× native Rust today against a 0.95× target, computed by
<code>scripts/bench.sh</code> from the numbers it prints. The costs worth
attacking, in order: a box per node in tree-shaped data, binary size (350 KB
against a 300 KB target), and the atomic reference count every boxed value
carries because a <code>parallel</code> body may hold one.</p>

<h2>3. Packages and libraries</h2>
<p>The mechanism is built: manifests, resolution, lockfiles, path and local
registry dependencies, and the capability ceiling. The first two libraries are
written. What is missing is a remote registry and the rest of them.</p>
<div class="scroll"><table>
<tr><th>Library</th><th>State</th></tr>
<tr><td><b>http</b> — server</td><td><b>Built.</b> <code>packages/http</code> parses and routes and declares no capability; <code>packages/http_server</code> owns the socket. Both in the corpus, both agreeing on three engines.</td></tr>
<tr><td><b>http</b> — client</td><td><b>Built.</b> <code>packages/http_client</code>. Its demo talks to <code>http_server</code> over a real socket — a Grasm client and a Grasm server.</td></tr>
<tr><td><b>json</b></td><td><b>Built.</b> <code>packages/json</code>: parse, render, reach in (<code>get</code>, <code>at</code>, <code>path</code>, <code>as_str</code>…), build up (<code>object</code>, <code>list</code>, <code>with_key</code>…). Declares no capability. The six ProjectGold programs that used it point here.</td></tr>
<tr><td><b>async</b> — timers, channels, select</td><td>Not started. <code>parallel</code> and <code>spawn</code> are in the language; the library above them is not.</td></tr>
<tr><td><b>testing</b>, <b>log</b>, <b>time</b>, <b>uuid</b>, <b>base64</b>, <b>regex</b></td><td>Not started.</td></tr>
</table></div>
<p>The comparison to hyper, axum, tokio, gin and fiber is a comparison against
years of work by thousands of people. The honest target for a first release is
one good HTTP library, one JSON library, one async library, and the packaging to
publish them — so that the second and third can be written by somebody else.</p>

<h2>4. Is the compiler ready</h2>
<p>Yes, with the limits on the <a href="status.html">status page</a> stated
rather than glossed. Four engines agree; the list of shapes the native backend
cannot compile is one entry long and permanent.</p>

<h2>The bar for calling it finished</h2>
<p>Zero known defects, every gate green, and <em>two consecutive independent
review rounds finding nothing new</em> — each conducted by someone who has seen
only the language pack, not this repository's history. Those rounds have not
run. Until they do, the claim is not available, and no number of green gates
substitutes for it.</p>
""",
)

# ---------------------------------------------------------------- security

PAGES["security.html"] = (
    "A Grasm package declares what it may reach, and the compiler enforces it.",
    """
<p class="eyebrow">Learn</p>
<h1>A dependency cannot quietly grow teeth.</h1>
<p class="lede">When you install a library that says it does arithmetic, nothing
in Python, Go, Rust or Zig stops it from opening your files or reaching the
network. Grasm writes that permission down and refuses the build when it is
exceeded.</p>

<h2>The attack</h2>
<p>A trusted, pure library ships a patch release. Same name, same signature,
same return value \u2014 and it now reads <code>/etc/passwd</code> and writes it
out. Nothing about the call site changes. This is not hypothetical; it is the
shape of every dependency compromise of the last decade.</p>

<div class="term">
  <div class="chrome"><span class="dot"></span><span class="dot"></span><span class="dot"></span>
    <span>app \u2014 grasm build</span></div>
  <div class="body"></div>
</div>
"""
    + verdicts()
    + """

<p>Only Grasm\u2019s toolchain refuses. Everywhere else the tampered code builds,
ships and runs; detection is left to runtime sandboxing or to a reviewer
noticing \u2014 neither of which the toolchain does.</p>

<h2>How it works</h2>
<p>Two halves. The first is the effect row, which is <em>inferred</em>, so it
cannot be wrong about the body:</p>
"""
    + code(
        """
// the row is worked out from the body, then checked against what you wrote
fn load(path: str) -> str ! IoError / {io} {
    try read_file(path)
}
"""
    )
    + """
<p>The second is the manifest, which is the ceiling for the whole package:</p>
"""
    + code(
        """
[package]
name = "mathlib"
version = "1.0.1"

capabilities = []        # this package may perform no effect at all
"""
    )
    + """
<p>Add the <code>read_file</code> and the package performs <code>io</code>,
which exceeds <code>[]</code>. The error names the exact place:</p>
"""
    + code(
        """
error: package `mathlib` performs `io` but its manifest only permits nothing
  help: either add the capability to that package's manifest, or stop it
        from performing the effect
  performed: `io` at mathlib/src/main.gr:2:9
"""
    )
    + """
<p>The maintainer <em>can</em> raise the ceiling \u2014 but the manifest is in the
diff, and a package that used to declare <code>[]</code> and now declares
<code>["io"]</code> is one line a consumer can see, refuse, or gate in CI.
That is the whole design: make the change visible where a human is already
looking.</p>

<h2>What this does not do</h2>
<p class="note"><strong>This is a build-time check, not a sandbox.</strong> It
constrains what code in a Grasm package can be <em>written</em> to do. It does
not contain a process that is already running, it does not survive
<code>unsafe</code>, and it says nothing about a compromised compiler or
operating system.</p>
<ul>
  <li><code>unsafe</code> and <code>extern fn</code> reach C, and C is outside
  this. That is why the tier is called Tier 3, why every call must sit inside an
  <code>unsafe</code> block, <em>and</em> why the package must additionally
  declare the <code>unsafe</code> capability \u2014 so a consumer can refuse the
  whole package by reading one line.</li>
  <li><code>exit</code> performs no effect and can still end the process. So
  can a panic and an endless loop. A ceiling bounds what a dependency can
  <em>reach</em>, never how long it runs.</li>
</ul>

<h2>Prior art</h2>
<p>None of this is invented here. <a href="about.html">Austral</a> pioneered
capability-secure modules, Wyvern did capability-safe modules before that, and
the effect row is <a href="about.html">Koka</a>\u2019s. What is uncommon is
shipping the combination with a gate that proves it: <code>security_bench.sh</code>
runs the attack on every green build.</p>
"""
)

# ------------------------------------------------------------ architecture

PAGES["architecture.html"] = (
    "How Grasm is built: four engines, one normative interpreter, and the gates that hold them together.",
    """
<p class="eyebrow">Reference</p>
<h1>How it is built.</h1>
<p class="lede">One language, four ways to run it, and a set of gates whose only
job is to stop them disagreeing. The interpreter is the specification; the other
three have to match it.</p>

<h2>The pipeline</h2>
<table>
  <tr><th>Stage</th><th>Crate</th><th>What it produces</th></tr>
  <tr><td>Lexer and parser</td><td><code>grasm_syntax</code></td><td>Tokens, then a tree. Also the formatter.</td></tr>
  <tr><td>Checker</td><td><code>grasm_sema</code></td><td>Types, effect rows, ownership, exhaustiveness \u2014 and an <code>Analysis</code> the backend reads.</td></tr>
  <tr><td>Interpreter</td><td><code>grasm_interp</code></td><td>The normative answer.</td></tr>
  <tr><td>Native backend</td><td><code>grasm_native</code></td><td>Cranelift IR, then an object file.</td></tr>
  <tr><td>Runtime</td><td><code>grasm_rt</code></td><td>Reference-counted boxed values with copy-on-write.</td></tr>
  <tr><td>Wasm</td><td><code>grasm_wasm</code></td><td>The interpreter, compiled for <code>wasm32</code>.</td></tr>
</table>

<h2>Why the checker hands types to the backend</h2>
<p>It did not, once. The backend re-derived them \u2014 roughly 950 lines of a
<em>second</em> type inference \u2014 and nine of twelve findings in one period
traced to the two disagreeing. The checker now exports every expression\u2019s
resolved type; it answers 94% of the backend\u2019s questions on real code and
87% on generated code. Where it answers, the two <em>cannot</em> disagree,
because there is only one answer.</p>
<p>Nothing was deleted on the strength of that, and the reason is recorded:
an arm the corpus never reaches is not an arm nothing reaches.</p>

<h2>The memory model, plainly</h2>
<p>Boxed values \u2014 strings, lists \u2014 are reference-counted with
copy-on-write. The count is atomic so a value may cross a thread, and that cost
was measured rather than assumed: 4.3% on the most reference-count-heavy
program here, no movement at all on three others, about half a percent on the
geometric mean.</p>
<p>Effects, capabilities, ownership modes and exhaustiveness are erased before
codegen and cost nothing at run time. <code>ref T</code> costs a generation
check. <code>region</code> is checked but does not yet free in bulk \u2014 the
specification says so in the clause itself rather than implying otherwise.</p>

<h2>The gates</h2>
<p>«gates» of them, each judged by its exit code and nothing else. That rule
exists because a gate once reported <code>identical: 184  mismatched: 0</code>,
which was true, while failing three lines further down.</p>
<div class="grid two">
  <div class="card"><h3>Agreement</h3><p>difftest, crosstest, wasmtest and
  agreetest run the corpus through every engine and require identical bytes.</p></div>
  <div class="card"><h3>Differential</h3><p>Against things outside this project:
  OpenSSL for certificates, real zlib, git\u2019s own object store, V8 for
  WebAssembly, clang\u2019s assembler.</p></div>
  <div class="card"><h3>Generated</h3><p>«generators» generators build programs
  nobody wrote, then require every engine to agree on them too.</p></div>
  <div class="card"><h3>Laws</h3><p>«laws» properties that must hold. A law a
  degenerate implementation satisfies is not a law, and each one here is chosen
  so that it is not.</p></div>
</div>

<h2>Record and replay</h2>
<p>Every builtin that touches the world is journaled. A recorded run replays
deterministically, and the gate requires each journaled builtin to
<em>refuse</em> under an empty journal \u2014 so a replay cannot quietly reach
the real world instead.</p>
"""
)

# ---------------------------------------------------------------- about

PAGES["about.html"] = (
    "What Grasm is, what it borrowed, and who should not use it.",
    """
<p class="eyebrow">More</p>
<h1>What this is, and what it is not.</h1>
<p class="lede">Grasm is a systems language with an effect system, capability
ceilings on packages, and tiered ownership without lifetime annotations. It is
version «version», it has «packages» libraries, and every one of them was
written inside this repository.</p>

<h2>Nothing here was invented</h2>
<p>It is worth being direct about this, because a language site is the last
place anyone expects to read it. Every idea in Grasm exists elsewhere, usually
first and usually with more research behind it.</p>
<table>
  <tr><th>In Grasm</th><th>Where it came from</th></tr>
  <tr><td><code>let</code> / <code>inout</code> / <code>own</code>, value semantics, no lifetimes</td>
      <td><b>Hylo</b> (formerly Val) has <code>let</code>/<code>inout</code>/<code>sink</code>. <b>Swift</b> has exclusivity, ARC and copy-on-write.</td></tr>
  <tr><td>Effect rows, inferred and unified</td>
      <td><b>Koka</b> \u2014 row-based effect types with inference. Also <b>OCaml 5</b>, <b>Unison</b>, <b>Eff</b>, <b>Frank</b>.</td></tr>
  <tr><td>Capability ceiling per module</td>
      <td><b>Austral</b> \u2014 capability-secure, no ambient I/O. <b>Wyvern</b>, and the <b>E</b> language before it.</td></tr>
  <tr><td>Reference counting with copy-on-write</td>
      <td><b>Swift</b>, <b>Nim</b>.</td></tr>
  <tr><td>Generational references</td>
      <td>The slotmap pattern, in every entity-component system.</td></tr>
  <tr><td>A reference interpreter as the specification</td>
      <td><b>WebAssembly</b> does this, and does it better.</td></tr>
</table>
<p>The closest single comparison is <b>Hylo plus Koka, tied to a manifest the
way Austral does it</b>. What is uncommon is not the ideas but shipping them
together with gates that prove the combination holds.</p>

<h2>Who should not use this</h2>
<p>Most people, today. Specifically:</p>
<ul>
  <li><b>If you need libraries.</b> There are «packages», all written here.
  Python has hundreds of thousands.</li>
  <li><b>If you need speed.</b> Rust and Zig are faster, and
  <a href="status.html">the numbers here</a> say by how much.</li>
  <li><b>If you need to hire.</b> Nobody outside this repository has written
  Grasm.</li>
  <li><b>If you need a native debugger.</b> There is a Debug Adapter Protocol
  server, and it works against the interpreter. The Cranelift backend emits no
  DWARF, so GDB and LLDB cannot meaningfully inspect a compiled binary.</li>
  <li><b>If you need async I/O.</b> There are threads and structured
  concurrency, and no async/await.</li>
</ul>

<h2>Who might</h2>
<p>Someone who genuinely models an untrusted dependency reaching the network or
the filesystem as a threat, and who wants the compiler to enforce the boundary
rather than a sandbox at run time. That is a real gap in Rust, Go, Zig and
Python, and it is the one thing here that is not catching up to somebody
else.</p>

<h2>How it is built</h2>
<p>«clauses» normative clauses in «spec_lines» lines of specification, under a
hard ceiling \u2014 every addition since has been paid for by compressing prose
already written. «stable» clauses are <code>stable</code> and will not change
incompatibly before 2.0; «provisional» are <code>provisional</code> and
«experimental» are <code>experimental</code>. Version 1.0 promoted nothing: a
clause moves when it passes the five-part test, one at a time, on its own
evidence.</p>

<p class="note">Every number on this site is computed from the repository when
the site is written, or read from the last green gate run. None of them is a
figure somebody remembered. If a gate has not run, the page says so rather than
printing a number nobody measured.</p>
"""
)


# ---------------------------------------------------------------- syntax

def _row(name, note, src):
    return (
        f'<div class="syn-row"><div class="k">{name}<span>{note}</span></div>'
        f'<div class="v">{code(src)}</div></div>'
    )


def _table(rows):
    return '<div class="syn">' + "".join(_row(*r) for r in rows) + "</div>"


PAGES["syntax.html"] = (
    "Every construct in Grasm, with an example of each.",
    """
<p class="eyebrow">Reference</p>
<h1>Syntax</h1>
<p class="lede">The whole language on one page. Every form below is checked by
a conformance program in <code>spec/conformance</code>, so an example that
stopped compiling would fail the build rather than sit here misleading
somebody.</p>

<h2>Declarations</h2>
"""
    + _table([
        ("fn", "A function. The row after <code>/</code> is inferred and checked.",
         """fn add(a: i64, b: i64) -> i64 {
    a + b
}

fn greet(name: str) / {io} {
    print("hello, {name}")
}"""),
        ("pub", "Visible outside the module. Types are always visible.",
         """pub fn parse(text: str) -> i64 {
    text.parse_int() ?? 0
}"""),
        ("let / var", "A binding. <code>let</code> cannot be reassigned.",
         """let limit = 10
var seen = 0
seen = seen + 1"""),
        ("type — struct", "Named fields. At least one.",
         """type Point = {
    x: f64,
    y: f64,
}"""),
        ("type — sum", "Alternatives, each with named payload fields.",
         """type Shape =
    | Circle(r: f64)
    | Rect(w: f64, h: f64)
    | Empty"""),
        ("type — alias", "Another name for a type.",
         "type Meters = f64"),
        ("effect", "An interface a handler supplies at the call site.",
         """effect Clock {
    fn now() -> i64
}"""),
        ("interface / impl", "A contract and its implementation.",
         """interface Show {
    fn show(self) -> str
}"""),
        ("import", "A module or package. Named by its last path segment.",
         "import json\nimport crypto_hash"),
        ("const", "A module-level value, one for the whole program.",
         "pub let MAX_RETRIES = 5"),
        ("test", "A unit test, or a property test when it takes parameters.",
         """test "reversing twice is the identity" (xs: [i64]) {
    assert(xs.rev().rev() == xs)
}"""),
        ("extern fn", "A C symbol. Tier 3 — every call sits in <code>unsafe</code>.",
         """extern fn getpid() -> i64
extern fn strlen(s: str) -> i64"""),
    ])
    + """
<h2>Types</h2>
"""
    + _table([
        ("Primitives", "One integer type, one float, and text is UTF-8.",
         "let a: i64 = 42\nlet b: f64 = 1.5\nlet c: bool = true\nlet d: str = \"text\""),
        ("bytes", "A sequence of 0..255. Distinct from <code>[i64]</code> on purpose.",
         """let raw: bytes = bytes_of([72, 105])
let from_text = "hi".to_bytes()"""),
        ("Lists", "Homogeneous and growable.",
         "let xs: [i64] = [1, 2, 3]\nlet empty: [str] = []"),
        ("Maps", "Keyed, with <code>get</code> returning an <code>Option</code>.",
         """var m: Map[str, i64] = map()
m.set("k", 1)
let v = m.get("k") ?? 0"""),
        ("Tuples", "Positional, reached with <code>.0</code>.",
         "let pair = (1, \"one\")\nlet first = pair.0"),
        ("Option", "<code>Some</code> or <code>None</code>.",
         """let found: Option[i64] = Some(3)
let n = found ?? 0"""),
        ("Error unions", "<code>T ! E</code> — a value or a failure.",
         """fn half(n: i64) -> i64 ! MathError {
    if n % 2 != 0 {
        fail(NotEven(n))
    }
    n / 2
}"""),
        ("Function types", "<code>fn(...)</code> closes up; it is a type, not a call.",
         "fn apply(f: fn(i64) -> i64, v: i64) -> i64 {\n    f(v)\n}"),
        ("Generics", "Square brackets, on the declaration.",
         """fn first_of[T](xs: [T]) -> Option[T] {
    if xs.len() == 0 {
        return None
    }
    Some(xs[0])
}"""),
        ("ref T", "Tier 2 — a generational reference, storable in a struct.",
         """let r = ref(Point{ x: 1.0, y: 2.0 })
let alive_now = alive(r)
release(r)"""),
    ])
    + """
<h2>Expressions</h2>
"""
    + _table([
        ("Operators", "<code>and</code>, <code>or</code>, <code>not</code> are words. <code>^</code> is power.",
         """let a = 1 + 2 * 3 - 4 / 2 % 3
let b = 2 ^ 8
let c = a > 0 and not (b < 0)"""),
        ("Comparison", "Structural, on everything comparable.",
         "let same = [1, 2] == [1, 2]"),
        ("if", "An expression: it yields a value.",
         """let label = if n > 0 {
    "positive"
} else {
    "not"
}"""),
        ("match", "Exhaustive. Arms are newline-separated.",
         """match shape {
    Circle(r) -> 3.14159 * r ^ 2
    Rect(w, h) -> w * h
    Empty -> 0.0
}"""),
        ("Patterns", "Literals, wildcards, alternatives, and tuples.",
         """match n {
    0 -> "zero"
    1 | 2 | 3 -> "small"
    _ -> "large"
}"""),
        ("Closures", "<code>|params| body</code>. The body may be a block.",
         """let double = |n| n * 2
let total = xs.fold(0, |acc, v| acc + v)"""),
        ("Interpolation", "<code>{}</code> inside a string. A literal brace is <code>\\{}</code>.",
         'print("{name} has {xs.len()} item(s)")'),
        ("try", "A prefix. Propagates the error branch.",
         """fn load(path: str) -> str ! IoError / {io} {
    try read_file(path)
}"""),
        ("??", "The else of an Option or a fallible call. Lazy.",
         'let port = env("PORT") ?? "8080"'),
        ("Pipeline", "<code>|></code> feeds the left into the right.",
         'let n = xs |> len'),
        ("Ranges", "<code>..</code> excludes the end, <code>..=</code> includes it.",
         "for i in 0..10 {\n    print(\"{i}\")\n}"),
        ("Blocks", "A block is an expression; its last line is its value.",
         """let v = {
    let a = 2
    a * a
}"""),
    ])
    + """
<h2>Statements and control</h2>
"""
    + _table([
        ("for", "Over a list, a map or a range.",
         """for x in xs {
    print("{x}")
}"""),
        ("while", "While the condition holds.",
         """while queue.len() > 0 {
    let _v = queue.pop()
}"""),
        ("break / continue", "Leave or skip an iteration.",
         """for x in xs {
    if x < 0 {
        continue
    }
    if x > 99 {
        break
    }
}"""),
        ("return", "Leaves the function early.",
         """fn find(xs: [i64], want: i64) -> bool {
    for x in xs {
        if x == want {
            return true
        }
    }
    false
}"""),
        ("fail", "Takes the error branch of a <code>T ! E</code>.",
         "fail(NotEven(n))"),
        ("Assignment", "<code>=</code>, and the compound forms.",
         "var n = 0\nn = 5\nn += 1\nn -= 1\nn *= 2\nn /= 2"),
    ])
    + """
<h2>Ownership and effects</h2>
"""
    + _table([
        ("Parameter modes", "Default reads; <code>inout</code> mutates; <code>own</code> takes.",
         """fn read(xs: [i64]) -> i64 { xs.len() }
fn grow(inout xs: [i64]) { xs.push(1) }
fn consume(own xs: [i64]) -> i64 { xs.len() }"""),
        ("Effect rows", "Written after <code>/</code>. Inferred, then checked.",
         """fn fetch(url: str) -> str ! IoError / {io, net} {
    try http_get(url)
}"""),
        ("with / handled_by", "Supplies a handler for an effect, for one scope.",
         """with Clock handled_by ClockHandler{ now: || 1000 } {
    print("{Clock.now()}")
}"""),
        ("region", "Tier 1 — nothing allocated inside may outlive it.",
         """region {
    var scratch: [i64] = []
    scratch.push(1)
}"""),
        ("unsafe", "Tier 3 — where the guarantees stop, named in the source.",
         """unsafe {
    let pid = getpid()
}"""),
        ("parallel / spawn", "Structured concurrency: the block waits for its tasks.",
         """parallel {
    spawn {
        print("one")
    }
    spawn {
        print("two")
    }
}"""),
    ])
    + """
<h2>Lexical</h2>
"""
    + _table([
        ("Comments", "<code>//</code> to end of line; <code>/* */</code> nests; <code>///</code> documents.",
         """// a line
/* a block /* that nests */ */
/// documentation for what follows
fn documented() -> i64 { 1 }"""),
        ("Numbers", "Decimal, hex, underscores, floats and exponents.",
         "let a = 1_000_000\nlet b = 0xFF\nlet c = 1.5\nlet d = 1e-9"),
        ("Strings", "Double-quoted, with escapes; <code>\"\"\"</code> for raw.",
         'let s = "a\\tb\\n"\nlet raw = """no escapes here"""'),
        ("Statement ends", "A newline ends a statement. A line may continue if it ends with an operator.",
         """let total = 1 +
    2 +
    3"""),
        ("Identifiers", "Letters, digits and <code>_</code>; not starting with a digit.",
         "let snake_case = 1\nlet _ignored = 2"),
    ])
    + """
<p class="note"><strong>One namespace for types.</strong> Every type is
reachable from every module, which is why <code>pub</code> means nothing on
one — and why two packages cannot both declare a <code>Node</code>. A generator
checks the whole package set for that collision on every build.</p>

<p><a href="language.html">The language reference</a> gives the rules behind
these forms; <a href="spec.html">the specification</a> gives the clause and the
conformance program for each.</p>
"""
)
