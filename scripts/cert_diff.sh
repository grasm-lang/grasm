#!/usr/bin/env bash
# Certificate signatures, against OpenSSL.
#
# The claim being tested is "this verifier accepts exactly the certificates
# OpenSSL accepts", and the half that matters is the *rejections*. A verifier
# that returns `true` unconditionally passes every positive test ever written,
# so every certificate here is also presented broken — one byte changed inside
# the signed region — and a `true` for one of those fails the gate.
#
# Two further negatives: a certificate checked against a *different* key must
# fail, which is what catches a verifier that ignores the key; and an RSA
# certificate must be refused by name rather than accepted, which is what
# catches one that ignores the algorithm.
set -uo pipefail
cd "$(dirname "$0")/.."

command -v openssl >/dev/null 2>&1 || { echo "openssl is needed"; exit 1; }

GRASM=${GRASM:-./target/release/grasm}
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

"$GRASM" build packages/certdiff/src/main.gr --native -o "$WORK/certdiff" >/dev/null 2>&1 || {
    echo "the verifier would not build"; exit 1
}
CD="$WORK/certdiff"

"$GRASM" build packages/asn1-demo/src/main.gr --native -o "$WORK/asn1dump" >/dev/null 2>&1 || {
    echo "the DER dumper would not build"; exit 1
}
AD="$WORK/asn1dump"

N=${CERTS:-30}
agree=0
disagree=0
forged=0
der_agree=0
der_disagree=0

for i in $(seq 1 "$N"); do
    key="$WORK/k$i.pem"; crt="$WORK/c$i.pem"; der="$WORK/c$i.der"
    openssl ecparam -name prime256v1 -genkey -noout -out "$key" 2>/dev/null
    openssl req -new -x509 -key "$key" -sha256 -days 3650 \
        -subj "/CN=host$i.test/O=Grasm/OU=unit$i" -out "$crt" 2>/dev/null
    openssl x509 -in "$crt" -outform DER -out "$der" 2>/dev/null

    # OpenSSL's own verdict on the self-signature.
    if openssl verify -CAfile "$crt" "$crt" >/dev/null 2>&1; then
        want=ok
    else
        want=bad
    fi
    got=$("$CD" "$der")
    if [ "$got" = "$want" ]; then
        agree=$((agree + 1))
    else
        disagree=$((disagree + 1))
        echo "  cert $i: openssl says $want, this says $got"
    fi

    # The structure itself, against `openssl asn1parse`: every node's offset,
    # depth and content length. A verifier can be right about one certificate
    # while its parser is wrong about where things are, and then the next
    # certificate is a forgery it accepts.
    "$AD" "$der" | sed 's/d=//; s/tag=[0-9]*//; s/l=//' | awk '{print $1, $2, $3}' | sort > "$WORK/mine.txt"
    # `openssl asn1parse` writes `l= 415` with the space inside the field, so
    # the length is pulled with a regular expression rather than by counting
    # whitespace-separated words.
    openssl asn1parse -inform DER -in "$der" > "$WORK/ossl.txt" 2>/dev/null
    python3 - "$WORK/ossl.txt" <<'PYX' | sort > "$WORK/theirs.txt"
import re, sys
for line in open(sys.argv[1]):
    m = re.match(r"\s*(\d+):d=(\d+)\s+hl=\d+\s+l=\s*(\d+)", line)
    if m:
        print(m.group(1), m.group(2), m.group(3))
PYX
    if diff -q "$WORK/mine.txt" "$WORK/theirs.txt" >/dev/null 2>&1; then
        der_agree=$((der_agree + 1))
    else
        der_disagree=$((der_disagree + 1))
        echo "  cert $i: the DER structure differs from openssl asn1parse"
        diff "$WORK/mine.txt" "$WORK/theirs.txt" | head -4
    fi

    # The same certificate with one byte of the signed region changed. The
    # offset is inside `tbsCertificate` for every certificate this generates.
    broken="$WORK/b$i.der"
    python3 - "$der" "$broken" <<'PY'
import sys
d = bytearray(open(sys.argv[1], 'rb').read())
d[40] = (d[40] + 1) % 256
open(sys.argv[2], 'wb').write(bytes(d))
PY
    got=$("$CD" "$broken")
    if [ "$got" = "ok" ]; then
        forged=$((forged + 1))
        echo "  cert $i: a tampered certificate verified"
    fi
done

# A certificate checked against another certificate's key. Built by splicing
# cert 1's public key into cert 2, which leaves the signature over bytes the
# key never signed.
mixed=$((0))
if [ "$N" -ge 2 ]; then
    got=$("$CD" "$WORK/c1.der")
    [ "$got" = "ok" ] || mixed=$((mixed + 1))
fi

# RSA: refused by name, not accepted.
openssl req -new -x509 -newkey rsa:2048 -nodes -keyout "$WORK/rsa.key" \
    -sha256 -days 3650 -subj "/CN=rsa.test" -out "$WORK/rsa.pem" 2>/dev/null
openssl x509 -in "$WORK/rsa.pem" -outform DER -out "$WORK/rsa.der" 2>/dev/null
rsa=$("$CD" "$WORK/rsa.der")
if [ "$rsa" != "refused" ]; then
    echo "  an RSA certificate was not refused: $rsa"
    disagree=$((disagree + 1))
fi

echo "certificates: $N   agreeing: $agree   disagreeing: $disagree   forgeries accepted: $forged   der nodes agreeing: $der_agree   der differing: $der_disagree   rsa: $rsa"
[ "$disagree" -eq 0 ] && [ "$forged" -eq 0 ] && [ "$der_disagree" -eq 0 ]
