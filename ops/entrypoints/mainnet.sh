# Configless entrypoint for the mainnet mina-indexer image.
# The mainnet genesis ledger + genesis block are embedded in the binary, so this
# needs nothing mounted: `docker run` and it self-initializes and follows the tip.
#
# Network-specific variables; the shared body follows (concatenated from
# common.sh at image-build time).
NETWORK=mainnet
GENESIS_HASH=3NKeMoncuHab5ScarV5ViyF16cJPT4taWNSaTLS64Dp67wuXigPZ
FETCH_EXE=/bin/block-pull
# mainnet: no GENESIS_GZ — the genesis ledger is embedded in the binary.
