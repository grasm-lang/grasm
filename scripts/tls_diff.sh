#!/usr/bin/env bash
# A TLS 1.3 handshake, against OpenSSL's server.
#
# The claim being tested is "this client completes a handshake with a real
# server and refuses the ones it should refuse", and as with the certificate
# gate the half that matters is the refusals: a client that returned
# `verified` unconditionally would pass every positive case here, so every
# positive case is paired with a negative built from the same server.
#
# The five cases:
#
#   1. a correct handshake with an RSA server, ALPN negotiated, data both
#      ways
#   2. the same with an ECDSA P-256 server, which signs with a different
#      scheme and exercises the other verifier
#   3. the same certificate, connected to by an address it does not carry
#   4. the same certificate, checked at a moment outside its validity window
#   5. the same server, with a trust store that does not contain its root
#
# One through five all reach the same server; only the client's inputs
# differ, which is what makes the refusals attributable.
set -uo pipefail
# Job control off: killing a backgrounded server otherwise prints a
# "Terminated" line per case, which reads as a gate failure and is not one.
set +m
cd "$(dirname "$0")/.."

command -v openssl >/dev/null 2>&1 || { echo "openssl is needed"; exit 1; }

GRASM=${GRASM:-./target/release/grasm}
WORK=$(mktemp -d)
SERVER=""
cleanup() {
    if [ -n "$SERVER" ]; then kill "$SERVER" 2>/dev/null; fi
    rm -rf "$WORK"
}
trap cleanup EXIT

"$GRASM" build packages/tlsdiff/src/main.gr --native -o "$WORK/tlsdiff" \
    >/dev/null 2>&1 || { echo "the client would not build"; exit 1; }
CLIENT="$WORK/tlsdiff"

# An RSA certificate and an ECDSA one, both for localhost.
openssl req -x509 -newkey rsa:2048 -keyout "$WORK/rsa.key" \
    -out "$WORK/rsa.pem" -days 30 -nodes -subj "/CN=localhost" \
    -addext "subjectAltName=DNS:localhost" >/dev/null 2>&1
openssl x509 -in "$WORK/rsa.pem" -outform DER -out "$WORK/rsa.der"
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 \
    -keyout "$WORK/ec.key" -out "$WORK/ec.pem" -days 30 -nodes \
    -subj "/CN=localhost" -addext "subjectAltName=DNS:localhost" \
    >/dev/null 2>&1
openssl x509 -in "$WORK/ec.pem" -outform DER -out "$WORK/ec.der"
# A certificate nobody in the store vouches for.
openssl req -x509 -newkey rsa:2048 -keyout "$WORK/other.key" \
    -out "$WORK/other.pem" -days 30 -nodes -subj "/CN=elsewhere" \
    -addext "subjectAltName=DNS:elsewhere" >/dev/null 2>&1
openssl x509 -in "$WORK/other.pem" -outform DER -out "$WORK/other.der"

NOW=$(date +%s)
LONG_AGO=1000000000
PORT=${TLSPORT:-44711}
passed=0
failed=0

serve() {
    local cert=$1 key=$2
    ( printf 'pong\n'; sleep 6 ) | openssl s_server -accept "$PORT" \
        -cert "$cert" -key "$key" -tls1_3 -alpn http/1.1 -quiet -naccept 1 \
        >/dev/null 2>&1 &
    SERVER=$!
    sleep 1
}

stop() {
    if [ -n "$SERVER" ]; then kill "$SERVER" 2>/dev/null; SERVER=""; fi
    wait 2>/dev/null
    sleep 1
}

# The port is picked once and reused, so a leftover listener from an earlier
# run would make every case pass against the wrong server. Refuse that.
if nc -z 127.0.0.1 "$PORT" 2>/dev/null; then
    echo "something is already listening on $PORT"; exit 1
fi

expect() {
    local name=$1 want=$2 got=$3
    case "$got" in
        $want) passed=$((passed + 1)) ;;
        *) failed=$((failed + 1)); echo "  $name: wanted $want, got $got" ;;
    esac
}

serve "$WORK/rsa.pem" "$WORK/rsa.key"
got=$("$CLIENT" "$PORT" "$WORK/rsa.der" localhost "$NOW" 2>&1)
expect "rsa handshake" 'verified|localhost|2052|http/1.1|true|pong' "$got"
stop

serve "$WORK/ec.pem" "$WORK/ec.key"
got=$("$CLIENT" "$PORT" "$WORK/ec.der" localhost "$NOW" 2>&1)
expect "ecdsa handshake" 'verified|localhost|1027|http/1.1|true|pong' "$got"
stop

serve "$WORK/rsa.pem" "$WORK/rsa.key"
got=$("$CLIENT" "$PORT" "$WORK/rsa.der" 127.0.0.1 "$NOW" 2>&1)
expect "wrong name" 'unverified|the certificate does not carry 127.0.0.1*' "$got"
stop

serve "$WORK/rsa.pem" "$WORK/rsa.key"
got=$("$CLIENT" "$PORT" "$WORK/rsa.der" localhost "$LONG_AGO" 2>&1)
expect "outside the window" 'unverified|*validity window*' "$got"
stop

serve "$WORK/rsa.pem" "$WORK/rsa.key"
got=$("$CLIENT" "$PORT" "$WORK/other.der" localhost "$NOW" 2>&1)
expect "untrusted root" 'unverified|*trusted root*' "$got"
stop

echo "handshakes: $((passed + failed))   as expected: $passed   wrong: $failed"
[ "$failed" -eq 0 ]
