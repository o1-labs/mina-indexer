#!/usr/bin/env bash
#
# Fetch precomputed blocks for the mesa-mut network from its public GCS bucket
# and rename them into the filename convention the Mina Indexer expects.
#
# mesa-mut blocks live in `gs://mesa-mut-precomputed-blocks`, named
#   mina-mesa-mut-1-<height>-<state_hash>.json
# The indexer parses block file names as `<network>-<height>-<hash>.json`,
# splitting on the FIRST dash for the network and requiring the next segment to
# be a u32 height (extract_network_height_hash, rust/src/block/mod.rs). The
# multi-dash `mina-mesa-mut-1-...` prefix would make it read "mesa" as the
# network and "mut" as the height and panic, so we rewrite the prefix to the
# single token `mesa` -> `mesa-<height>-<state_hash>.json`.
#
# mesa-mut is a hardfork at height 297734 (its genesis); ingest from there up.
#
# Usage:
#   ops/mesa-mut/fetch-blocks.sh OUTPUT_DIR [START_HEIGHT] [END_HEIGHT]
#
#   OUTPUT_DIR     destination directory for the renamed blocks
#   START_HEIGHT   first height to fetch (default 297735, i.e. genesis+1;
#                  the genesis block 297734 is embedded in the binary)
#   END_HEIGHT     last height to fetch (default START+199)
set -euo pipefail

BUCKET="mesa-mut-precomputed-blocks"
SRC_PREFIX="mina-mesa-mut-1"
DST_NETWORK="mesa"

OUT_DIR="${1:?usage: fetch-blocks.sh OUTPUT_DIR [START_HEIGHT] [END_HEIGHT]}"
START="${2:-297735}"
END="${3:-$((START + 199))}"

mkdir -p "$OUT_DIR"
echo "Fetching mesa-mut blocks ${START}..${END} into $OUT_DIR" >&2

downloaded=0
for h in $(seq "$START" "$END"); do
  # The bucket can hold several blocks per height (forks); take the first.
  name="$(curl -fsS \
    "https://storage.googleapis.com/storage/v1/b/${BUCKET}/o?prefix=${SRC_PREFIX}-${h}-&fields=items(name)" \
    | grep -oE '"name": "[^"]+\.json"' | head -1 | sed -E 's/"name": "(.*)"/\1/')"
  if [[ -z "$name" ]]; then
    echo "  (no block at height $h)" >&2
    continue
  fi

  # mina-mesa-mut-1-<height>-<hash>.json -> mesa-<height>-<hash>.json
  rest="${name#"${SRC_PREFIX}"-}"
  dst="${DST_NETWORK}-${rest}"
  if [[ -f "$OUT_DIR/$dst" ]]; then
    continue
  fi

  curl -fsS "https://storage.googleapis.com/${BUCKET}/${name}" -o "$OUT_DIR/$dst"
  downloaded=$((downloaded + 1))
  if (( downloaded % 25 == 0 )); then
    echo "  downloaded $downloaded blocks (through height $h)..." >&2
  fi
done

echo "Done: $downloaded new blocks in $OUT_DIR" >&2
