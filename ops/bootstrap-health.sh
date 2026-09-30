#!/usr/bin/env bash
#
# Placeholder health endpoint for the FIRST-BOOT window, before the indexer's
# own web server binds the port:  bootstrap-health serve <port> <blocks_dir> <phase_file>
#
# Why this exists:
#
#   The entrypoint decompresses the genesis ledger and bulk-fetches the block
#   backlog (tens of thousands of blocks) BEFORE it execs `mina-indexer server
#   start`. Only then does the real server bind :8080 and start answering
#   /healthz and /readyz. For those first minutes nothing is listening, so a
#   Kubernetes probe gets `connection refused` — which reads as "broken", not
#   as "busy". Worse, a livenessProbe with a normal failureThreshold kills the
#   container mid-fetch, and the pod restarts forever without ever finishing
#   the bootstrap.
#
#   So we hold the port during that window and answer probes honestly:
#     /healthz          -> 200  the container is alive; do NOT restart it
#     everything else   -> 503  {"status":"bootstrapping", ...}  keep it out
#                               of the Service until it has data
#
#   This is the same contract the real server serves afterwards (liveness
#   independent of sync state, readiness gated on having a fresh tip), so the
#   handover is invisible to the probes. The body also carries the current
#   phase and the block count on disk, so `kubectl exec ... curl :8080/readyz`
#   shows fetch progress instead of a refused connection.
#
# The entrypoint stops this before exec'ing the real server, which then binds
# the same port.
set -uo pipefail

MODE="${1:?usage: bootstrap-health <serve|respond> ...}"

case "$MODE" in
serve)
  PORT="${2:?port}"
  export BOOTSTRAP_BLOCKS_DIR="${3:?blocks_dir}"
  export BOOTSTRAP_PHASE_FILE="${4:?phase_file}"

  # fork: one short-lived child per connection, so a probe is never queued
  # behind another and the listener survives a child that dies.
  #
  # EXEC, not SYSTEM: SYSTEM would route every probe through /bin/sh. The image
  # happens to have one (via bash), but nothing in it guarantees that, and the
  # shell buys us nothing here. EXEC execs us directly, and "$0" is the absolute
  # store path, so a connection is served with no /bin/sh and no PATH at all.
  exec socat "TCP-LISTEN:${PORT},reuseaddr,fork,backlog=16" EXEC:"$0 respond"
  ;;

respond)
  DIR="${BOOTSTRAP_BLOCKS_DIR:-/data/blocks}"
  PHASE_FILE="${BOOTSTRAP_PHASE_FILE:-}"

  # Request line only ("GET /readyz HTTP/1.1"); the headers are drained below.
  # -t guards against a probe that opens a connection and never writes.
  request_line=""
  IFS= read -r -t 5 request_line
  while IFS= read -r -t 5 header; do
    [ -z "${header%$'\r'}" ] && break
  done

  path="$(printf '%s' "$request_line" | cut -d' ' -f2)"
  phase="starting"
  [ -n "$PHASE_FILE" ] && [ -s "$PHASE_FILE" ] && phase="$(cat "$PHASE_FILE")"
  blocks=0
  [ -d "$DIR" ] && blocks="$(find "$DIR" -maxdepth 1 -name '*.json' -printf '.' | wc -c)"

  # Liveness is deliberately independent of progress: the container is up, and
  # restarting it would only throw away the blocks fetched so far.
  case "$path" in
  /healthz | /healthz\?*)
    status="200 OK"
    body="{\"status\":\"ok\",\"phase\":\"$phase\"}"
    ;;
  *)
    status="503 Service Unavailable"
    body="{\"status\":\"bootstrapping\",\"ready\":false,\"synced\":false,\"phase\":\"$phase\",\"blocks_on_disk\":$blocks}"
    ;;
  esac

  printf 'HTTP/1.1 %s\r\nContent-Type: application/json\r\nContent-Length: %s\r\nConnection: close\r\n\r\n%s' \
    "$status" "${#body}" "$body"
  ;;

*)
  echo "bootstrap-health: unknown mode '$MODE'" >&2
  exit 64
  ;;
esac
