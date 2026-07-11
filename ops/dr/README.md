# Disaster-recovery tooling

Off-host backup kit for the Mina Indexer. The full runbook (RTO/RPO, restore
procedures, failover playbook, drill) is [`docs/disaster-recovery.md`](../../docs/disaster-recovery.md).

```
ops/dr/
├── backup.sh                     # take a snapshot, (optionally verify), push off-host, prune
├── mina-indexer-backup.service   # systemd oneshot that runs backup.sh
└── mina-indexer-backup.timer     # schedule (daily by default)
```

## Quick start

```bash
# one-off, online snapshot (indexer running) pushed over rsync/ssh, keep 14, verified:
REMOTE=u123@backups.example:/backups/indexer RETENTION=14 VERIFY=1 ops/dr/backup.sh

# offline snapshot of a stopped DB, pushed to S3:
MINA_DB_DIR=/data/db \
  REMOTE_CMD='aws s3 cp {src} s3://my-bucket/mina-indexer/{name}' ops/dr/backup.sh
```

## Scheduled (systemd)

```bash
sudo install -m755 ops/dr/backup.sh /usr/local/bin/mina-indexer-backup.sh
sudo install -m644 ops/dr/mina-indexer-backup.{service,timer} /etc/systemd/system/
printf 'REMOTE=%s\nRETENTION=14\nVERIFY=1\nLABEL=mina-indexer-mainnet\n' \
  'u123@backups.example:/backups/indexer' | sudo tee /etc/mina-indexer/backup.env
sudo systemctl enable --now mina-indexer-backup.timer
systemctl list-timers mina-indexer-backup.timer   # confirm next run
```

## Configuration (environment)

| Var | Default | Meaning |
|---|---|---|
| `MINA_INDEXER_BIN` | `mina-indexer` (PATH) | Binary to invoke. |
| `MINA_SOCKET` | `./mina-indexer.sock` | Server socket for an **online** snapshot. |
| `MINA_DB_DIR` | *(unset)* | Set for an **offline** snapshot straight from a DB dir. |
| `BACKUP_DIR` | `./backups` | Local staging dir for snapshots. |
| `REMOTE` | *(unset)* | Off-host target for the default rsync/ssh push. |
| `REMOTE_CMD` | rsync over ssh → `$REMOTE` | Push command template; `{src}`/`{name}` substituted (rclone, `aws s3`, `gsutil`, …). |
| `RETENTION` | `7` | Local snapshots to keep. |
| `VERIFY` | `0` | `1` = restore to a temp dir + `verify-integrity` before pushing (proves restorability). |
| `LABEL` | `mina-indexer` | Filename prefix (use per-network, e.g. `mina-indexer-mainnet`). |

> A snapshot on the **same host** as the primary is not disaster recovery — set
> `REMOTE`/`REMOTE_CMD` so it lands off-host. `backup.sh` warns loudly if neither is set.

## Notes

- **Online vs offline.** Online snapshots go through the running server's socket and
  don't interrupt it. Offline snapshots read a stopped (or read-only) DB dir directly.
  Both produce the same self-contained tar archive.
- **This is the disaster layer.** For a *tight* RPO, also enable on-host periodic
  checkpoints (`MINA_CHECKPOINT_DIR`) — see the runbook.
- **Restore:** `mina-indexer database restore --snapshot-file <f> --restore-dir <dir>`,
  then `database verify-integrity` before serving.
