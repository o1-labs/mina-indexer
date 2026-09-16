# Configless per-network indexer images

Turnkey OCI images, one per network. Each `docker run`s with **zero flags or
mounts** and self-initializes: it knows its network, genesis, and block source, then
follows the chain tip. Built reproducibly with Nix (`flake.nix`), published to a single
GHCR repo with a `-<network>` tag suffix.

| Network    | Image tag                                   | Genesis ledger            | Block source                         |
|------------|---------------------------------------------|---------------------------|--------------------------------------|
| mainnet    | `ghcr.io/o1-labs/mina-indexer:<tag>-mainnet`  | embedded (V1, in binary)  | `gs://mina_network_block_data`       |
| devnet     | `ghcr.io/o1-labs/mina-indexer:<tag>-devnet`   | baked state-dump (`.gz`)  | `gs://mina_network_block_data`       |
| mesa-mut   | `ghcr.io/o1-labs/mina-indexer:<tag>-mesa-mut` | baked state-dump (`.gz`)  | `gs://mesa-mut-precomputed-blocks`   |

> **devnet** is a hardfork network; the image roots at a recent published state-dump
> checkpoint (`devnet-527922-3NK4DL35`, embedded emptied) and follows the tip — it does
> not index history before that checkpoint. Re-bake (newer state dump + block) to advance
> the start point.

## Run

```bash
docker run -d --name indexer -p 8080:8080 \
  -v indexer-mainnet:/data \
  ghcr.io/o1-labs/mina-indexer:<tag>-mainnet
```

Nothing else is required. A volume on `/data` is optional but recommended so the DB and
fetched blocks survive restarts. The first boot of the devnet/mesa images decompresses the
baked genesis ledger to `/data` (mesa is ~900 MB; takes a few seconds).

**The first boot is long.** After the ledger, devnet/mesa bulk-fetch their block backlog
(devnet: ~32,000 heights) and then ingest it, so expect tens of minutes before `/readyz`
returns 200. Port 8080 answers throughout — `/healthz` 200, everything else 503 with the
current phase and block count — so you can watch progress:

```bash
curl -s localhost:8080/readyz
# {"status":"bootstrapping","ready":false,"phase":"fetching-blocks","blocks_on_disk":18342}
```

On Kubernetes this needs a `startupProbe`, or the liveness probe kills the container
mid-fetch and it never finishes. See
[*Kubernetes probes*](../docs/operating.md#kubernetes-probes).

## What's inside

- `mina-indexer` — the (network-agnostic) indexer binary.
- A network entrypoint (`ops/entrypoints/<network>.sh`) that runs the indexer configless.
- A block fetcher wired to `--fetch-new-blocks-exe` / `--missing-block-recovery-exe`:
  - `block-pull` (`ops/block-pull.sh`) for the public networks — pulls from the
    `mina_network_block_data` bucket (objects already named `<network>-<height>-<hash>.json`).
  - `mesa-pull` (`ops/mesa-mut/mesa-pull.sh`) for mesa — different bucket + prefix rewrite.
- `bootstrap-health` (`ops/bootstrap-health.sh`) — holds port 8080 during the first-boot
  window, before `server start` binds it, so probes see
  `503 {"status":"bootstrapping","phase":…,"blocks_on_disk":…}` (and `/healthz` 200)
  instead of `connection refused`. Stopped at handover; `MINA_BOOTSTRAP_HEALTH_PORT=0`
  disables it.
- `verify-block` — the trustless verify shim, baked but **dormant** (the images do not pass
  `--verify-block-exe`; trustless verification remains a separate opt-in + sidecar concern).
- The mesa/devnet genesis ledgers ship gzipped at `/genesis/<network>.json.gz`.

## Build locally

```bash
nix build .#dockerImage-mainnet      # or -devnet / -mesa
./result | docker load               # streamLayeredImage streams a docker-archive
```

CI (`.github/workflows/oci-image.yml`) builds the `[mainnet, devnet, mesa-mut]` matrix on every PR
(build-only) and, on a `v*` tag, pushes `:<tag>-<network>` and `:latest-<network>`.

## Status

- **mainnet**, **devnet**, and **mesa-mut**: built and runnable configless.
- **devnet** roots at the published state-dump checkpoint `devnet-527922-3NK4DL35` (genesis
  ledger from `gs://o1labs-gitops-infrastructure/devnet/`); it indexes from that checkpoint
  forward, not full history. `DEVNET_CHAIN_ID` is a placeholder (like mesa's) — the real
  chain id only affects the REST chain-id endpoint, not indexing.
