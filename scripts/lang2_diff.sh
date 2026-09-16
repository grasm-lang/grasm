#!/usr/bin/env bash
# A command line, a query string, a printf template, an INI file and Python
# source — against Python, Node and Perl.
set -uo pipefail
cd "$(dirname "$0")/.."
GRASM=${GRASM:-./target/release/grasm} exec python3 scripts/lang2_diff.py
