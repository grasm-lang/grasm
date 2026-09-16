#!/usr/bin/env bash
# JavaScript numbers, Tcl lists, Lua literals, Ruby's Marshal and Perl's pack
# — against node, tclsh, lua, ruby and perl.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/lang_diff.py
