# Configless entrypoint for the mesa-mut mina-indexer image.
# The mesa genesis ledger (~900 MB) ships gzipped at /genesis/mesa.json.gz; the
# shared body decompresses it to /data on first boot, then follows the tip from
# the mesa bucket via mesa-pull.
#
# Network-specific variables; the shared body follows (concatenated from
# common.sh at image-build time).
NETWORK=mesa
GENESIS_HASH=3NKQttwm8QRdvSZL62Lid8YAPCXBuAucZPDT8mJriHmw2qk9cVcr
FETCH_EXE=/bin/mesa-pull
GENESIS_GZ=/genesis/mesa.json.gz
