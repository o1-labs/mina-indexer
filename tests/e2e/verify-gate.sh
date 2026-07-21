#!/usr/bin/env bash
#
# Trustless verify-block gate — negative-path test (issue #19, WS5).
#
# The `--verify-block-exe` gate must be fail-closed: a block whose proof does not
# verify, or that the verifier cannot be run against, is NEVER ingested. This
# exercises the *startup bulk-load* path (blocks already in --blocks-dir at boot),
# which is the one that historically bypassed the gate.
#
# Three cases, each on a fresh DB against the same vendored mainnet slice:
#   accept  (verifier exits 0)      -> blocks ingested, tip advances
#   reject  (verifier exits 1)      -> nothing ingested, blocks quarantined
#   missing (verifier does not run) -> nothing ingested (fail-closed)
#
# Usage:  tests/e2e/verify-gate.sh [path-to-mina-indexer-binary]
set -euo pipefail

TOP="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BIN="${1:-$TOP/rust/target/release/mina-indexer}"
BLOCKS="$TOP/rust/tests/data/mainnet-e2e"
GENESIS_HASH=3NK4BpDSekaqsG6tx8Nse2zJchRft2JpnbvMiog55WCr5xJZaKeP
GENESIS_HEIGHT=359605   # hardfork genesis; "nothing ingested" == tip stays here
SLICE_TOP=359624        # top of the vendored slice; "ingested" == tip reaches here

[ -x "$BIN" ] || { echo "no indexer binary at $BIN (run 'rake build:release')" >&2; exit 2; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"; [ -n "${PID:-}" ] && kill "$PID" 2>/dev/null || true' EXIT

# Verifier stubs.
ACCEPT="$WORK/accept.sh"; printf '#!/bin/sh\nexit 0\n' > "$ACCEPT"; chmod +x "$ACCEPT"
REJECT="$WORK/reject.sh"; printf '#!/bin/sh\nexit 1\n' > "$REJECT"; chmod +x "$REJECT"
MISSING="$WORK/does-not-exist"   # deliberately absent

port=8101
tip_height() {
  curl -s -m 5 -X POST "http://localhost:$1/graphql" -H 'Content-Type: application/json' \
    -d '{"query":"{ blocks(limit:1, sortBy: BLOCKHEIGHT_DESC, query:{canonical:true}) { blockHeight } }"}' \
    2>/dev/null | grep -oE '"blockHeight":[0-9]+' | grep -oE '[0-9]+' || true
}

# Boot the indexer against a fresh copy of the slice with the given verifier, wait
# for it to settle, and echo the resulting canonical tip height.
run_case() {
  local verifier="$1" data="$WORK/blocks.$2" db="$WORK/db.$2" log="$WORK/log.$2"
  port=$((port + 1))
  mkdir -p "$data"; tar xzf "$BLOCKS/blocks.tar.gz" -C "$data"
  ulimit -n 4096 || true
  RUST_LOG=warn GIT_COMMIT_HASH="${GIT_COMMIT_HASH:-e2e}" "$BIN" \
    --socket "$WORK/s.$2" server start \
    --network mainnet --genesis-hash "$GENESIS_HASH" \
    --database-dir "$db" --blocks-dir "$data" --web-port "$port" \
    --verify-block-exe "$verifier" >"$log" 2>&1 &
  PID=$!

  # Wait for the web server to answer.
  local tip=""
  for _ in $(seq 1 90); do
    kill -0 "$PID" 2>/dev/null || { echo "indexer exited early ($2):" >&2; tail -15 "$log" >&2; exit 1; }
    tip="$(tip_height "$port")"
    [ -n "$tip" ] && break
    sleep 2
  done

  # The startup gate runs synchronously in initialize(), before the web server
  # comes up. So by now `.rejected/` is already populated iff blocks were
  # quarantined. If it is, nothing will ingest -- read the (genesis) tip and
  # return. Otherwise this is the accept case: wait for the blocks to ingest.
  if [ -d "$data/.rejected" ] && [ -n "$(ls -A "$data/.rejected" 2>/dev/null)" ]; then
    tip="$(tip_height "$port")"
  else
    for _ in $(seq 1 90); do
      tip="$(tip_height "$port")"
      [ "${tip:-0}" -ge "$SLICE_TOP" ] 2>/dev/null && break
      sleep 2
    done
  fi
  kill "$PID" 2>/dev/null || true; wait "$PID" 2>/dev/null || true; PID=""
  echo "${tip:-none}|$data"
}

fail() { echo "FAIL: $1" >&2; exit 1; }

echo "--- case: accepting verifier (exit 0) must ingest"
IFS='|' read -r tip data < <(run_case "$ACCEPT" accept)
[ "$tip" -ge "$SLICE_TOP" ] 2>/dev/null || fail "accept: tip $tip did not reach $SLICE_TOP"
echo "    ok: tip=$tip (ingested)"

echo "--- case: rejecting verifier (exit 1) must ingest nothing + quarantine"
IFS='|' read -r tip data < <(run_case "$REJECT" reject)
[ "${tip:-0}" -le "$GENESIS_HEIGHT" ] 2>/dev/null || fail "reject: tip $tip advanced past genesis $GENESIS_HEIGHT"
[ -d "$data/.rejected" ] && [ -n "$(ls -A "$data/.rejected" 2>/dev/null)" ] || fail "reject: no blocks quarantined in .rejected/"
echo "    ok: tip=$tip (genesis only), $(ls "$data/.rejected" | wc -l) block(s) quarantined"

echo "--- case: unavailable verifier must fail closed (ingest nothing)"
IFS='|' read -r tip data < <(run_case "$MISSING" missing)
[ "${tip:-0}" -le "$GENESIS_HEIGHT" ] 2>/dev/null || fail "missing: tip $tip advanced past genesis (not fail-closed)"
[ -d "$data/.rejected" ] && [ -n "$(ls -A "$data/.rejected" 2>/dev/null)" ] || fail "missing: no blocks quarantined (not fail-closed)"
echo "    ok: tip=$tip (genesis only), fail-closed"

echo "PASS: verify-block gate is fail-closed on the startup bulk-load path"
