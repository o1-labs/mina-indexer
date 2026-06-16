#!/usr/bin/env bash
#
# Configless entrypoint for the mainnet mina-indexer image.
# The mainnet genesis ledger + genesis block are embedded in the binary, so this
# needs nothing mounted: `docker run` and it self-initializes and follows the tip.
set -euo pipefail

# hourly consistent DB checkpoints to /data/checkpoints/latest (override the
# cadence with MINA_CHECKPOINT_INTERVAL_SECS); a crash resumes from it instead
# of replaying a large WAL.
export MINA_CHECKPOINT_DIR="${MINA_CHECKPOINT_DIR:-/data/checkpoints}"

exec mina-indexer --socket /data/mi.sock server start \
  --network mainnet \
  --genesis-hash 3NKeMoncuHab5ScarV5ViyF16cJPT4taWNSaTLS64Dp67wuXigPZ \
  --restore-from-checkpoint "$MINA_CHECKPOINT_DIR" \
  --database-dir /data/db \
  --blocks-dir /data/blocks \
  --fetch-new-blocks-exe /bin/block-pull --fetch-new-blocks-delay 60 \
  --missing-block-recovery-exe /bin/block-pull --missing-block-recovery-delay 120
