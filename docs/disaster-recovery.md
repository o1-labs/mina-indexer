# Disaster recovery runbook

How to survive — and recover from — losing a Mina Indexer's database: what to back
up, how, how fast you can come back, and the exact restore/failover steps. For
day-to-day operation see [`operating.md`](operating.md); the backup tooling lives in
[`ops/dr/`](../ops/dr/).

## What can go wrong, and the fastest way back

| Failure | Fastest recovery | Time |
|---|---|---|
| Process crash / OOM | Restart. Clean shutdown drained the WAL; otherwise WAL replay. | seconds–minutes |
| Corrupt-but-present DB | Restore from the newest **on-host checkpoint** (`--restore-from-checkpoint`), or from an off-host snapshot. | minutes |
| Host/disk lost | Provision a new host, **restore the latest off-host snapshot**, start in Sync mode; it catches up from the block source. | minutes + catch-up |
| Silent corruption (wrong answers) | Detect early with [`database verify-integrity`](operating.md#checking-a-database-for-corruption); then restore as above. | detection is the hard part |

The indexer is **not the source of truth** — the chain is. Any database can be
rebuilt from precomputed blocks (`database create`), but a full re-index of mainnet
takes hours. Backups exist to turn "hours of re-index" into "minutes of restore."

## RTO / RPO

- **RPO (how much data you can lose).** Two independent layers:
  - **On-host checkpoints** — with `MINA_CHECKPOINT_DIR` set, a consistent speedb
    checkpoint is written to `<dir>/latest` every `MINA_CHECKPOINT_INTERVAL_SECS`
    (default **1h**). This is the tight-RPO layer, but it lives on the **same disk**
    as the primary — it does **not** survive host/disk loss.
  - **Off-host snapshots** — [`ops/dr/backup.sh`](../ops/dr/backup.sh) pushes a
    self-contained snapshot to another host/bucket on a schedule (the timer ships
    at **daily**). This is the survives-anything layer; its cadence is your
    disaster RPO. Tighten `OnCalendar` for a smaller RPO.
  - **Effective RPO for a total-host loss = the off-host snapshot interval.** Even
    so, any gap re-ingests from the block source on restart, so the *serving* gap is
    only the catch-up time, not lost data — provided the block source still has those
    blocks.
- **RTO (how long to be back).** Restore is a tar extract + open:
  - checkpoint restore (same host): **seconds–minutes** (hard-link-cheap).
  - off-host snapshot restore: **minutes** (download + extract + open), then Sync-mode
    catch-up from the block source proportional to the gap.

Pin real numbers per deployment by running the **restore drill** below and timing it.

## Backups — the two layers

### 1. On-host checkpoints (tight RPO, not DR)
Already built in. Enable and let it run:
```bash
MINA_CHECKPOINT_DIR=/data/checkpoints MINA_CHECKPOINT_INTERVAL_SECS=3600 \
  mina-indexer server start --database-dir /data/db …
```
Recovery is automatic-ish: `server start --restore-from-checkpoint /data/checkpoints`
seeds an empty/absent `--database-dir` from `<dir>/latest` (see
[operating.md → Checkpoints & recovery](operating.md#checkpoints--recovery)). The
three configless images bake this in.

> Checkpoints share the primary's SSD. They protect against a corrupt/wiped DB, **not**
> against losing the disk. That's what off-host snapshots are for.

### 2. Off-host snapshots (disaster RPO)
[`ops/dr/backup.sh`](../ops/dr/backup.sh) takes a consistent snapshot (online via the
server socket, or offline from a stopped DB), optionally **proves it restores**
(`VERIFY=1` restores to a temp dir and runs `verify-integrity`), pushes it off-host,
and prunes old local copies. Cloud-agnostic — the push is a command template
(`REMOTE_CMD`), so rsync/ssh, rclone, `aws s3`, `gsutil`, etc. all work.

Run it from the systemd timer:
```bash
sudo install -m755 ops/dr/backup.sh /usr/local/bin/mina-indexer-backup.sh
sudo install -m644 ops/dr/mina-indexer-backup.service ops/dr/mina-indexer-backup.timer \
  /etc/systemd/system/
sudoedit /etc/mina-indexer/backup.env       # set REMOTE / RETENTION / VERIFY …
sudo systemctl enable --now mina-indexer-backup.timer
```
`/etc/mina-indexer/backup.env`, minimal:
```sh
REMOTE=u123@backups.example.com:/backups/mina-indexer
RETENTION=14
VERIFY=1
LABEL=mina-indexer-mainnet
```

## Restore procedures

### A. Restore from an off-host snapshot (host/disk lost)
```bash
# On the new host, with the snapshot pulled to ./snapshot:
mina-indexer database restore --snapshot-file ./snapshot --restore-dir /data/db
mina-indexer database verify-integrity --database-dir /data/db      # confirm before serving
mina-indexer server start --database-dir /data/db --blocks-dir /data/blocks …  # Sync mode; catches up
```

### B. Restore from an on-host checkpoint (corrupt DB, disk intact)
```bash
# Empty/absent DB dir self-heals from the checkpoint:
mina-indexer server start --restore-from-checkpoint /data/checkpoints --database-dir /data/db …
# Corrupt-but-present DB: clear it (or pass --restore-force) so the checkpoint wins:
rm -rf /data/db   # or: server start … --restore-from-checkpoint … --restore-force
```

### C. Last resort — rebuild from blocks
No usable backup? Rebuild from the block source (hours for mainnet):
```bash
mina-indexer database create --database-dir /data/db --blocks-dir /data/blocks
```

## Failover playbook (host loss)

1. **Provision** a replacement host meeting the [sizing tiers](operating.md#host-requirements)
   (SSD, `ulimit -n ≥ 4096`).
2. **Pull** the latest off-host snapshot for the network.
3. **Restore** it (procedure A) and **`verify-integrity`** before exposing traffic.
4. **Start** the indexer in Sync mode pointed at the block source; watch
   `mina_indexer_synced` / `tip_age_seconds` recover
   ([Grafana dashboard](../ops/observability/README.md)).
5. **Cut over** traffic (DNS / load balancer) once `synced=1`.
6. For **near-zero serving gap**, keep a warm **read replica** seeded from snapshots
   (`store::read_only`, see [operating.md](operating.md)) and promote it instead of
   cold-restoring — replica-promotion automation is future work.

## Verify your DR actually works (do this quarterly)

A backup you have never restored is a hope, not a backup. Drill it:
```bash
# 1. Take (or fetch) a snapshot.  2. Restore to a scratch dir.  3. Verify.  4. Time it.
time mina-indexer database restore --snapshot-file ./snapshot --restore-dir /tmp/dr-drill
mina-indexer database verify-integrity --database-dir /tmp/dr-drill
```
`VERIFY=1` in `backup.sh` folds steps 2–3 into every backup, so a broken backup is
caught when it's made — not when you need it. Record the measured restore time as your
RTO.
