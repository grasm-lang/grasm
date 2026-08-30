#!/usr/bin/env bash
# Does the tunnel actually carry traffic?
#
# Every other check on GrasmWire runs the protocol as arithmetic: no socket, no
# clock, deterministic. That is what makes forty-five properties testable in no
# time at all, and it is also what those checks cannot see — whether two real
# processes, each with its own random ephemeral key, agree over a real network.
#
# So this runs two. It is the only GrasmWire check that binds a port.
set -uo pipefail

cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm}
WORK="${TMPDIR:-/tmp}/grasm_wire_$$"
mkdir -p "$WORK"
trap 'rm -rf "$WORK"' EXIT

if [ ! -x "$GRASM" ]; then
    echo "error: $GRASM missing — run: cargo build --release --workspace"
    exit 1
fi

if ! "$GRASM" build apps/grasmwire/src/main.gr --native -o "$WORK/peer" >/dev/null 2>&1; then
    echo "the endpoint would not build"
    exit 1
fi

# Fixed keys for the identities, because who is allowed to connect is
# configuration. The *ephemeral* keys are still fresh from the operating system
# on every run, so the handshake is different every time and the transport keys
# are never the same twice — which is the property this check is here for.
A=a8abababababababababababababababababababababababababababababab6b
B=c8cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd4b
PA=e3712d851a0e5d79b831c5e34ab22b41a198171de209b8b8faca23a11c624859
PB=6f22dd3bb0d5ac76eabf6f0b028cc20ec69921e49692c7402f089e2618236c26
PSK=0505050505050505050505050505050505050505050505050505050505050505

# A port pair nothing standard uses, offset by the process id so two runs of
# this script cannot collide with each other.
BASE=$((41000 + ($$ % 2000) * 2))
LPORT=$BASE
CPORT=$((BASE + 1))

pass=0
fail=0
note() {
    if [ "$1" == "yes" ]; then
        pass=$((pass + 1))
    else
        fail=$((fail + 1))
        echo "  FAIL: $2"
    fi
}

# The listener is told a deliberately wrong fallback port. It has to learn
# where its peer actually is from the datagram, with `udp_recv_from`, or
# nothing below this line works — which is the difference between that builtin
# doing something and merely being called.
"$WORK/peer" listen  "$LPORT" 9 "$B" "$PA" "$PSK" >"$WORK/listen.out" 2>&1 &
listener=$!
"$WORK/peer" connect "$CPORT" "$LPORT" "$A" "$PB" "$PSK" >"$WORK/connect.out" 2>&1 &
caller=$!
wait $caller; wait $listener

L=$(cat "$WORK/listen.out")
C=$(cat "$WORK/connect.out")

grep -q "TUNNEL OK" <<<"$C" && note yes || note no "the caller did not get every packet back"
grep -q "TUNNEL OK" <<<"$L" && note yes || note no "the listener did not echo every packet"
grep -q "came back byte for byte" <<<"$C" && note yes || note no "no packet survived the round trip"
grep -q "replayed handshake, refused" <<<"$L" && note yes || note no "a recorded first message was not refused"

# Both sides must have derived the same session hash without either sending it.
lh=$(grep -o 'session hash [0-9a-f]*' <<<"$L" | head -1)
ch=$(grep -o 'session hash [0-9a-f]*' <<<"$C" | head -1)
if [ -n "$lh" ] && [ "$lh" == "$ch" ]; then
    note yes
else
    note no "the two sides disagreed on the session: '$lh' against '$ch'"
fi

# A handshake that authenticates but comes from a key nobody allowed.
"$WORK/peer" listen  "$((LPORT + 4))" 9 "$B" "$PB" "$PSK" >"$WORK/l2.out" 2>&1 &
l2=$!
"$WORK/peer" connect "$((CPORT + 4))" "$((LPORT + 4))" "$A" "$PB" "$PSK" >"$WORK/c2.out" 2>&1 &
c2=$!
wait $c2; wait $l2
grep -q "unknown key" "$WORK/l2.out" && note yes || note no "an unlisted peer was not refused"

# And the address it replied to was the one it read off the wire, not the one
# it was configured with.
grep -q "first message from 127.0.0.1:$CPORT" "$WORK/listen.out" && note yes \
    || note no "the listener did not learn its peer's address from the datagram"

# And the ephemeral keys really are fresh: a second run must not reproduce the
# first run's session. A protocol that derived the same keys twice from the
# same identities would have no forward secrecy at all.
"$WORK/peer" listen  "$((LPORT + 8))" 9 "$B" "$PA" "$PSK" >"$WORK/l3.out" 2>&1 &
l3=$!
"$WORK/peer" connect "$((CPORT + 8))" "$((LPORT + 8))" "$A" "$PB" "$PSK" >"$WORK/c3.out" 2>&1 &
c3=$!
wait $c3; wait $l3
again=$(grep -o 'session hash [0-9a-f]*' "$WORK/l3.out" | head -1)
if [ -n "$again" ] && [ "$again" != "$lh" ]; then
    note yes
else
    note no "two runs produced the same session — the ephemeral keys are not fresh"
fi

echo "checked: $((pass + fail))  passed: $pass  failed: $fail"
[ "$fail" -eq 0 ]
