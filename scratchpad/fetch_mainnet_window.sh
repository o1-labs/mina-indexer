#!/usr/bin/env bash
# Bounded parallel fetch of mainnet blocks in [FROM, TO] from the public bucket.
# Lists by shared height prefix (359, 360, ...) — a few paged calls instead of one
# per height — then downloads in parallel. Existing files are skipped.
set -uo pipefail

FROM="${1:?from}"
TO="${2:?to}"
DIR="${3:?dir}"
WORKERS="${4:-24}"
NET=mainnet
API="https://storage.googleapis.com/storage/v1/b/mina_network_block_data/o"
OBJ="https://storage.googleapis.com/mina_network_block_data"

mkdir -p "$DIR"
rcurl() { local i; for i in 1 2 3 4 5; do curl -fsS --max-time 120 "$@" && return 0; sleep $((i*2)); done; return 1; }

# distinct 3-digit height prefixes covering [FROM, TO]
prefixes="$(seq "$FROM" "$TO" | cut -c1-3 | sort -u)"
list="$(mktemp)"
for p in $prefixes; do
  token=""
  while :; do
    url="${API}?prefix=${NET}-${p}&fields=items(name),nextPageToken&maxResults=1000"
    [ -n "$token" ] && url="${url}&pageToken=${token}"
    page="$(rcurl "$url")" || break
    echo "$page" | grep -oE '"name": "[^"]+\.json"' | sed -E 's/"name": "(.*)"/\1/' >> "$list"
    token="$(echo "$page" | grep -oE '"nextPageToken": "[^"]+"' | sed -E 's/.*"nextPageToken": "(.*)"/\1/')"
    [ -z "$token" ] && break
  done
done

# filter to [FROM, TO] by parsed height (field 2 of mainnet-<h>-<hash>.json)
awk -F- -v lo="$FROM" -v hi="$TO" '{h=$2+0; if (h>=lo && h<=hi) print}' "$list" | sort -u > "${list}.f"
n=$(wc -l < "${list}.f")
echo "listing done: $n block objects in [$FROM,$TO]; downloading with $WORKERS workers..." >&2

export DIR OBJ
cat "${list}.f" | xargs -P "$WORKERS" -I{} bash -c '
  dst="$DIR/{}"
  [ -f "$dst" ] && exit 0
  for i in 1 2 3; do
    if curl -fsS --max-time 120 "$OBJ/{}" -o "$dst.tmp"; then mv "$dst.tmp" "$dst"; exit 0; fi
    sleep $((i*2))
  done
  echo "FAILED {}" >&2
'
echo "download done: $(ls "$DIR" | wc -l) files in $DIR" >&2
rm -f "$list" "${list}.f"
