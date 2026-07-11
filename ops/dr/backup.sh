#!/bin/sh
#
# Off-host backup of a Mina Indexer database.
#
# Takes a consistent snapshot (a single tar archive: a speedb checkpoint + the
# store version), optionally verifies it is restorable, pushes it to an off-host
# target, and prunes old local snapshots. Designed to run from cron or the
# systemd timer in this directory (`mina-indexer-backup.timer`).
#
# Snapshots are consistent whether the indexer is running or not:
#   - online  (default): taken via the running server over its Unix socket.
#   - offline (MINA_DB_DIR set): taken directly from a database dir (read-only).
#
# Configure via environment (all optional except a REMOTE target for off-host):
#   MINA_INDEXER_BIN   path to the mina-indexer binary        (default: mina-indexer on PATH)
#   MINA_SOCKET        server socket for an online snapshot   (default: ./mina-indexer.sock)
#   MINA_DB_DIR        database dir for an OFFLINE snapshot    (default: unset -> online)
#   BACKUP_DIR         local dir to stage snapshots           (default: ./backups)
#   REMOTE             off-host push target; see REMOTE_CMD   (default: unset -> local only, warns)
#   REMOTE_CMD         push command template; {src} {name} are substituted
#                      (default: rsync over ssh -> "$REMOTE")
#   RETENTION          number of local snapshots to keep      (default: 7)
#   VERIFY             1 = restore to a temp dir and run `database verify-integrity`
#                      before pushing (proves the backup is restorable)   (default: 0)
#   LABEL              network/instance label in the filename  (default: mina-indexer)
#
# Examples:
#   # online snapshot, push to a storagebox over rsync/ssh, keep 14:
#   REMOTE="u123@host.example:/backups/indexer" RETENTION=14 ops/dr/backup.sh
#
#   # offline snapshot of a stopped DB, push to S3, verify it restores:
#   MINA_DB_DIR=/data/db VERIFY=1 \
#     REMOTE_CMD='aws s3 cp {src} s3://my-bucket/mina-indexer/{name}' ops/dr/backup.sh

set -eu

RESULT='FAILED'
cleanup() {
	echo "backup: ${RESULT}" >&2
	if [ -n "${TMP_VERIFY:-}" ]; then
		rm -rf "$TMP_VERIFY" 2>/dev/null || true
	fi
}
trap cleanup EXIT

BIN="${MINA_INDEXER_BIN:-mina-indexer}"
SOCKET="${MINA_SOCKET:-./mina-indexer.sock}"
BACKUP_DIR="${BACKUP_DIR:-./backups}"
RETENTION="${RETENTION:-7}"
VERIFY="${VERIFY:-0}"
LABEL="${LABEL:-mina-indexer}"

# UTC timestamp; snapshot files are self-describing and sortable.
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
NAME="${LABEL}-${STAMP}.snapshot"
mkdir -p "$BACKUP_DIR"
SRC="${BACKUP_DIR}/${NAME}"

# 1) Take the snapshot (offline if MINA_DB_DIR is set, else online via socket).
if [ -n "${MINA_DB_DIR:-}" ]; then
	echo "backup: offline snapshot of ${MINA_DB_DIR} -> ${SRC}" >&2
	"$BIN" database snapshot --output-path "$SRC" --database-dir "$MINA_DB_DIR"
else
	echo "backup: online snapshot via ${SOCKET} -> ${SRC}" >&2
	"$BIN" --socket "$SOCKET" database snapshot --output-path "$SRC"
fi
[ -s "$SRC" ] || { echo "backup: snapshot file is empty/missing: $SRC" >&2; exit 1; }

# 2) Optionally prove the snapshot is restorable (restore to temp + verify).
if [ "$VERIFY" = "1" ]; then
	TMP_VERIFY="$(mktemp -d)"
	echo "backup: verifying snapshot restores cleanly..." >&2
	"$BIN" database restore --snapshot-file "$SRC" --restore-dir "$TMP_VERIFY/db"
	"$BIN" database verify-integrity --database-dir "$TMP_VERIFY/db"
	rm -rf "$TMP_VERIFY"
	TMP_VERIFY=""
	echo "backup: snapshot verified restorable" >&2
fi

# 3) Push off-host. Prefer an explicit REMOTE_CMD template (rclone / aws s3 /
#    gsutil / …); otherwise rsync over ssh into "$REMOTE". `{src}`/`{name}` in
#    the template are substituted with the snapshot's path and filename.
if [ -n "${REMOTE_CMD:-}" ]; then
	CMD="$REMOTE_CMD"
elif [ -n "${REMOTE:-}" ]; then
	CMD="rsync -a --partial {src} ${REMOTE}/{name}"
else
	CMD=""
fi

if [ -n "$CMD" ]; then
	CMD="$(printf '%s' "$CMD" | sed -e "s#{src}#${SRC}#g" -e "s#{name}#${NAME}#g")"
	echo "backup: pushing off-host: ${CMD}" >&2
	sh -c "$CMD"
else
	echo "backup: WARNING no REMOTE/REMOTE_CMD set — snapshot kept LOCAL ONLY at ${SRC}." >&2
	echo "backup: a backup on the same host as the primary is not disaster recovery." >&2
fi

# 4) Prune old local snapshots (keep the newest RETENTION).
#    ls -t is newest-first; delete everything past the retention count.
# shellcheck disable=SC2012
ls -1t "${BACKUP_DIR}/${LABEL}-"*.snapshot 2>/dev/null | tail -n "+$((RETENTION + 1))" | while IFS= read -r old; do
	echo "backup: pruning old snapshot ${old}" >&2
	rm -f "$old"
done

RESULT="OK (${NAME})"
