# Configless entrypoint for the mainnet mina-indexer image.
# mainnet is a hardfork network; the indexer roots at the genesis of its
# 2026-09-03 mesa-protocol fork (block 548147, embedded in the binary) and
# follows the tip from there. The fork's genesis ledger ships gzipped at
# /genesis/mainnet.json.gz; the shared body decompresses it to /data on first boot.
#
# Network-specific variables; the shared body follows (concatenated from
# common.sh at image-build time).
NETWORK=mainnet
GENESIS_HASH=3NLntCRR9RvjtzP24Q1dRvdhmpjiPvfKGaJmAZuJgYoqGpevqDR7
GENESIS_GZ=/genesis/mainnet.json.gz
# o1Labs publishes a mainnet snapshot here (ops/snapshot-publish.sh); the shared
# body restores it on first boot unless MINA_SNAPSHOT_URL overrides it.
DEFAULT_SNAPSHOT_URL=https://storage.googleapis.com/mina-indexer-snapshots/mainnet/mainnet-latest.tar
