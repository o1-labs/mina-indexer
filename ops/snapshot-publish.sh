# Publish a database snapshot of the running indexer, on a schedule.
#
# Runs next to the indexer (a sidecar in the same pod, sharing /data and the
# server socket). A cold start restores the newest published snapshot
# (MINA_SNAPSHOT_URL) and fetches only the blocks after it, so the snapshot
# must stay newer than the oldest block still in the public block bucket. This
# loop keeps it fresh:
#
#   1. Once per MINA_SNAPSHOT_CHECK_SECS, read `created_at` of the published
#      <net>-latest.manifest.json. Continue only when it is older than
#      MINA_SNAPSHOT_PUBLISH_INTERVAL_SECS (or absent), so a pod restart does
#      not publish again.
#   2. Wait for /readyz 200: publish only a database that is at the tip.
#   3. Take an online snapshot over the socket, check that its manifest belongs
#      to this network's genesis, and upload to gs://<bucket>/<net>/:
#        <net>-<height>-<hash>-store-<version>.tar  + .manifest.json
#        <net>-latest.tar  + <net>-latest.manifest.json  (Cache-Control: no-cache)
#      The latest manifest is written last, so it never names a tar that is not
#      there yet.
#
# A failure is logged and retried on the next check; the loop never exits, so a
# publisher problem never takes the indexer's pod out of service.
#
# The per-network file (mainnet.sh / devnet.sh) is concatenated AHEAD of this
# one at image-build time and sets NETWORK and GENESIS_HASH.
#
# Environment:
#   GOOGLE_APPLICATION_CREDENTIALS  service account key with write access to the
#                                   bucket (required; without it the loop idles)
#   MINA_SNAPSHOT_BUCKET            default mina-indexer-snapshots
#   MINA_SNAPSHOT_PUBLISH_INTERVAL_SECS  default 604800 (7 days)
#   MINA_SNAPSHOT_CHECK_SECS        default 3600
#   MINA_SOCKET                     default /data/mi.sock
#   MINA_WEB_PORT                   default 8080
#   MINA_SNAPSHOT_STAGING_DIR       default /data/snapshots (needs free space
#                                   of about the database size)

BUCKET="${MINA_SNAPSHOT_BUCKET:-mina-indexer-snapshots}"
INTERVAL="${MINA_SNAPSHOT_PUBLISH_INTERVAL_SECS:-604800}"
CHECK="${MINA_SNAPSHOT_CHECK_SECS:-3600}"
SOCKET="${MINA_SOCKET:-/data/mi.sock}"
WEB_PORT="${MINA_WEB_PORT:-8080}"
STAGING="${MINA_SNAPSHOT_STAGING_DIR:-/data/snapshots}"
KEY="${GOOGLE_APPLICATION_CREDENTIALS:-}"
PUBLIC="https://storage.googleapis.com/${BUCKET}/${NETWORK}"
# the remote is given on the command line; no config file (the root fs is read-only)
export RCLONE_CONFIG="${RCLONE_CONFIG:-/tmp/rclone.conf}"

log() {
  echo "snapshot-publish: $*" >&2
}

rc() {
  rclone --gcs-service-account-file "$KEY" --gcs-bucket-policy-only \
    --retries 5 --low-level-retries 20 --stats 0 "$@"
}

# seconds since the published latest snapshot, or a large number if none
published_age() {
  local created
  created="$(curl -fsSL "${PUBLIC}/${NETWORK}-latest.manifest.json" 2>/dev/null | jq -r '.created_at // empty' 2>/dev/null)" || true
  if [ -z "$created" ]; then
    echo 999999999
    return
  fi
  echo $(($(date -u +%s) - $(date -u -d "$created" +%s)))
}

publish() {
  rm -rf "$STAGING"
  mkdir -p "$STAGING"
  local tar="${STAGING}/snapshot.tar"

  log "taking an online snapshot via ${SOCKET}"
  mina-indexer --socket "$SOCKET" database snapshot --output-path "$tar" || return 1
  [ -s "$tar" ] || {
    log "snapshot file is empty or missing"
    return 1
  }

  local manifest genesis height hash store
  manifest="$(tar -xOf "$tar" SNAPSHOT_MANIFEST)" || return 1
  genesis="$(jq -r '.genesis_state_hash // empty' <<<"$manifest")"
  height="$(jq -r '.best_block_height // empty' <<<"$manifest")"
  hash="$(jq -r '.best_block_hash // empty' <<<"$manifest")"
  # {"major":0,"minor":19,"patch":0,...} -> 0.19.0
  store="$(jq -r '.store_version | if type == "object" then "\(.major).\(.minor).\(.patch)" else (. // empty) end' <<<"$manifest")"
  if [ "$genesis" != "$GENESIS_HASH" ] || [ -z "$height" ] || [ -z "$hash" ] || [ -z "$store" ]; then
    log "snapshot manifest does not match ${NETWORK} (${GENESIS_HASH}): ${manifest}"
    return 1
  fi

  local name="${NETWORK}-${height}-${hash}-store-${store}"
  local bytes sha256
  bytes="$(stat -c %s "$tar")"
  sha256="$(sha256sum "$tar" | cut -d' ' -f1)"
  jq -n \
    --arg network "$NETWORK" \
    --arg object "${NETWORK}/${name}.tar" \
    --argjson bytes "$bytes" \
    --arg sha256 "$sha256" \
    --argjson snapshot_manifest "$manifest" \
    --arg created_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --arg built_with "$(mina-indexer version 2>/dev/null | head -1)" \
    --arg restore "curl -fsSL ${PUBLIC}/${name}.tar | mina-indexer database restore --snapshot-file - --restore-dir /data/db --genesis-hash ${GENESIS_HASH}" \
    '{format: 1, network: $network, object: $object, bytes: $bytes, sha256: $sha256,
      snapshot_manifest: $snapshot_manifest, created_at: $created_at,
      built_with: $built_with, restore: $restore}' >"${STAGING}/manifest.json" || return 1

  local remote=":gcs:${BUCKET}/${NETWORK}"
  log "uploading ${name}.tar (${bytes} bytes, sha256 ${sha256})"
  # upload once, to the mutable latest object (a server-side copy keeps the
  # default caching, so the no-cache header must come with the upload), then
  # copy it server side to the immutable versioned name
  rc --header-upload "Cache-Control: no-cache" copyto "$tar" "${remote}/${NETWORK}-latest.tar" || return 1
  rc copyto "${remote}/${NETWORK}-latest.tar" "${remote}/${name}.tar" || return 1
  rc copyto "${STAGING}/manifest.json" "${remote}/${name}.manifest.json" || return 1
  rc --header-upload "Cache-Control: no-cache" copyto "${STAGING}/manifest.json" "${remote}/${NETWORK}-latest.manifest.json" || return 1
  log "published ${PUBLIC}/${NETWORK}-latest.tar (${name})"
}

if [ -z "$KEY" ] || [ ! -s "$KEY" ]; then
  log "GOOGLE_APPLICATION_CREDENTIALS is not set or empty; not publishing"
  exec sleep infinity
fi

log "${NETWORK}: publishing to gs://${BUCKET}/${NETWORK}/ every ${INTERVAL}s (checked every ${CHECK}s)"
while true; do
  age="$(published_age)"
  if [ "$age" -lt "$INTERVAL" ]; then
    log "the published snapshot is ${age}s old; next check in ${CHECK}s"
  elif [ "$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:${WEB_PORT}/readyz")" != "200" ]; then
    log "the indexer is not ready (not at the tip); next check in ${CHECK}s"
  elif publish; then
    rm -rf "$STAGING"
  else
    log "publish failed; retry in ${CHECK}s"
    rm -rf "$STAGING"
  fi
  sleep "$CHECK"
done
