"""BIP-39 against the vectors its reference implementation publishes.

Built natively rather than interpreted: the seed is PBKDF2 with two
thousand rounds of HMAC-SHA-512, which the interpreter takes half a minute
per vector to do and Cranelift takes half a second.
"""
import json
import os
import subprocess
import sys
import tempfile


def main():
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    grasm = os.environ.get("GRASM", os.path.join(root, "target/release/grasm"))
    fixture = os.path.join(root, "scripts/fixtures/bip39-vectors.json")
    vectors = json.load(open(fixture))["english"]

    work = tempfile.mkdtemp()
    harness = os.path.join(work, "mntest")
    built = subprocess.run([grasm, "build", "packages/mntest/src/main.gr",
                            "--native", "-o", harness], cwd=root,
                           capture_output=True, text=True)
    if built.returncode != 0:
        print(built.stderr[-1500:])
        sys.exit(1)

    argv = []
    for entropy, phrase, _seed, _xprv in vectors:
        argv += ["words", entropy, "-", "entropy", phrase, "-",
                 "seed", phrase, "TREZOR"]
    got = subprocess.run([harness] + argv, capture_output=True, text=True)
    if got.returncode != 0:
        print(got.stderr[-1500:])
        sys.exit(1)
    lines = got.stdout.split("\n")

    agree = differ = shown = 0
    at = 0
    for entropy, phrase, seed, _xprv in vectors:
        for label, want in (("words", phrase), ("entropy", entropy),
                            ("seed", seed)):
            mine = lines[at] if at < len(lines) else "<missing>"
            at += 1
            if mine == want:
                agree += 1
            else:
                differ += 1
                shown += 1
                if shown <= 10:
                    print(f"  {label} of {entropy}")
                    print(f"    bip39 {want}")
                    print(f"    grasm {mine}")
    print(f"vectors: {agree + differ}   agreeing: {agree}   "
          f"differing: {differ}")
    sys.exit(1 if differ else 0)


main()
