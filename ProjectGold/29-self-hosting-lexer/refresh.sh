#!/usr/bin/env bash
# Regenerates counts.txt: the reference lexer's token count for every program.
#
# Run from ProjectGold/. `run.sh` does this before every gate run, because a
# stale table turns "my lexer disagrees with the reference" into "somebody
# edited a program", and those are not the same finding.
set -euo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-../target/release/grasm}
: > 29-self-hosting-lexer/counts.txt
for f in [0-9]*/main.gr; do
    n=$("$GRASM" tokens "$f" 2>/dev/null | grep -v Eof | grep -c . || true)
    echo "$f $n" >> 29-self-hosting-lexer/counts.txt
done
