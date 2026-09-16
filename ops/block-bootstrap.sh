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
#   Result: bandwidth-bound instead of rate-limited -- tens of minutes rather
#   than the ~9 hours block-pull would need. It is not instant: devnet's range
#   has grown to ~46k objects / ~30 GB, so budget tens of minutes and size
#   the blocks volume accordingly. Ingesting what was downloaded is fast
#   (devnet's 11k blocks ingest in ~3 minutes), so the tip is reached quickly
#   and block-pull takes over from there.
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

on_disk() { find "$DIR" -maxdepth 1 -name "${NET}-*.json" -printf '.' 2>/dev/null | wc -c; }

# Report progress while the fetch runs.
#
# The fetch below prints nothing for as long as it takes, and for devnet's full
# range that is tens of minutes and tens of GB (~680 KiB per block on average,
# so ~30 GB for 46k objects -- it is bandwidth-bound, not latency-bound). A
# healthy download and a stalled one therefore look identical in the log, which
# is how "no output for 45 minutes" gets read as a hang. Print a line every
# BOOTSTRAP_PROGRESS_SECS so the difference is visible.
PROGRESS_SECS="${BOOTSTRAP_PROGRESS_SECS:-30}"
BASE="$(on_disk)"      # blocks already present, so the rate reflects THIS run
START_TS="$(date +%s)"

progress_loop() {
  local now elapsed done_n fetched pct size eta
  while :; do
    sleep "$PROGRESS_SECS"
    done_n="$(on_disk)"
    fetched=$((done_n - BASE))
    now="$(date +%s)"
    elapsed=$((now - START_TS))
    pct=0
    [ "$COUNT" -gt 0 ] && pct=$((done_n * 100 / COUNT))
    size="$(du -sh "$DIR" 2>/dev/null | cut -f1)"
    eta=""
    if [ "$fetched" -gt 0 ] && [ "$elapsed" -gt 0 ]; then
      eta=", ~$(((COUNT - done_n) * elapsed / fetched / 60)) min left"
    fi
    echo "block-bootstrap: $done_n/$COUNT objects (${pct}%), ${size:-?} on disk${eta}" >&2
  done
}

progress_loop &
progress_pid=$!
# shellcheck disable=SC2064  # expand the pid now, not at trap time
trap "kill $progress_pid 2>/dev/null" EXIT INT TERM

echo "$NAMES" | grep . | xargs -P "$WORKERS" -I{} bash -c 'fetch_one "$@"' _ {} "$DIR" "$OBJ"

kill "$progress_pid" 2>/dev/null
trap - EXIT INT TERM

echo "block-bootstrap: done — $(on_disk) blocks on disk, $(du -sh "$DIR" 2>/dev/null | cut -f1)" >&2
