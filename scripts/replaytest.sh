#!/usr/bin/env bash
# Every builtin the table calls `Journal::Recorded` must actually be hooked,
# on every engine that has its own implementation of it.
#
# The check is behavioural rather than a grep for the hook. Under an *empty*
# journal a hooked call has nothing to hand back, so it must stop with exit 7
# and say the journal ran out. An unhooked call reaches the world instead and
# the program runs to completion — which is the exact defect this exists to
# catch, and the one a source scan would miss for the eleven socket operations
# that share a single gate keyed by a variable.
set -u
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
. "$(dirname "$0")/lib/fresh.sh"
fresh_or_stop "./target/release/grasm"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
printf 'GRASMJRNL2' > "$WORK/empty.jrnl"

# One call each, written so the checker accepts it: every fallible result is
# handled, because that is the language's rule and this is not exempt from it.
prog() {
  case "$1" in
    input)          echo 'print(input().len().to_str())' ;;
    read_file)      echo 'match read_file("/etc/hosts") { Ok(s) -> print("y") Err(e) -> print("n") }' ;;
    write_file)     echo 'match write_file("'"$WORK"'/o", "x") { Ok(u) -> print("y") Err(e) -> print("n") }' ;;
    read_bytes)     echo 'match read_bytes("/etc/hosts") { Ok(b) -> print("y") Err(e) -> print("n") }' ;;
    write_bytes)    echo 'match write_bytes("'"$WORK"'/o", bytes_of([1])) { Ok(u) -> print("y") Err(e) -> print("n") }' ;;
    file_exists)    echo 'if file_exists("/tmp") { print("y") } else { print("n") }' ;;
    remove_file)    echo 'match remove_file("'"$WORK"'/gone") { Ok(u) -> print("y") Err(e) -> print("n") }' ;;
    make_dir)       echo 'match make_dir("'"$WORK"'/d") { Ok(u) -> print("y") Err(e) -> print("n") }' ;;
    list_dir)       echo 'match list_dir("/tmp") { Ok(n) -> print("y") Err(e) -> print("n") }' ;;
    args)           echo 'print(args().len().to_str())' ;;
    env)            echo 'match env("HOME") { Some(v) -> print("y") None -> print("n") }' ;;
    now_millis)     echo 'print(now_millis().to_str())' ;;
    random_bytes)   echo 'match random_bytes(1) { Ok(b) -> print("y") Err(e) -> print("n") }' ;;
    tcp_listen)     echo 'match tcp_listen(0) { Ok(h) -> print("y") Err(e) -> print("n") }' ;;
    tcp_connect)    echo 'match tcp_connect("127.0.0.1", 9) { Ok(h) -> print("y") Err(e) -> print("n") }' ;;
    tcp_accept)     echo 'match tcp_accept(0) { Ok(h) -> print("y") Err(e) -> print("n") }' ;;
    tcp_read)       echo 'match tcp_read(0) { Ok(s) -> print("y") Err(e) -> print("n") }' ;;
    tcp_read_bytes) echo 'match tcp_read_bytes(0) { Ok(b) -> print("y") Err(e) -> print("n") }' ;;
    tcp_write)      echo 'match tcp_write(0, "x") { Ok(u) -> print("y") Err(e) -> print("n") }' ;;
    tcp_write_bytes) echo 'match tcp_write_bytes(0, bytes_of([1])) { Ok(u) -> print("y") Err(e) -> print("n") }' ;;
    net_close)      echo 'net_close(0)' ;;
    net_wait)       echo 'match net_wait([0], 1) { Ok(r) -> print("y") Err(e) -> print("n") }' ;;
    net_port)       echo 'match net_port(0) { Ok(p) -> print("y") Err(e) -> print("n") }' ;;
    udp_bind)       echo 'match udp_bind(0) { Ok(h) -> print("y") Err(e) -> print("n") }' ;;
    udp_send)       echo 'match udp_send(0, "127.0.0.1", 9, bytes_of([1])) { Ok(n) -> print("y") Err(e) -> print("n") }' ;;
    udp_recv)       echo 'match udp_recv(0) { Ok(m) -> print("y") Err(e) -> print("n") }' ;;
    proc_run)       echo 'match proc_run("echo", ["x"]) { Ok(r) -> print("y") Err(e) -> print("n") }' ;;
    proc_pipeline)  echo 'match proc_pipeline([("echo", ["x"])], "") { Ok(r) -> print("y") Err(e) -> print("n") }' ;;
    *) return 1 ;;
  esac
}

NAMES=$(sed -n 's/.*Builtin { name: "\([a-z_0-9]*\)".*Journal::Recorded.*/\1/p' crates/grasm_sema/src/builtins.rs)
hooked=0; unhooked=0; unwritten=0

for n in $NAMES; do
  body=$(prog "$n") || { echo "  $n: NO PROGRAM — add one when you add the builtin"; unwritten=$((unwritten+1)); continue; }
  src="$WORK/$n.gr"
  printf 'fn main() / {io, net, proc} {\n  %s\n}\n' "$body" > "$src"
  bad=""
  for engine in run native; do
    if [ "$engine" = run ]; then
      out=$(GRASM_REPLAY="$WORK/empty.jrnl" "$GRASM" run "$src" 2>&1); code=$?
    else
      "$GRASM" build --native "$src" -o "$WORK/$n.bin" >/dev/null 2>&1 || continue
      out=$(GRASM_REPLAY="$WORK/empty.jrnl" "$WORK/$n.bin" 2>&1); code=$?
    fi
    case "$code:$out" in
      7:*"journal ran out"*) ;;
      *) bad="$bad $engine(exit $code)" ;;
    esac
  done
  if [ -n "$bad" ]; then
    echo "  $n: reached the world under replay —$bad"
    unhooked=$((unhooked+1))
  else
    hooked=$((hooked+1))
  fi
done

# And once end to end on the browser engine, which has no filesystem, no
# sockets and no processes — so a recording of all three replaying there is the
# clearest statement of what a journal is: everything the program read, and
# nothing of the machine it read it from.
wasm_note="skipped (no module or no node)"
WASM=${WASM:-./target/wasm32-unknown-unknown/release/grasm_wasm.wasm}
if [ -f "$WASM" ] && command -v node >/dev/null 2>&1; then
  cat > "$WORK/cross.gr" <<'EOF'
fn main() / {io, proc} {
  print("clock " + now_millis().to_str())
  match proc_run("echo", ["a child"]) {
    Ok(r) -> print("child: " + r.1.trim())
    Err(e) -> print("no child")
  }
  match list_dir("/tmp") {
    Ok(n) -> print("listed")
    Err(e) -> print("no listing")
  }
}
EOF
  if "$GRASM" build --native "$WORK/cross.gr" -o "$WORK/cross" >/dev/null 2>&1; then
    GRASM_RECORD="$WORK/cross.jrnl" "$WORK/cross" > "$WORK/cross.native" 2>&1
    node scripts/wasm/run.mjs "$WASM" "$WORK/cross.gr" --journal "$WORK/cross.jrnl" \
      > "$WORK/cross.wasm" 2>&1
    if [ -s "$WORK/cross.native" ] && diff -q "$WORK/cross.native" "$WORK/cross.wasm" >/dev/null; then
      wasm_note="identical"
    else
      wasm_note="DIFFERS"
      diff "$WORK/cross.native" "$WORK/cross.wasm"
      unhooked=$((unhooked+1))
    fi
  fi
fi

echo "=========================================="
echo "journaled: $hooked   unhooked: $unhooked   no test program: $unwritten"
echo "a native recording replayed in the browser engine: $wasm_note"
echo "=========================================="
[ "$unhooked" = 0 ] && [ "$unwritten" = 0 ]
