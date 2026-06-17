#!/usr/bin/env bash
#
# Configless entrypoint for the mesa-mut mina-indexer image.
# The mesa genesis ledger (~900 MB) ships gzipped at /genesis/mesa.json.gz; we
# decompress it to /data on first boot, then follow the tip from the mesa bucket.
set -euo pipefail

GEN=/data/mesa-genesis.json
if [ ! -s "$GEN" ]; then
  echo "first boot: decompressing the baked mesa genesis ledger (~900 MB)..." >&2
  gunzip -c /genesis/mesa.json.gz > "$GEN"
fi

# hourly consistent DB checkpoints to /data/checkpoints/latest (override the
# cadence with MINA_CHECKPOINT_INTERVAL_SECS); a crash resumes from it instead
# of replaying a large WAL.
export MINA_CHECKPOINT_DIR="${MINA_CHECKPOINT_DIR:-/data/checkpoints}"

# Bound /data/blocks growth: keep only recent block files on disk (older blocks
# already live in the speedb DB and are never re-read). Tune with
# MINA_BLOCKS_RETENTION_LENGTH; set it to 0 to disable and keep every block.
RETENTION="${MINA_BLOCKS_RETENTION_LENGTH:-1000}"
retention_args=()
if [ "$RETENTION" -gt 0 ] 2>/dev/null; then
  retention_args=(--blocks-retention-length "$RETENTION")
fi

exec mina-indexer --socket /data/mi.sock server start \
  --network mesa \
  --genesis-hash 3NKQttwm8QRdvSZL62Lid8YAPCXBuAucZPDT8mJriHmw2qk9cVcr \
  --genesis-ledger "$GEN" \
  --restore-from-checkpoint "$MINA_CHECKPOINT_DIR" \
  --database-dir /data/db \
  --blocks-dir /data/blocks \
  --fetch-new-blocks-exe /bin/mesa-pull --fetch-new-blocks-delay 60 \
  --missing-block-recovery-exe /bin/mesa-pull --missing-block-recovery-delay 120 \
  ${retention_args[@]+"${retention_args[@]}"}
