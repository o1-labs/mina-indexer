#!/usr/bin/env bash
#
# Block-verification shim for the indexer's --verify-block-exe hook.
# The indexer invokes it as:  verify-block <network> <block-file>
# and ingests the block only if this exits 0.
#
# It forwards the precomputed-block JSON to the mina-verify-server sidecar, which
# checks the block's SNARK proof and answers { "valid": true/false, ... }.
# Exit 0 = proof verified (ingest); non-zero = reject / verifier unreachable.
set -uo pipefail

NET="${1:?usage: verify-block <network> <block-file>}"
FILE="${2:?block-file}"
: "$NET" # network is selected by the verifier's own config (MINA_NETWORK / MINA_VK_JSON)

ENDPOINT="${VERIFY_ENDPOINT:-http://mina-verifier:8090/verify}"

resp="$(curl -fsS --max-time 30 -H 'Content-Type: application/json' \
  --data-binary "@${FILE}" "$ENDPOINT")" || {
  echo "verify-block: verifier unreachable at $ENDPOINT" >&2
  exit 2 # fail closed — the indexer treats any non-zero as "do not ingest"
}

case "$resp" in
  *'"valid":true'*) exit 0 ;;
  *)
    echo "verify-block: rejected ${FILE}: ${resp}" >&2
    exit 1
    ;;
esac
