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

exec mina-indexer --socket /data/mi.sock server start \
  --network mesa \
  --genesis-hash 3NKQttwm8QRdvSZL62Lid8YAPCXBuAucZPDT8mJriHmw2qk9cVcr \
  --genesis-ledger "$GEN" \
  --database-dir /data/db \
  --blocks-dir /data/blocks \
  --fetch-new-blocks-exe /bin/mesa-pull --fetch-new-blocks-delay 60 \
  --missing-block-recovery-exe /bin/mesa-pull --missing-block-recovery-delay 120
