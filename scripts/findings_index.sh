#!/usr/bin/env bash
# Regenerates the index at the top of ProjectGold/FINDINGS.md.
#
# The file is 1,700 lines and 43 entries, and had no way in but scrolling. An
# index typed by hand would drift the way every other hand-copied count in this
# repository has — the G4 scorecard row said 218 where the gates said 222, the
# README said seven programs above a table of eight — so it is generated from
# the headings themselves, and `--check` fails when the two disagree.
set -uo pipefail
cd "$(dirname "$0")/.."
python3 scripts/findings_index.py "$@"
