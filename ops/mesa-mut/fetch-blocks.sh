#!/usr/bin/env bash
#
# Fetch precomputed blocks for the `mesa-mut` (a.k.a. `hetzner-pre-mesa-1`)
# Mina network from the public GCS bucket and rename them into the filename
# convention the Mina Indexer expects.
#
# The indexer parses block file names as `<network>-<height>-<state_hash>.json`
# and splits on the FIRST dash to obtain the network, then requires the next
# segment to parse as a u32 height (see `extract_network_height_hash` in
# rust/src/block/mod.rs). The bucket stores blocks as
#   hetzner-pre-mesa-1-<height>-<state_hash>.json
# whose multi-dash prefix would make the parser read "pre" as the height and
# panic. We therefore rewrite the prefix to a single token: `mesa`.
#
# Usage:
#   ops/mesa-mut/fetch-blocks.sh [OUTPUT_DIR] [MAX_BLOCKS]
#
#   OUTPUT_DIR   Destination directory for the renamed blocks
#                (default: ./mesa-mut-blocks)
#   MAX_BLOCKS   Stop after downloading this many blocks (default: 200)
#
set -euo pipefail

BUCKET="mesa-hf-precomputed-blocks"
SRC_PREFIX="hetzner-pre-mesa-1"
DST_NETWORK="mesa"

OUT_DIR="${1:-./mesa-mut-blocks}"
MAX_BLOCKS="${2:-200}"

mkdir -p "$OUT_DIR"

echo "Fetching up to $MAX_BLOCKS mesa-mut precomputed blocks into $OUT_DIR" >&2

page_token=""
downloaded=0

while :; do
  url="https://storage.googleapis.com/storage/v1/b/${BUCKET}/o?maxResults=1000&fields=items(name),nextPageToken"
  if [[ -n "$page_token" ]]; then
    url="${url}&pageToken=${page_token}"
  fi

  resp="$(curl -fsS "$url")"

  # Extract object names (one per line) without jq.
  names="$(printf '%s' "$resp" | grep -oE '"name": "[^"]+\.json"' | sed -E 's/"name": "(.*)"/\1/')"

  while IFS= read -r name; do
    [[ -z "$name" ]] && continue

    # hetzner-pre-mesa-1-<height>-<hash>.json  ->  mesa-<height>-<hash>.json
    rest="${name#"${SRC_PREFIX}"-}"          # <height>-<hash>.json
    dst="${DST_NETWORK}-${rest}"

    if [[ -f "$OUT_DIR/$dst" ]]; then
      continue
    fi

    curl -fsS "https://storage.googleapis.com/${BUCKET}/${name}" -o "$OUT_DIR/$dst"
    downloaded=$((downloaded + 1))

    if (( downloaded % 25 == 0 )); then
      echo "  downloaded $downloaded blocks..." >&2
    fi
    if (( downloaded >= MAX_BLOCKS )); then
      echo "Done: $downloaded blocks in $OUT_DIR" >&2
      exit 0
    fi
  done <<< "$names"

  page_token="$(printf '%s' "$resp" | grep -oE '"nextPageToken": "[^"]+"' | sed -E 's/"nextPageToken": "(.*)"/\1/' || true)"
  if [[ -z "$page_token" ]]; then
    break
  fi
done

echo "Done: $downloaded blocks in $OUT_DIR" >&2
