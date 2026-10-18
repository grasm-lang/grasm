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
