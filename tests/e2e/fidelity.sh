#!/usr/bin/env bash
#
# End-to-end ledger-fidelity gate (issue #19, WS2).
#
# Boots the indexer against a small, vendored slice of real mainnet post-hardfork
# blocks (mainnet uses the *embedded* hardfork genesis ledger, so nothing external
# is needed), lets it ingest, then asserts every account's balance matches the
# `accounts_accessed` oracle in the source blocks via ops/fidelity-check.py.
#
# This is the correctness check that found the ledger bugs fixed in #87/#88/#90,
# run as a self-contained CI gate. Exits non-zero on any balance mismatch.
#
# Usage:  tests/e2e/fidelity.sh [path-to-mina-indexer-binary]
set -euo pipefail

TOP="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BIN="${1:-$TOP/rust/target/release/mina-indexer}"
BLOCKS="$TOP/rust/tests/data/mainnet-e2e"

# mainnet hardfork V2: embedded genesis ledger, ingest starts at 359605
GENESIS_HASH=3NK4BpDSekaqsG6tx8Nse2zJchRft2JpnbvMiog55WCr5xJZaKeP
PORT="${E2E_WEB_PORT:-8091}"
GQL="http://localhost:${PORT}/graphql"
# Slice top and the settled height to check. The indexer is idle once ingest
# finishes, so a small margin is safe (no moving-tip settling in a static test).
SLICE_TOP=359624
MARGIN=8

[ -x "$BIN" ] || { echo "no indexer binary at $BIN (run 'rake build:release')" >&2; exit 2; }

WORK="$(mktemp -d)"
SOCK="$WORK/s"   # keep the socket path short (SUN_LEN limit)
LOG="$WORK/indexer.log"
cleanup() { [ -n "${PID:-}" ] && kill "$PID" 2>/dev/null || true; rm -rf "$WORK"; }
trap cleanup EXIT

echo "--- booting indexer against $(ls "$BLOCKS" | wc -l) vendored blocks"
ulimit -n 4096 || true
RUST_LOG=warn GIT_COMMIT_HASH="${GIT_COMMIT_HASH:-e2e}" "$BIN" \
  --socket "$SOCK" server start \
  --network mainnet --genesis-hash "$GENESIS_HASH" \
  --database-dir "$WORK/db" --blocks-dir "$BLOCKS" --web-port "$PORT" \
  >"$LOG" 2>&1 &
PID=$!

tip_height() {
  curl -s -m 5 -X POST "$GQL" -H 'Content-Type: application/json' \
    -d '{"query":"{ blocks(limit:1, sortBy: BLOCKHEIGHT_DESC, query:{canonical:true}) { blockHeight } }"}' \
    2>/dev/null | grep -oE '"blockHeight":[0-9]+' | grep -oE '[0-9]+' || true
}

echo "--- waiting for ingest to reach height $SLICE_TOP"
for _ in $(seq 1 120); do
  kill -0 "$PID" 2>/dev/null || { echo "indexer exited early:" >&2; tail -20 "$LOG" >&2; exit 1; }
  h="$(tip_height)"
  [ -n "$h" ] && [ "$h" -ge "$SLICE_TOP" ] && { echo "--- tip reached $h"; break; }
  sleep 2
done
h="$(tip_height)"
[ -n "$h" ] && [ "$h" -ge "$SLICE_TOP" ] || { echo "indexer did not reach $SLICE_TOP (got '${h:-none}')" >&2; tail -20 "$LOG" >&2; exit 1; }

echo "--- checking ledger fidelity vs accounts_accessed oracle"
python3 "$TOP/ops/fidelity-check.py" \
  --blocks-dir "$BLOCKS" --network mainnet --margin "$MARGIN" --gql "$GQL"
