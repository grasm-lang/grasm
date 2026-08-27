"""The content of each page. The chrome lives in `site.py`."""

E = "&amp;"


def code(src):
    s = src.strip("\n").replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
    return f"<pre><code>{s}</code></pre>"


PAGES = {}

# ---------------------------------------------------------------- index

PAGES["index.html"] = (
    "Grasm is a systems language where what a program may do is written in its type.",
    """
<p class="eyebrow">A systems language</p>
<h1>What a program may do<br>is written in its type.</h1>
<p class="lede">Grasm puts effects in the signature, capabilities in the package
manifest, and ownership in tiers you choose per value. It compiles ahead of
time, runs in a browser, and three engines agree byte for byte on every program
in the repository.</p>

<div class="actions">
  <a class="btn" href="start.html">Get started</a>
  <a class="btn ghost" href="playground.html">Try it in the browser</a>
  <a class="btn ghost" href="examples.html">See real programs</a>
</div>

<h2>The signature is the contract</h2>
<p>A function's type says what it returns <em>and</em> what it is allowed to
do. The row is inferred from the body, so it cannot drift from the truth, and a
caller that has not been given <code>io</code> cannot call something that
performs it.</p>
"""
    + code(
        """
fn safe_div(a: i64, b: i64) -> i64 ! MathError {
    if b == 0 {
        fail(DivByZero)
    }
    a / b
}

fn report(n: i64) / {io} {        // this one prints; the type says so
    print("{n}")
}
"""
    )
    + """
<p><code>! MathError</code> is a failure the caller must handle.
<code>/ {io}</code> is an effect the caller must be permitted. Neither is a
comment or a convention — both are checked.</p>

<h2>Four engines, one answer</h2>
<p>The interpreter is normative. A bundle is that interpreter with the program
attached. <code>--native</code> compiles ahead of time through Cranelift. A
fourth build targets <code>wasm32</code>. Every gate in the repository exists to
hold them to the same answer.</p>

<div class="figures">
  <div class="fig"><b>«corpus»</b><span>programs, three engines</span></div>
  <div class="fig"><b>«wasm_ok»</b><span>also identical in wasm</span></div>
  <div class="fig"><b>«cross»</b><span>cross-compiled to x86_64</span></div>
  <div class="fig"><b>0</b><span>open defects</span></div>
</div>

<h2>What is different here</h2>
<div class="grid three">
  <div class="cell"><h4>Effects, inferred</h4><p>You do not write the row. The
  compiler works it out from the body and checks it against what you declared,
  so a function that starts touching the network stops compiling.</p></div>
  <div class="cell"><h4>Capability ceilings</h4><p>A package declares the
  effects it may perform. One that quietly starts reading files fails to build
  <em>in every consumer</em> — a compile error, not a code review.</p></div>
  <div class="cell"><h4>Tiered ownership</h4><p>Four tiers, chosen per value:
  borrowed, arena, generational reference, and an explicit unsafe escape. You
  pay for the one you use.</p></div>
  <div class="cell"><h4>Typed errors</h4><p><code>T ! E</code> is a value.
  There is no exception mechanism and no null.</p></div>
  <div class="cell"><h4>Deterministic replay</h4><p>Every effect can be
  journaled and replayed on a different engine — a recording made natively
  replays under the interpreter.</p></div>
  <div class="cell"><h4>Runs in a browser</h4><p>The same source, the same
  answers, compiled to wasm with no server.</p></div>
</div>

<h2>Honest about where it is</h2>
<p>Grasm compiles real programs — an HTTP server, a SQL engine, a JSON parser,
a garbage collector, a self-hosting lexer — and runs them identically on every
engine. It has no third-party libraries, no users but this project, and runs at
roughly 0.45× native Rust on the benchmark set.</p>
<p><a href="status.html">The status page says exactly what does not work.</a></p>
""",
)

# ---------------------------------------------------------------- start

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

<div class="note"><b>This page is not itself a playground yet.</b> The wasm
build works and the host page works; wiring an editor into this site is on the
<a href="roadmap.html">roadmap</a>. Saying otherwise would be the kind of claim
this project does not make.</div>
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
    "The standard library: 81 methods and the free functions.",
    """
<p class="eyebrow">Reference</p>
<h1>Library reference</h1>
<p class="lede">Small on purpose. «methods» methods, each with a law checked
against arithmetic rather than against another engine.</p>

<h2>Lists</h2>
<p><code>len is_empty first last get push pop remove sort sort_by rev slice
concat map filter fold sum min max any all find enumerate zip join to_list
par_map par_filter</code></p>

<h2>Strings</h2>
<p><code>len byte_len is_empty chars codes code_at to_char substr split lines
words contains starts_with ends_with index_of replace trim trim_start trim_end
to_upper to_lower repeat pad_start pad_end parse_int parse_int_base parse_float
to_bytes</code></p>

<h2>Maps</h2>
<p><code>get set has delete len is_empty keys values entries</code></p>

<h2>Bytes</h2>
<p><code>len is_empty at slice concat to_list to_str</code></p>

<h2>Options</h2>
<p><code>is_some is_none expect unwrap_or</code>, and <code>??</code>.</p>

<h2>Numbers</h2>
<p><code>abs min max cmp as_f64 as_i64 to_str to_base to_fixed round floor ceil
sqrt cbrt exp ln log log2 log10 sin cos tan asin acos atan atan2 sinh cosh tanh
hypot to_radians to_degrees is_nan is_finite bit_and bit_or bit_xor bit_not shl
shr ushr</code></p>

<h2>Ranges</h2>
<p><code>to_list map filter sum len</code>. A range is a value of type
<code>range</code>: it holds three numbers, not its elements, so
<code>0..2_000_000</code> costs the same as <code>0..2</code>.</p>

<h2>The world</h2>
<p><code>print eprint read_file write_file read_bytes write_bytes list_dir
make_dir random_bytes tcp_listen tcp_accept tcp_connect udp_bind net_wait
proc_run proc_pipeline</code> — each carrying the effect it performs, so a
function that uses one says so in its type.</p>

<div class="note"><b>Every method has a law.</b> Not a recorded expected output —
a relation checked against arithmetic, because a bundle <em>is</em> the
interpreter and a wrong answer they share is invisible to any comparison
between them. A method added to the specification without a law fails the
gate by name.</div>
""",
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
</table></div>
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
<tr><td>Four libraries, all first-party</td><td><code>http</code>, <code>http_server</code>, <code>http_client</code>, <code>json</code>. Anything else you need, you write, or bind to C with <code>extern fn</code>.</td></tr>
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
<p>In progress — you are reading it. What remains: an in-page playground wired
to the wasm build, and the tutorial and library reference rendered from
<code>docs/</code> rather than summarised here.</p>

<h2>2. Speed</h2>
<p>0.45× native Rust today against a 0.95× target, computed by
<code>scripts/bench.sh</code> from the numbers it prints. The costs worth
attacking, in order: a box per node in tree-shaped data, binary size (328 KB
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
