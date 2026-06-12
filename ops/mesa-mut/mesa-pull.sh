#!/usr/bin/env bash
#
# Block puller for the mina-indexer fetch/recovery hooks. Invoked by the indexer
# as:  mesa-pull <network> <height> <blocks_dir>
#   --fetch-new-blocks-exe       -> <height> is best_tip+1 (forward growth)
#   --missing-block-recovery-exe -> <height> is a missing block's height (gap fill)
#
# Downloads EVERY block at each height in [height, height+WINDOW) from the
# mesa-mut precomputed-blocks bucket (all forks, so canonical-chain discovery can
# resolve mesa's heavy forking), renaming the multi-dash bucket prefix to the
# single-token `mesa-` the indexer's filename parser expects. Writes to a temp
# file and atomically renames, so the indexer's directory watcher never sees a
# half-written block. Retries transient GCS errors so it never leaves a gap.
set -uo pipefail

NET="${1:?usage: mesa-pull <network> <height> <blocks_dir>}"
FROM="${2:?height}"
DIR="${3:?blocks_dir}"
[ "$NET" = "mesa" ] || exit 0   # only mesa is sourced from this bucket

WINDOW="${MESA_FETCH_WINDOW:-200}"
# Never fetch below the mesa hard-fork genesis (297735). The block at 297734 is
# the RETIRED pre-fork chain (genesis 3NLp6dKN) with the old structure; the
# post-fork deserializer can't parse it ("missing field protocol_state") and the
# indexer panics. missing-block-recovery asks for 297734 because our genesis's
# previous_state_hash points there, but it must never be ingested.
MIN_HEIGHT="${MESA_MIN_HEIGHT:-297735}"
BUCKET="mesa-mut-precomputed-blocks"
SRC="mina-mesa-mut-1"
API="https://storage.googleapis.com/storage/v1/b/${BUCKET}/o"
OBJ="https://storage.googleapis.com/${BUCKET}"

mkdir -p "$DIR"

rcurl() { local i; for i in 1 2 3 4 5; do curl -fsS --max-time 90 "$@" && return 0; sleep $((i * 2)); done; return 1; }

dl=0
for h in $(seq "$FROM" $((FROM + WINDOW - 1))); do
  [ "$h" -lt "$MIN_HEIGHT" ] && continue   # skip pre-fork blocks (unparseable, would crash the indexer)
  names="$(rcurl "${API}?prefix=${SRC}-${h}-&fields=items(name)" | grep -oE '"name": "[^"]+\.json"' | sed -E 's/"name": "(.*)"/\1/')" || continue
  [ -z "$names" ] && continue
  while IFS= read -r n; do
    [ -z "$n" ] && continue
    dst="mesa-${n#"${SRC}"-}"
    [ -f "$DIR/$dst" ] && continue
    tmp="$DIR/.${dst}.part"
    if rcurl "${OBJ}/${n}" -o "$tmp"; then
      mv -f "$tmp" "$DIR/$dst"   # atomic: watcher only ever sees a complete file
      dl=$((dl + 1))
    else
      rm -f "$tmp"
    fi
  done <<< "$names"
done
echo "mesa-pull: ${dl} new blocks from height ${FROM} (window ${WINDOW})"
