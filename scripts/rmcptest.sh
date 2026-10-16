#!/usr/bin/env bash
# Does `packages/rmcp` speak MCP to a client that did not come from here?
#
# The oracle is the official MCP Python SDK -- the same library a real client
# uses -- over the same stdio transport, driven as a subprocess. Agreement
# with my own idea of the protocol would prove nothing; three bugs here were
# found only because the real client refused what this server sent.
#
# Two parts:
#
#   mcpdrive.py  the handshake, then list and call tools, read a resource,
#                get a prompt, ping, and two refusals. All through the SDK.
#   mcpraw.py    lines a well-behaved client never sends -- a bare array, a
#                number, `null`, the wrong `jsonrpc`, no method, params of
#                the wrong type, two notifications. Every request must get
#                exactly one JSON-RPC reply and every notification none.
#
# The SDK is not vendored, so this skips when it is absent rather than
# failing: a missing oracle is not a defect in the server. It says which.
set -eu
cd "$(dirname "$0")/.."

GRASM=${GRASM:-./target/release/grasm}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# `.mcpvenv` is gitignored on purpose: an oracle kept in the repo can be
# edited to agree with us, which is the one thing an oracle must not do.
PY=""
for cand in "${MCP_PYTHON:-}" ./.mcpvenv/bin/python; do
    [ -n "$cand" ] && [ -x "$cand" ] &&
        "$cand" -c "import mcp" 2>/dev/null && { PY=$cand; break; }
done
if [ -z "$PY" ]; then
    # A skip is not a pass. `allgates.sh` reads the exit code, so exiting 0
    # here would make a gate that tested nothing report green -- which it did
    # once, when the search for the interpreter was wrong and nobody noticed
    # because the line said "skipped" and the code said fine.
    echo "    the mcp SDK is not here, so nothing was checked. Set it up with:"
    echo "      python3 -m venv .mcpvenv && ./.mcpvenv/bin/pip install mcp"
    echo "    or point MCP_PYTHON at a Python that already has it."
    exit 1
fi

if ! "$GRASM" build packages/rmcp-demo/src/main.gr --native -o "$work/srv" \
    >"$work/build.log" 2>&1; then
    echo "    the server did not build:"
    sed 's/^/      /' "$work/build.log"
    exit 1
fi

if ! "$PY" scripts/lib/mcpdrive.py "$work/srv" >"$work/drive.out" 2>&1; then
    echo "    the official client could not drive it:"
    tail -20 "$work/drive.out" | sed 's/^/      /'
    exit 1
fi
grep -q "^ALL OK$" "$work/drive.out" || { echo "    the client did not finish"; exit 1; }

# The handshake must settle on a version the client offered, not on ours.
proto=$(sed -n 's/^HANDSHAKE .*proto=//p' "$work/drive.out")
case "$proto" in
    2024-11-05|2025-03-26|2025-06-18|2025-11-25) ;;
    *) echo "    negotiated $proto, which is not a handshake version"; exit 1 ;;
esac

if ! "$PY" scripts/lib/mcpraw.py "$work/srv" >"$work/raw.out" 2>&1; then
    echo "    malformed input was mishandled:"
    sed 's/^/      /' "$work/raw.out"
    exit 1
fi

tools=$(sed -n "s/^TOOLS *//p" "$work/drive.out")
replies=$(sed -n 's/^sent .*got \([0-9]*\) replies.*/\1/p' "$work/raw.out")
echo "    official client: handshake $proto, tools $tools"
echo "    malformed lines: $replies replies, every request answered once"
[ "$replies" -ge 20 ]
