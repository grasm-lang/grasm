#!/usr/bin/env bash
# Where runtime time actually goes, aggregated over several real programs.
S=${TMPDIR:-/tmp}/grasm-mine
E=${TMPDIR:-/tmp}/grasm-systems-etl
mkdir -p "$S" "$S/mine"
: > $S/mine/dyn.txt
run() {
    "$@" >/dev/null 2>&1 &
    local p=$!
    sleep 0.25
    sample $p 3 -file $S/mine/one.sample >/dev/null 2>&1
    wait $p 2>/dev/null
    grep -A400 "Sort by top of stack" $S/mine/one.sample 2>/dev/null \
      | awk '/\(in /{n=$0; sub(/^ +/,"",n); sub(/ +\(in .*/,"",n); c=$NF; if (c+0>0) print c"\t"n}' \
      >> $S/mine/dyn.txt
}
run $S/m_etl $E/etl-data/etl50.jsonl
run $S/m_dh 2000000
run $S/m_tree
run $S/m_wf
