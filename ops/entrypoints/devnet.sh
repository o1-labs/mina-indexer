#!/usr/bin/env bash
#
# Configless entrypoint for the live-devnet mina-indexer image.
# devnet is a hardfork network; the indexer roots at a recent published state-dump
# checkpoint (block 527922, embedded in the binary) and follows the tip from there.
# The matching genesis ledger ships gzipped at /genesis/devnet.json.gz; we
# decompress it to /data on first boot, then follow the tip from the public bucket.
set -euo pipefail

GEN=/data/devnet-genesis.json
if [ ! -s "$GEN" ]; then
  echo "first boot: decompressing the baked devnet genesis ledger..." >&2
  gunzip -c /genesis/devnet.json.gz > "$GEN"
fi

exec mina-indexer --socket /data/mi.sock server start \
  --network devnet \
  --genesis-hash 3NK4DL35iKQ6G8VPqPFLZ122M82dcRRPt8rHrpRW662kXWpH8fRa \
  --genesis-ledger "$GEN" \
  --database-dir /data/db \
  --blocks-dir /data/blocks \
  --fetch-new-blocks-exe /bin/block-pull --fetch-new-blocks-delay 60 \
  --missing-block-recovery-exe /bin/block-pull --missing-block-recovery-delay 120
