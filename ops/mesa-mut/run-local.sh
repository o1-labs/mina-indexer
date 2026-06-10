#!/usr/bin/env bash
#
# Run a local mina-indexer instance for the mesa-mut network.
#
#   ops/mesa-mut/run-local.sh setup     # download the genesis ledger (state dump)
#   ops/mesa-mut/run-local.sh fetch [N]  # download N blocks from genesis (default 200)
#   ops/mesa-mut/run-local.sh create     # build the indexer database from the blocks
#   ops/mesa-mut/run-local.sh start      # start the web server (background)
#   ops/mesa-mut/run-local.sh status     # print the chain summary
#   ops/mesa-mut/run-local.sh stop       # stop the web server
#
# Config via env (with defaults):
#   INSTANCE   instance dir            (default ~/mesa-indexer)
#   BIN        path to the mina-indexer binary
#   WEB_PORT   web server port         (default 8080)
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"

INSTANCE="${INSTANCE:-$HOME/mesa-indexer}"
WEB_PORT="${WEB_PORT:-8080}"
BIN="${BIN:-$REPO/rust/target/release/mina-indexer}"

GENESIS_HASH="3NLp6dKNhYtsqUj49QYV5GtDaeocSJBAa2y2ER2QQLqLukE3wuZT"
STATE_DUMP_URL="https://storage.googleapis.com/o1labs-gitops-infrastructure/mina-mesa-mut-1/mina-mesa-mut-1-state-dump-3NLp6dKNhYtsqUj49QYV5GtDaeocSJBAa2y2ER2QQLqLukE3wuZT-df71a5f2dd5abdae8e2e5d4d0047b383bdfca4d75ec1d2260b8ad621f1a18ffe.json.gz"

LEDGER="$INSTANCE/mesa-genesis-ledger.json"
BLOCKS="$INSTANCE/blocks"
DB="$INSTANCE/db"
SOCK="$INSTANCE/mina-indexer.sock"
LOG="$INSTANCE/server.log"
PIDFILE="$INSTANCE/server.pid"

mkdir -p "$INSTANCE" "$BLOCKS" "$DB"
[ -x "$BIN" ] || { echo "binary not found at $BIN (build it first, or set BIN=)"; exit 1; }
export GIT_COMMIT_HASH="${GIT_COMMIT_HASH:-$(git -C "$REPO" rev-parse --short=8 HEAD 2>/dev/null || echo local)}"

case "${1:-}" in
  setup)
    if [ -s "$LEDGER" ]; then
      echo "genesis ledger already present: $LEDGER"
    else
      echo "downloading + decompressing the mesa-mut genesis ledger (~79MB gz -> ~900MB)..."
      curl -fsS "$STATE_DUMP_URL" | gunzip > "$LEDGER"
      echo "wrote $LEDGER ($(du -h "$LEDGER" | cut -f1))"
    fi
    ;;

  fetch)
    "$HERE/fetch-blocks.sh" "$BLOCKS" 297735 $((297735 + ${2:-200} - 1))
    echo "blocks now: $(ls "$BLOCKS"/*.json 2>/dev/null | wc -l)"
    ;;

  create)
    [ -s "$LEDGER" ] || { echo "run 'setup' first (no genesis ledger)"; exit 1; }
    ulimit -n 8192 || true
    "$BIN" --socket "$SOCK" database create --network mesa \
      --genesis-hash "$GENESIS_HASH" --genesis-ledger "$LEDGER" \
      --blocks-dir "$BLOCKS" --database-dir "$DB" --log-level info
    ;;

  start)
    if [ -f "$PIDFILE" ] && kill -0 "$(cat "$PIDFILE")" 2>/dev/null; then
      echo "already running (pid $(cat "$PIDFILE"))"; exit 0
    fi
    ulimit -n 8192 || true
    # --blocks-dir makes the indexer watch the directory and auto-ingest new
    # blocks as `fetch` (or your own sync) drops them in. setsid fully detaches
    # so the server survives the shell/CI step that launched it.
    setsid "$BIN" --socket "$SOCK" server start --network mesa \
      --genesis-hash "$GENESIS_HASH" --genesis-ledger "$LEDGER" \
      --blocks-dir "$BLOCKS" --database-dir "$DB" \
      --web-hostname 127.0.0.1 --web-port "$WEB_PORT" \
      --log-level info > "$LOG" 2>&1 < /dev/null &
    echo $! > "$PIDFILE"
    sleep 3
    echo "started (pid $(cat "$PIDFILE")) — GraphQL at http://127.0.0.1:$WEB_PORT/graphql"
    ;;

  status)
    curl -fsS "http://127.0.0.1:$WEB_PORT/summary" || echo "not responding on :$WEB_PORT"
    echo
    ;;

  stop)
    if [ -f "$PIDFILE" ]; then
      kill "$(cat "$PIDFILE")" 2>/dev/null || true
      rm -f "$PIDFILE"
      echo "stopped"
    else
      echo "no pidfile; nothing to stop"
    fi
    ;;

  *)
    sed -n '2,20p' "$0"
    exit 1
    ;;
esac
