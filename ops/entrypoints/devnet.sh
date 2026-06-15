#!/usr/bin/env bash
#
# Configless entrypoint for the devnet mina-indexer image.
# The devnet genesis ledger ships gzipped at /genesis/devnet.json.gz; we
# decompress it to /data on first boot, then follow the tip from the public bucket.
#
# NOTE: the --genesis-hash below must match DEVNET_GENESIS_HASH in
# rust/src/constants.rs so the binary selects the V2/devnet config.
set -euo pipefail

GEN=/data/devnet-genesis.json
if [ ! -s "$GEN" ]; then
  echo "first boot: decompressing the baked devnet genesis ledger..." >&2
  gunzip -c /genesis/devnet.json.gz > "$GEN"
fi

exec mina-indexer --socket /data/mi.sock server start \
  --network devnet \
  --genesis-hash 3NK2tkzqqK5spR2sZ7tujjqPksL45M3UUrcA4WhCkeiPtnugyE2x \
  --genesis-ledger "$GEN" \
  --database-dir /data/db \
  --blocks-dir /data/blocks \
  --fetch-new-blocks-exe /bin/block-pull --fetch-new-blocks-delay 60 \
  --missing-block-recovery-exe /bin/block-pull --missing-block-recovery-delay 120
