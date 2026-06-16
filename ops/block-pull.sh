#!/usr/bin/env bash
#
# Block puller for the mina-indexer fetch/recovery hooks, for the PUBLIC networks
# (mainnet, devnet). Invoked by the indexer as:  block-pull <network> <height> <blocks_dir>
#   --fetch-new-blocks-exe       -> <height> is best_tip+1 (forward growth)
#   --missing-block-recovery-exe -> <height> is a missing block's height (gap fill)
#
# Downloads EVERY block at each height in [height, height+WINDOW) from the public
# `mina_network_block_data` bucket. Objects there are already named
# `<network>-<height>-<hash>.json` — exactly the form the indexer's filename
# parser expects — so no renaming is needed (unlike mesa-pull's multi-dash
# prefix). Writes to a temp file and atomically renames so the directory watcher
# never sees a half-written block. Retries transient GCS errors so it never
# leaves a gap.
set -uo pipefail

NET="${1:?usage: block-pull <network> <height> <blocks_dir>}"
FROM="${2:?height}"
DIR="${3:?blocks_dir}"
# This puller serves the public networks only; mesa is sourced from its own
# bucket by mesa-pull. Anything else is a no-op (exit 0 = nothing to fetch).
case "$NET" in
  mainnet | devnet) ;;
  *) exit 0 ;;
esac

# Small window on purpose: the indexer calls this synchronously inside its
# fetch/reconcile timer branch, so a slow fetch BLOCKS the reconcile step that
# actually ingests the downloaded blocks. The GCS list API costs ~2s per height,
# so keep the window well under fetch-new-blocks-delay (default 60s) — each call
# returns fast, reconcile runs every cycle, and the tip advances incrementally
# (catch-up just takes more cycles). Override with BLOCK_FETCH_WINDOW if needed.
WINDOW="${BLOCK_FETCH_WINDOW:-15}"
BUCKET="${BLOCK_BUCKET:-mina_network_block_data}"
API="https://storage.googleapis.com/storage/v1/b/${BUCKET}/o"
OBJ="https://storage.googleapis.com/${BUCKET}"

mkdir -p "$DIR"

rcurl() { local i; for i in 1 2 3 4 5; do curl -fsS --max-time 90 "$@" && return 0; sleep $((i * 2)); done; return 1; }

dl=0
for h in $(seq "$FROM" $((FROM + WINDOW - 1))); do
  # all blocks at this height (forks included), prefix is `<network>-<height>-`
  names="$(rcurl "${API}?prefix=${NET}-${h}-&fields=items(name)" | grep -oE '"name": "[^"]+\.json"' | sed -E 's/"name": "(.*)"/\1/')" || continue
  [ -z "$names" ] && continue
  while IFS= read -r n; do
    [ -z "$n" ] && continue
    dst="$n"   # already <network>-<height>-<hash>.json
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
echo "block-pull: ${dl} new ${NET} blocks from height ${FROM} (window ${WINDOW})"
