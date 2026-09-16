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

# Hold the web port while we bootstrap.
#
# Everything below (genesis decompression, the bulk block fetch) runs BEFORE
# `mina-indexer server start` binds the port, and on a fresh instance that is
# minutes. Until then a probe gets `connection refused`, which looks like a
# crash rather than a long first boot -- and a livenessProbe would keep killing
# the container before the fetch ever completes.
#
# So a placeholder answers probes for that window with the same contract the
# real server uses: /healthz 200 (alive, do not restart), everything else 503
# {"status":"bootstrapping", phase, blocks_on_disk}. It is stopped just before
# the exec below, so the real server binds the same port.
#
# Set MINA_BOOTSTRAP_HEALTH_PORT=0 to disable.
BOOTSTRAP_HEALTH_PORT="${MINA_BOOTSTRAP_HEALTH_PORT:-8080}"
PHASE_FILE=/data/.bootstrap-phase
health_pid=""

phase() {
  echo "$1" > "$PHASE_FILE" 2>/dev/null || true
}

stop_bootstrap_health() {
  [ -n "$health_pid" ] || return 0
  kill "$health_pid" 2>/dev/null || true
  wait "$health_pid" 2>/dev/null || true
  health_pid=""
  rm -f "$PHASE_FILE" 2>/dev/null || true

  # Do not exec the real server until the port is actually free, otherwise it
  # fails to bind and the container dies at the finish line.
  for _ in $(seq 1 20); do
    (exec 3<>"/dev/tcp/127.0.0.1/$BOOTSTRAP_HEALTH_PORT") 2>/dev/null || return 0
    sleep 0.5
  done
  echo "warning: port $BOOTSTRAP_HEALTH_PORT still held after stopping the bootstrap responder" >&2
}

if [ "$BOOTSTRAP_HEALTH_PORT" -gt 0 ] 2>/dev/null; then
  phase starting
  bootstrap-health serve "$BOOTSTRAP_HEALTH_PORT" /data/blocks "$PHASE_FILE" &
  health_pid=$!
  # A failure here must not take the container down; the real server still comes up.
  trap 'stop_bootstrap_health' EXIT
fi

# Hardfork networks (mesa, devnet) ship a gzipped genesis ledger we decompress
# to /data on first boot; mainnet's ledger is embedded in the binary.
ledger_args=()
if [ -n "${GENESIS_GZ:-}" ]; then
  GEN="/data/${NETWORK}-genesis.json"
  if [ ! -s "$GEN" ]; then
    echo "first boot: decompressing the baked ${NETWORK} genesis ledger..." >&2
    phase decompressing-genesis-ledger
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
  phase fetching-blocks
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

# Hand the port over to the real server, which serves /healthz and /readyz from
# here on. It answers /readyz 503 "bootstrapping" until the fetched blocks are
# ingested, so the pod stays out of the Service without ever looking crashed.
stop_bootstrap_health
trap - EXIT

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
