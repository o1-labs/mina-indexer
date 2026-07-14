# Shared body for the configless per-network image entrypoints.
#
# The per-network file (mainnet.sh / devnet.sh / mesa.sh) is concatenated AHEAD
# of this one at image-build time (see flake.nix `mkEntry`) and sets:
#   NETWORK       — --network value
#   GENESIS_HASH  — --genesis-hash value
#   FETCH_EXE     — fetcher for --fetch-new-blocks-exe / --missing-block-recovery-exe
#   GENESIS_GZ    — (hardfork networks only) baked genesis ledger to decompress
#
# Composed at build time rather than sourced at runtime so each image's
# entrypoint stays a single self-contained script.

# hourly consistent DB checkpoints to /data/checkpoints/latest (override the
# cadence with MINA_CHECKPOINT_INTERVAL_SECS); a crash resumes from it instead
# of replaying a large WAL.
export MINA_CHECKPOINT_DIR="${MINA_CHECKPOINT_DIR:-/data/checkpoints}"

# Hardfork networks (mesa, devnet) ship a gzipped genesis ledger we decompress
# to /data on first boot; mainnet's ledger is embedded in the binary.
ledger_args=()
if [ -n "${GENESIS_GZ:-}" ]; then
  GEN="/data/${NETWORK}-genesis.json"
  if [ ! -s "$GEN" ]; then
    echo "first boot: decompressing the baked ${NETWORK} genesis ledger..." >&2
    gunzip -c "$GENESIS_GZ" > "$GEN"
  fi
  ledger_args=(--genesis-ledger "$GEN")
fi

# First boot: bulk-fetch the backlog in parallel.
#
# FETCH_EXE follows the *tip*. The indexer calls it synchronously inside its
# fetch/reconcile timer, so it keeps a deliberately small window (15 heights) --
# a slow fetch there starves the reconcile step that ingests what was fetched.
# That is right for staying at the tip and hopeless for starting from nothing:
# devnet's ~7,700 blocks from its checkpoint would take ~9 hours at that rate.
#
# So on a fresh instance (no DB yet) we bulk-fetch the range once, in parallel,
# which takes minutes. Ingesting it is fast (devnet: ~11k blocks in ~3 minutes),
# and FETCH_EXE takes over at the tip. Networks that set no BOOTSTRAP_FROM (i.e.
# mainnet, whose history is far too large to pull this way) simply skip it.
#
# Set BOOTSTRAP_FROM=0 to disable; BOOTSTRAP_WORKERS tunes the parallelism.
if [ -n "${BOOTSTRAP_FROM:-}" ] && [ "${BOOTSTRAP_FROM}" -gt 0 ] && [ ! -d /data/db ]; then
  echo "first boot: bulk-fetching ${NETWORK} blocks from height ${BOOTSTRAP_FROM}..." >&2
  block-bootstrap "$NETWORK" "$BOOTSTRAP_FROM" /data/blocks "${BOOTSTRAP_WORKERS:-16}" ||
    echo "bootstrap incomplete; the fetcher will fill the gaps (slowly)" >&2
fi

# Bound /data/blocks growth: keep only recent block files on disk (older blocks
# already live in the speedb DB and are never re-read). Tune with
# MINA_BLOCKS_RETENTION_LENGTH; set it to 0 to disable and keep every block.
RETENTION="${MINA_BLOCKS_RETENTION_LENGTH:-1000}"
retention_args=()
if [ "$RETENTION" -gt 0 ] 2>/dev/null; then
  retention_args=(--blocks-retention-length "$RETENTION")
fi

exec mina-indexer --socket /data/mi.sock server start \
  --network "$NETWORK" \
  --genesis-hash "$GENESIS_HASH" \
  ${ledger_args[@]+"${ledger_args[@]}"} \
  --restore-from-checkpoint "$MINA_CHECKPOINT_DIR" \
  --database-dir /data/db \
  --blocks-dir /data/blocks \
  --fetch-new-blocks-exe "$FETCH_EXE" --fetch-new-blocks-delay 60 \
  --missing-block-recovery-exe "$FETCH_EXE" --missing-block-recovery-delay 120 \
  ${retention_args[@]+"${retention_args[@]}"}
