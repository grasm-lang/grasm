"""File sniffing against `file(1)`'s own media types.

The corpus is built here rather than checked in: every file is made from
the standard library or from a tool the machine already has, so the gate
brings its own evidence and nothing has to be trusted to a fixture.

Two kinds of disagreement are named rather than counted as failures.
`file(1)` knows some types by parsing rather than by signature -- it
reads a whole JSON document to answer `application/json` -- and this does
not; and this knows a few registered types the installed `file(1)` is
older than.
"""
import bz2
import gzip
import lzma
import os
import shutil
import sqlite3
import subprocess
import sys
import tarfile
import tempfile
import wave
import zipfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GRASM = os.environ.get("GRASM", os.path.join(ROOT, "target/release/grasm"))
HARNESS = os.path.join(ROOT, "packages/sniftest/src/main.gr")

BY_PARSING = {"application/json", "text/csv", "application/csv"}
NEWER_THAN_FILE = {"application/wasm", "application/zstd",
                   "application/x-lz4", "image/avif"}


def build(work):
    """Make one file of every kind the sniffer claims to know."""
    # No commas: `file(1)` reads a comma-separated repeated line as CSV,
    # which it works out by parsing rather than from any signature.
    body = b"hello world - a plain text file with some words in it\n" * 4
    made = []

    def write(name, data):
        path = os.path.join(work, name)
        with open(path, "wb") as handle:
            handle.write(data)
        made.append(path)
        return path

    write("plain.txt", body)
    write("utf8.txt", "café naïve résumé\n".encode())
    write("script.sh", b"#!/bin/sh\necho hi\n")
    write("empty.bin", b"")
    write("random.bin", bytes([(i * 7 + 3) % 256 for i in range(512)]))
    write("wasm.wasm", b"\x00asm\x01\x00\x00\x00" + bytes(16))
    write("xml.xml", b"<?xml version=\"1.0\"?><a/>\n")
    write("pdf.pdf", b"%PDF-1.4\n%\xc7\xec\x8f\xa2\n" + bytes(64))
    write("gzip.gz", gzip.compress(body))
    write("bzip2.bz2", bz2.compress(body))
    write("xz.xz", lzma.compress(body))
    path = os.path.join(work, "archive.tar")
    with tarfile.open(path, "w") as handle:
        handle.add(os.path.join(work, "plain.txt"), arcname="plain.txt")
    made.append(path)
    path = os.path.join(work, "archive.zip")
    with zipfile.ZipFile(path, "w") as handle:
        handle.write(os.path.join(work, "plain.txt"), "plain.txt")
    made.append(path)
    path = os.path.join(work, "database.sqlite")
    connection = sqlite3.connect(path)
    connection.execute("create table t(a)")
    connection.commit()
    connection.close()
    made.append(path)
    path = os.path.join(work, "sound.wav")
    handle = wave.open(path, "wb")
    handle.setnchannels(1)
    handle.setsampwidth(2)
    handle.setframerate(8000)
    handle.writeframes(bytes(400))
    handle.close()
    made.append(path)
    # A real executable, whichever kind this machine has.
    for candidate in ("/bin/ls", "/usr/bin/ls"):
        if os.path.exists(candidate):
            shutil.copy(candidate, os.path.join(work, "program.bin"))
            made.append(os.path.join(work, "program.bin"))
            break
    return sorted(made)


def main():
    if not shutil.which("file"):
        print("file(1) is needed")
        return 1
    work = tempfile.mkdtemp()
    try:
        paths = build(work)
        got = subprocess.run([GRASM, "run", HARNESS, "--"] + paths,
                             capture_output=True, text=True)
        if got.returncode != 0:
            print(got.stderr[-2000:])
            return 1
        lines = got.stdout.split("\n")
        agree = differ = named = 0
        for i, path in enumerate(paths):
            answer = subprocess.run(["file", "--mime-type", "-b", path],
                                    capture_output=True, text=True)
            want = answer.stdout.split("\n")[0].strip()
            mine = lines[i].split("\x02")[0]
            if mine == want:
                agree += 1
            elif want in BY_PARSING:
                named += 1
                print(f"  by parsing, not by signature: "
                      f"{os.path.basename(path)} -- file(1) {want}, "
                      f"this {mine}")
            elif mine in NEWER_THAN_FILE and want == "application/octet-stream":
                named += 1
                print(f"  newer than this file(1): {os.path.basename(path)}"
                      f" -- file(1) {want}, this {mine}")
            else:
                differ += 1
                print(f"  {os.path.basename(path)}: file(1) {want!r}, "
                      f"grasm {mine!r}")
        print(f"files: {len(paths)}   agreeing: {agree}   "
              f"differing: {differ}   named: {named}")
        return 1 if differ else 0
    finally:
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
