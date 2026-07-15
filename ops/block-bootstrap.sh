#!/usr/bin/env bash
#
# One-shot bulk block fetcher for the PUBLIC networks (mainnet, devnet), used to
# bootstrap a fresh instance:  block-bootstrap <network> <from_height> <blocks_dir> [workers]
#
# Why this exists, next to block-pull:
#
#   block-pull follows the TIP. The indexer calls it synchronously inside its
#   fetch/reconcile timer, so it deliberately keeps a small window (15 heights) and
#   lists one height per call — a slow fetch there starves the reconcile step that
#   actually ingests. That is exactly right for staying at the tip, and hopeless for
#   starting from nothing: devnet's ~7,700 blocks from its checkpoint would take
#   ~9 hours at 15 blocks/minute.
#
#   This does the same job for a bounded range, once, before the server starts:
#     - lists by height PREFIX (devnet-527, devnet-528, ...) instead of one API call
#       per height, turning thousands of list calls into a handful of paged ones;
#     - downloads in parallel.
#
#   Result: minutes instead of hours. Ingesting the downloaded blocks is fast
#   (devnet's 11k blocks ingest in ~3 minutes), so the tip is reached quickly and
#   block-pull takes over from there.
#
# Writes to a temp file and atomically renames, so the indexer's directory watcher
# never sees a half-written block. Re-running is safe: existing blocks are skipped.
set -uo pipefail

NET="${1:?usage: block-bootstrap <network> <from_height> <blocks_dir> [workers]}"
FROM="${2:?from_height}"
DIR="${3:?blocks_dir}"
WORKERS="${4:-${BOOTSTRAP_WORKERS:-16}}"

case "$NET" in
  mainnet | devnet) ;;
  *) echo "block-bootstrap: $NET is not a public network, nothing to do" >&2; exit 0 ;;
esac

BUCKET="${BLOCK_BUCKET:-mina_network_block_data}"
API="https://storage.googleapis.com/storage/v1/b/${BUCKET}/o"
OBJ="https://storage.googleapis.com/${BUCKET}"

mkdir -p "$DIR"

rcurl() { local i; for i in 1 2 3 4 5; do curl -fsS --max-time 120 "$@" && return 0; sleep $((i * 2)); done; return 1; }

# Is there any block at this height?
have_height() {
  local h="$1"
  rcurl "${API}?prefix=${NET}-${h}-&fields=items(name)&maxResults=1" | grep -q '"name"'
}

# Find the chain tip: double until we overshoot, then bisect. ~20 calls, versus
# listing the whole bucket (devnet has 500k+ objects).
find_tip() {
  local lo="$1" hi step=1
  while have_height $((lo + step)); do
    lo=$((lo + step))
    step=$((step * 2))
  done
  hi=$((lo + step))

  while [ $((hi - lo)) -gt 1 ]; do
    local mid=$(((lo + hi) / 2))
    if have_height "$mid"; then lo="$mid"; else hi="$mid"; fi
  done
  echo "$lo"
}

echo "block-bootstrap: finding the $NET tip from height $FROM..." >&2
TIP="$(find_tip "$FROM")"
echo "block-bootstrap: tip is $TIP; fetching $((TIP - FROM + 1)) heights into $DIR" >&2

# Every object name in [FROM, TIP], listed by shared height prefix. Blocks in this
# bucket are already named <network>-<height>-<hash>.json — the exact shape the
# indexer's filename parser wants — so nothing is renamed. Forks (several blocks at
# one height) are all fetched; the indexer picks the chain.
list_range() {
  local prefixes p token url page
  prefixes="$(seq "$FROM" "$TIP" | cut -c1-3 | sort -u)"

  for p in $prefixes; do
    token=""
    while :; do
      url="${API}?prefix=${NET}-${p}&fields=items(name),nextPageToken&maxResults=1000"
      [ -n "$token" ] && url="${url}&pageToken=${token}"
      page="$(rcurl "$url")" || break

      echo "$page" | grep -oE '"name": "[^"]+\.json"' | sed -E 's/"name": "(.*)"/\1/' |
        awk -F- -v lo="$FROM" -v hi="$TIP" '$2 >= lo && $2 <= hi'

      token="$(echo "$page" | grep -oE '"nextPageToken": "[^"]+"' | sed -E 's/.*: "(.*)"/\1/')"
      [ -z "$token" ] && break
    done
  done
}

fetch_one() {
  local name="$1" dir="$2" obj="$3"
  [ -s "$dir/$name" ] && return 0

  local tmp="$dir/.$name.part"
  if curl -fsS --max-time 300 --retry 5 --retry-delay 2 "$obj/$name" -o "$tmp"; then
    mv -f "$tmp" "$dir/$name"     # atomic: the watcher never sees a partial block
  else
    rm -f "$tmp"
    echo "block-bootstrap: FAILED $name" >&2
    return 1
  fi
}
export -f fetch_one

NAMES="$(list_range)"
COUNT="$(echo "$NAMES" | grep -c . || true)"
echo "block-bootstrap: $COUNT objects (forks included), $WORKERS workers" >&2

echo "$NAMES" | grep . | xargs -P "$WORKERS" -I{} bash -c 'fetch_one "$@"' _ {} "$DIR" "$OBJ"

echo "block-bootstrap: done — $(find "$DIR" -name "${NET}-*.json" | wc -l) blocks on disk" >&2
