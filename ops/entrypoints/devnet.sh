# Configless entrypoint for the live-devnet mina-indexer image.
# devnet is a hardfork network; the indexer roots at the genesis of its
# 2026-08-19 mesa-protocol fork (block 545434, embedded in the binary) and
# follows the tip from there. The fork's genesis ledger ships gzipped at
# /genesis/devnet.json.gz; the shared body decompresses it to /data on first boot.
#
# Network-specific variables; the shared body follows (concatenated from
# common.sh at image-build time).
NETWORK=devnet
GENESIS_HASH=3NLT7n4LiVo6U4LXr9BjCEp9712fP61uXkRC6hnRnB682A8f4HrJ
GENESIS_GZ=/genesis/devnet.json.gz
# o1Labs publishes a devnet snapshot here (ops/snapshot-publish.sh); the shared
# body restores it on first boot unless MINA_SNAPSHOT_URL overrides it.
DEFAULT_SNAPSHOT_URL=https://storage.googleapis.com/mina-indexer-snapshots/devnet/devnet-latest.tar
