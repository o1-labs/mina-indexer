# Configless entrypoint for the live-devnet mina-indexer image.
# devnet is a hardfork network; the indexer roots at a recent published state-dump
# checkpoint (block 527922, embedded in the binary) and follows the tip from there.
# The matching genesis ledger ships gzipped at /genesis/devnet.json.gz; the shared
# body decompresses it to /data on first boot.
#
# Network-specific variables; the shared body follows (concatenated from
# common.sh at image-build time).
NETWORK=devnet
GENESIS_HASH=3NK4DL35iKQ6G8VPqPFLZ122M82dcRRPt8rHrpRW662kXWpH8fRa
FETCH_EXE=/bin/block-pull
GENESIS_GZ=/genesis/devnet.json.gz
