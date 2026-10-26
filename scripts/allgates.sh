#!/usr/bin/env bash
# Every gate, judged by exit code.
#
# A gate that fails says so in its exit status. Reading its summary line
# instead is how `wasmtest.sh` was reported green while it was exiting 1 over
# four stale committed modules -- the line said "identical: 184 mismatched: 0",
# which was true, and the failure was three lines further down. The same
# mistake in another shape hid nine of eleven `agreetest` failures behind a
# `tail -2`.
#
# So this reads one thing per gate: `$?`. The summary is printed for
# information and is never consulted. Full output goes to a log per gate, named
# on failure.
set -uo pipefail
cd "$(dirname "$0")/.."

# A subset, by name match, so the suite can be run in parts. The whole thing
# takes longer than some runners allow in one go, and the alternative to
# splitting it was a run that gets killed partway and reports nothing.
#
# `./scripts/allgates.sh diff` runs every gate whose name contains "diff".
# Several patterns may be given and a gate matching any of them runs.
#
# A subset run says so on its last line and never says "every gate passed",
# because a partial green quoted as a full one is the mistake this whole
# script exists to prevent.
#
# `sitefacts` reads other gates' logs to record their figures, and each run
# writes to a fresh `mktemp -d` unless `LOGS` is set. So splitting the suite
# across runs leaves it unable to find them -- it failed with "the crosstest
# gate left no log, so its figure cannot be recorded", which is the gate
# being right. Set `LOGS` to one directory across the slices, or run
# `sitefacts` in the same slice as the gates it reads: difftest, wasmtest,
# crosstest, gold, fuzz_laws and depth.
ONLY=("$@")
matches() {
    [ ${#ONLY[@]} -eq 0 ] && return 0
    local pat
    for pat in "${ONLY[@]}"; do
        case "$1" in
            *"$pat"*) return 0 ;;
        esac
    done
    return 1
}

LOGS=${LOGS:-$(mktemp -d)}
mkdir -p "$LOGS"
# The oracles are pinned for the run. Several gates hold ports to CPython's
# standard library, to pip, and to SQLite through Python's sqlite3 module, and
# the ports document the versions they follow: CPython 3.13.1, pip 24.3.1,
# SQLite 3.51.1. On 2026-10-25 a Homebrew upgrade moved all of them at once --
# `python3` to 3.15 (no Pillow), `python3.13` to 3.13.17, pip to 26.2.2,
# SQLite to 3.54.1 -- and twenty-five gates failed on code that had not
# changed. Each pin can be overridden; when the pinned version is not
# installed the gate runs against whatever is, and the line below says so.
PYSHIM=$(mktemp -d)
PY3=${GRASM_PYTHON3:-python3.14}
command -v "$PY3" >/dev/null 2>&1 && ln -s "$(command -v "$PY3")" "$PYSHIM/python3"
PY313=${GRASM_PYTHON313:-/opt/homebrew/Cellar/python@3.13/3.13.1/bin/python3.13}
[ -x "$PY313" ] && ln -s "$PY313" "$PYSHIM/python3.13"
export PATH="$PYSHIM:$PATH"
# pip 24.3.1 is the wheel CPython 3.13.1 bundles for ensurepip.
PIPWHL=${GRASM_PIP_ORACLE:-$(dirname "$(dirname "$PY313")")/Frameworks/Python.framework/Versions/3.13/lib/python3.13/ensurepip/_bundled/pip-24.3.1-py3-none-any.whl}
[ -f "$PIPWHL" ] && export PYTHONPATH="$PIPWHL${PYTHONPATH:+:$PYTHONPATH}"
# The SQLite library goes by name rather than as DYLD_LIBRARY_PATH: macOS
# strips DYLD_* from the environment of every protected binary it starts, and
# the gate scripts start through /usr/bin/env, so the variable never reached
# the oracle. b54_diff.py sets it on the oracle process itself.
SQLITELIB=${GRASM_SQLITE_ORACLE:-/opt/homebrew/Cellar/sqlite/3.51.1/lib}
[ -d "$SQLITELIB" ] && export GRASM_SQLITE_ORACLE="$SQLITELIB"
echo "oracles: python3 $(python3 -c 'import sys; print(sys.version.split()[0])' 2>/dev/null)," \
    "python3.13 $(python3.13 -c 'import sys; print(sys.version.split()[0])' 2>/dev/null)," \
    "pip $(python3.13 -c 'import pip; print(pip.__version__)' 2>/dev/null)," \
    "sqlite $(DYLD_LIBRARY_PATH=${GRASM_SQLITE_ORACLE:-} python3.13 -c 'import sqlite3; print(sqlite3.sqlite_version)' 2>/dev/null)," \
    "$(openssl version 2>/dev/null)"
# What git did not track before the run, so the `litter` gate can tell what
# the run itself left behind.
git ls-files --others --exclude-standard | sort > "$LOGS/untracked-before.txt"
# And what sits at the top of the checkout: git lists no empty directory,
# and two conformance programs left two empty ones at the root for weeks.
ls -A | sort > "$LOGS/top-before.txt"
failed=0
ran=0
skipped=0
declare -a FAILED=()

# Runs one gate and reports it. `$1` is the label, the rest is the command.
run_gate() {
    local name="$1"; shift
    if ! matches "$name"; then
        skipped=$((skipped + 1))
        return 0
    fi
    ran=$((ran + 1))
    local log="$LOGS/$name.log"
    printf '%-16s ' "$name"
    if "$@" >"$log" 2>&1; then
        # The last whole line carrying two or more counts, for information
        # only. Matching the counts themselves cut "not compiled: 0" down to
        # "compiled: 0", which says the opposite of what the gate found -- and
        # a runner built so a summary cannot mislead should not print a
        # misleading summary of its own.
        local note
        note=$(grep -hE '[a-z-]+: [0-9]+.*[a-z-]+: [0-9]+' "$log" | tail -1)
        echo "PASS  ${note:-ok}"
    else
        local code=$?
        echo "FAIL  (exit $code) $log"
        failed=$((failed + 1))
        FAILED+=("$name")
    fi
}

run_gate difftest      scripts/difftest.sh
run_gate leakcheck     scripts/leakcheck.sh
run_gate usecheck      scripts/usecheck.sh
run_gate agreetest     scripts/agreetest.sh
run_gate nativegaps    scripts/nativegaps.sh
run_gate tirtest       scripts/tirtest.sh
run_gate tirinterp     scripts/tirinterp.sh
run_gate tirccall      python3 scripts/tirinterp_ccall.py --check
run_gate s3subset      scripts/s3subset.sh
run_gate encodetest    scripts/encodetest.sh
run_gate objtest       scripts/objtest.sh
run_gate ra_check      scripts/ra_check.sh
run_gate s3rc          scripts/s3rc.sh
run_gate s3controls    scripts/s3controls.sh
run_gate linktest      scripts/linktest.sh
run_gate linkdiff      scripts/linkdiff.sh
run_gate linkcontrols  scripts/linkcontrols.sh
run_gate rtlayout      scripts/rtlayout.sh
run_gate rtswap        scripts/rtswap.sh
run_gate fuzz_rtfmt    scripts/fuzz_rtfmt.sh
run_gate fuzz_rtmath   scripts/fuzz_rtmath.sh
run_gate rtcontrols    scripts/rtcontrols.sh
run_gate reprotest     scripts/reprotest.sh
run_gate inctest       python3 scripts/inctest.py
run_gate brokertest    scripts/broker_test.sh
run_gate audiodiff     scripts/audio_diff.sh
run_gate mixertest     scripts/mixer_test.sh
run_gate langdiff      scripts/lang_diff.sh
run_gate lang2diff     scripts/lang2_diff.sh
run_gate replaytest    scripts/replaytest.sh
run_gate wasmtest      scripts/wasmtest.sh
run_gate gatecheck     scripts/gatecheck.sh
run_gate crosstest     scripts/crosstest.sh
run_gate fmttest       scripts/fmttest.sh
run_gate linttest      scripts/linttest.sh
run_gate gfmttest      scripts/gfmttest.sh
run_gate grammar       python3 scripts/grammar.py --check
run_gate sdltest       scripts/sdltest.sh
run_gate jpegtest      scripts/jpegtest.sh
run_gate fonttest      scripts/fonttest.sh
run_gate tifftest      scripts/tifftest.sh
run_gate blake3test    scripts/blake3test.sh
run_gate imagetest     scripts/imagetest.sh
run_gate zstdtest      scripts/zstdtest.sh
run_gate rontest       scripts/rontest.sh
run_gate webptest      scripts/webptest.sh
run_gate rmcptest      scripts/rmcptest.sh
run_gate pngwrite      scripts/pngwritetest.sh
run_gate xxhashtest    scripts/xxhashtest.sh
run_gate tiffwrite     scripts/tiffwritetest.sh
run_gate multisearch   scripts/multisearchtest.sh
run_gate walkdirtest   scripts/walkdirtest.sh
run_gate tempdirtest   scripts/tempdirtest.sh
run_gate gostddiff     scripts/gostd_diff.sh
run_gate pubkeydiff    scripts/pubkey_diff.sh
run_gate pqcdiff       scripts/pqc_diff.sh
run_gate gostd2diff    scripts/gostd2_diff.sh
run_gate packetdiff    scripts/packet_diff.sh
run_gate graphdiff     scripts/graph_diff.sh
run_gate dsdiff        scripts/ds_diff.sh
run_gate ds2diff       scripts/ds2_diff.sh
run_gate b28diff       scripts/b28_diff.sh
run_gate numdiff       scripts/num_diff.sh
run_gate b18diff       scripts/b18_diff.sh
run_gate gitdiff       scripts/git_diff.sh
run_gate caldiff       scripts/cal_diff.sh
run_gate sigdiff       scripts/sig_diff.sh
run_gate cjkdiff       scripts/cjk_diff.sh
run_gate packet4diff   scripts/packet4_diff.sh
run_gate b28bdiff      scripts/b28b_diff.sh
run_gate b28cdiff      scripts/b28c_diff.sh
run_gate b29diff       scripts/b29_diff.sh
run_gate b30diff       scripts/b30_diff.sh
run_gate b31diff       scripts/b31_diff.sh
run_gate b32diff       scripts/b32_diff.sh
run_gate b33diff       scripts/b33_diff.sh
run_gate b34diff       scripts/b34_diff.sh
run_gate b35diff       scripts/b35_diff.sh
run_gate b36diff       scripts/b36_diff.sh
run_gate b37diff       scripts/b37_diff.sh
run_gate b38diff       scripts/b38_diff.sh
run_gate b39diff       scripts/b39_diff.sh
run_gate b40diff       scripts/b40_diff.sh
run_gate b41diff       scripts/b41_diff.sh
run_gate b42diff       scripts/b42_diff.sh
run_gate b43diff       scripts/b43_diff.sh
run_gate b44diff       scripts/b44_diff.sh
run_gate b45diff       scripts/b45_diff.sh
run_gate b46diff       scripts/b46_diff.sh
run_gate b47diff       scripts/b47_diff.sh
run_gate b48diff       scripts/b48_diff.sh
run_gate b49diff       scripts/b49_diff.sh
run_gate b50diff       scripts/b50_diff.sh
run_gate b51diff       scripts/b51_diff.sh
run_gate b52diff       scripts/b52_diff.sh
run_gate b53diff       scripts/b53_diff.sh
run_gate b54diff       scripts/b54_diff.sh
run_gate b55diff       scripts/b55_diff.sh
run_gate b56diff       scripts/b56_diff.sh
run_gate b57diff       scripts/b57_diff.sh
run_gate b58diff       scripts/b58_diff.sh
run_gate b59diff       scripts/b59_diff.sh
run_gate b60diff       scripts/b60_diff.sh
run_gate b61diff       scripts/b61_diff.sh
run_gate b62diff       scripts/b62_diff.sh
run_gate grasmwire     scripts/grasmwire_e2e.sh
run_gate cryptodiff    scripts/crypto_diff.sh
run_gate certdiff      scripts/cert_diff.sh
run_gate tlsdiff       scripts/tls_diff.sh
run_gate httpsdiff     scripts/https_diff.sh
run_gate h2diff        scripts/h2_diff.sh
run_gate uritdiff      scripts/urit_diff.sh
run_gate patchdiff     scripts/patch_diff.sh
run_gate segmentdiff   scripts/segment_diff.sh
run_gate normdiff      scripts/normalise_diff.sh
run_gate bip39diff     scripts/bip39_diff.sh
run_gate sfdiff        scripts/sf_diff.sh
run_gate linebreakdiff scripts/linebreak_diff.sh
run_gate gitconfigdiff  scripts/gitconfig_diff.sh
run_gate netrcdiff      scripts/netrc_diff.sh
run_gate knownhostsdiff scripts/known_hosts_diff.sh
run_gate ipv6diff       scripts/ipv6_diff.sh
run_gate purldiff       scripts/purl_diff.sh
run_gate rpmvercmpdiff  scripts/rpm_vercmp_diff.sh
run_gate gemdiff        scripts/gem_version_diff.sh
run_gate sniffdiff      scripts/sniff_diff.sh
run_gate cpiodiff       scripts/cpio_diff.sh
run_gate checksumdiff   scripts/checksum_diff.sh
run_gate mimeworddiff   scripts/mimeword_diff.sh
run_gate addressdiff    scripts/address_diff.sh
run_gate mboxdiff       scripts/mbox_diff.sh
run_gate wsserverdiff  scripts/wsserver_diff.sh
run_gate respdiff      scripts/resp_diff.sh
run_gate structdiff     scripts/struct_diff.sh
run_gate encodingdiff   scripts/encoding_diff.sh
run_gate md5diff        scripts/md5_diff.sh
run_gate enc4diff       scripts/enc4_diff.sh
run_gate closematchdiff scripts/closematch_diff.sh
run_gate compressdiff   scripts/compress_diff.sh
run_gate unixcryptdiff  scripts/unixcrypt_diff.sh
run_gate sshkeydiff     scripts/sshkey_diff.sh
run_gate japanesediff   scripts/japanese_diff.sh
run_gate iso3166diff    scripts/iso3166_diff.sh
run_gate mersennediff   scripts/mersenne_diff.sh
run_gate gettextdiff    scripts/gettext_diff.sh
run_gate picklediff     scripts/pickle_diff.sh
run_gate formatspecdiff scripts/formatspec_diff.sh
run_gate xmlrpcdiff     scripts/xmlrpc_diff.sh
run_gate difftextdiff   scripts/difftext_diff.sh
run_gate floatreprdiff  scripts/shortestfloat_diff.sh
run_gate idnadiff       scripts/idna_diff.sh
run_gate encoding5diff  scripts/encoding5_diff.sh
run_gate gitfilediff    scripts/git_diff.sh
run_gate prepdiff       scripts/prep_diff.sh
run_gate paramdiff      scripts/param_diff.sh
run_gate gitsigndiff    scripts/gitsign_diff.sh
run_gate signingdiff    scripts/signing_diff.sh
run_gate repodiff       scripts/repo_diff.sh
run_gate dbcsdiff       scripts/dbcs_diff.sh
run_gate text2diff      scripts/text2_diff.sh
run_gate net3diff       scripts/net3_diff.sh
run_gate pydatadiff     scripts/pydata_diff.sh
run_gate gitwirediff    scripts/gitwire_diff.sh
run_gate unixdiff       scripts/unix_diff.sh
run_gate kvcrash       scripts/kvstore_crash.sh
run_gate zlibdiff      scripts/zlib_diff.sh
run_gate casefolddiff  scripts/casefold_diff.sh
run_gate wasmdiff      scripts/wasm_diff.sh
run_gate asminterop    scripts/asm_interop.sh
run_gate gitinterop    scripts/git_interop.sh
run_gate northstar     scripts/northstar.sh
run_gate depth         scripts/depthtest.sh
run_gate panics        scripts/panictest.sh
run_gate gold          ProjectGold/run.sh
run_gate findings      scripts/findings_index.sh --check
run_gate clippy        cargo clippy --release --workspace --all-targets -- -D warnings
run_gate unittests     cargo test --release --workspace

for f in scripts/fuzz_*.py; do
    run_gate "$(basename "$f" .py)" python3 "$f"
done

# The site states what the gates found. Rather than let somebody retype a
# figure, the run that proved it writes it down and the site is regenerated
# from that record, so a number on the page is the last green run's or the
# page does not build. Last, because there is nothing to record until the
# gates above have finished.
site_facts() {
    python3 - "$LOGS" <<'FACTS'
import datetime, json, os, re, sys

logs = sys.argv[1]
# gate log -> (the site's name for it, the count that gate's summary prints)
WANT = {
    "difftest":  [("corpus", "identical")],
    "wasmtest":  [("wasm_ok", "identical"), ("wasm_skip", "skipped")],
    "crosstest": [("cross", "identical")],
    "gold":      [("agreeing", "agreeing")],
    "fuzz_laws": [("laws", "laws")],
    "depth":     [("depth_cap", "capacity")],
}
out = {}
for gate, wants in sorted(WANT.items()):
    path = os.path.join(logs, gate + ".log")
    if not os.path.exists(path):
        print(f"the {gate} gate left no log, so its figure cannot be recorded")
        sys.exit(1)
    text = open(path).read()
    for key, field in wants:
        hit = re.findall(r"\b" + field + r": ([0-9]+)", text)
        if not hit:
            print(f"the {gate} gate no longer prints '{field}:'. Writing the")
            print("site from this run would state a figure nobody measured.")
            sys.exit(1)
        out[key] = int(hit[-1])
        # The date this figure was taken, because the file carried none and
        # the one written in September was still being read in October --
        # under a page claiming every number came from the run that proved
        # it. `scripts/site.py` prints this beside the figure.
        out.setdefault("taken", {})[key] = datetime.date.today().isoformat()
os.makedirs("site", exist_ok=True)
with open("site/facts.json", "w") as f:
    json.dump(out, f, indent=2, sort_keys=True)
    f.write("\n")
print("   ".join(f"{k}: {v}" for k, v in sorted(out.items()) if k != "taken"))
FACTS
}

# A gate that writes into the checkout. `pngwrite-demo` and `tiffwrite-demo`
# wrote 31 images into the repository root whenever a gate ran them with no
# arguments, and a commit that staged everything published them. A file
# created by hand while the suite runs shows up here too; that is a false
# alarm worth reading rather than a reason to look less.
litter() {
    git ls-files --others --exclude-standard | sort > "$LOGS/untracked-after.txt"
    local new
    new=$(comm -13 "$LOGS/untracked-before.txt" "$LOGS/untracked-after.txt")
    ls -A | sort > "$LOGS/top-after.txt"
    local entry
    for entry in $(comm -13 "$LOGS/top-before.txt" "$LOGS/top-after.txt"); do
        # Ignored paths (target/, a build's cache) are allowed to appear.
        git check-ignore -q "$entry" || new="$new
$entry/"
    done
    new=$(printf '%s\n' "$new" | sed '/^$/d' | sort -u)
    if [ -n "$new" ]; then
        echo "files that appeared in the checkout during this run:"
        echo "$new" | sed 's/^/  /'
        return 1
    fi
    echo "new untracked files: 0"
}
run_gate litter litter

if [ "$failed" -eq 0 ]; then
    run_gate sitefacts site_facts
    run_gate site      python3 scripts/site.py
fi

echo
if [ "$failed" -gt 0 ]; then
    echo "$failed gate(s) failed: ${FAILED[*]}"
    echo "logs under $LOGS"
    exit 1
fi
if [ "$ran" -eq 0 ]; then
    # Nothing matched, which is a typo in the pattern and not a pass.
    echo "no gate matched ${ONLY[*]}, so nothing ran"
    exit 1
fi
if [ "$skipped" -gt 0 ]; then
    echo "$ran gate(s) passed, $skipped skipped by ${ONLY[*]} — this is a subset"
    echo "the whole suite is $((ran + skipped)) gates"
    exit 0
fi
# The count comes from here and not from counting `run_gate` lines. Fifteen
# of the gates come out of a loop over `scripts/fuzz_*.py`, so a grep for
# the call sees one where there are fifteen -- which is how this was being
# quoted as 110 when it is 123.
echo "every gate passed — $ran of them"
