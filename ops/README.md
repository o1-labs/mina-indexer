# Configless per-network indexer images

Turnkey OCI images, one per network. Each `docker run`s with **zero flags or
mounts** and self-initializes: it knows its network, genesis, and block source, then
follows the chain tip. Built reproducibly with Nix (`flake.nix`), published to a single
GHCR repo with a `-<network>` tag suffix.

| Network    | Image tag                                   | Genesis ledger            | Block source                         |
|------------|---------------------------------------------|---------------------------|--------------------------------------|
| mainnet    | `ghcr.io/o1-labs/mina-indexer:<tag>-mainnet`  | baked state-dump (`.gz`)  | `gs://mina_network_block_data`       |
| devnet     | `ghcr.io/o1-labs/mina-indexer:<tag>-devnet`   | baked state-dump (`.gz`)  | `gs://mina_network_block_data`       |

> **devnet** is a hardfork network; the image roots at the genesis of its 2026-08-19
> mesa-protocol fork (`devnet-545434-3NLT7n4L`, embedded with no transactions) and follows
> the tip — it does not index the chain before that fork. The next devnet fork needs a
> re-bake: its genesis block (the daemon's `genesisBlock`) and its fork state dump.
>
> **mainnet** is rooted the same way, at the genesis of its 2026-09-03 mesa-protocol fork
> (`mainnet-548147-3NLntCRR`). It does not index the V1 or Berkeley chain before that fork.

## Run

```bash
docker run -d --name indexer -p 8080:8080 \
  -v indexer-mainnet:/data \
  ghcr.io/o1-labs/mina-indexer:<tag>-mainnet
```

Nothing else is required. A volume on `/data` is optional but recommended so the DB and
fetched blocks survive restarts. The first boot decompresses the baked genesis ledger to
`/data` (takes a few seconds).

**The first boot is long.** After the ledger, the indexer fetches its block backlog
in-process and ingests it, so expect tens of minutes before `/readyz` returns 200. Port
8080 answers throughout — `/healthz` 200, everything else 503 — so you can watch progress:

```bash
curl -s localhost:8080/readyz
# {"status":"catching_up","ready":false,"tip_height":551204,"tip_age_seconds":41207,...}
```

On Kubernetes this needs a `startupProbe`, or the liveness probe kills the container
mid-bootstrap and it never finishes. See
[*Kubernetes probes*](../docs/operating.md#kubernetes-probes).

## What's inside

- `mina-indexer` — the (network-agnostic) indexer binary.
- A network entrypoint (`ops/entrypoints/<network>.sh`) that runs the indexer configless.
- No external fetcher: the entrypoint passes `--block-source gs://mina_network_block_data`
  (override with `MINA_BLOCK_SOURCE`), and the indexer fetches blocks in-process. See
  [Block fetching](../docs/operating.md#block-fetching).
- `bootstrap-health` (`ops/bootstrap-health.sh`) — holds port 8080 during the first-boot
  window, before `server start` binds it, so probes see
  `503 {"status":"bootstrapping","phase":…,"blocks_on_disk":…}` (and `/healthz` 200)
  instead of `connection refused`. Stopped at handover; `MINA_BOOTSTRAP_HEALTH_PORT=0`
  disables it.
- `verify-block` — the trustless verify shim, baked but **dormant** (the images do not pass
  `--verify-block-exe`; trustless verification remains a separate opt-in + sidecar concern).
- The devnet genesis ledger ships gzipped at `/genesis/devnet.json.gz`.

## Build locally

```bash
nix build .#dockerImage-mainnet      # or -devnet
./result | docker load               # streamLayeredImage streams a docker-archive
```

CI (`.github/workflows/oci-image.yml`) builds the `[mainnet, devnet]` matrix on every PR
(build-only) and, on a `v*` tag, pushes `:<tag>-<network>` and `:latest-<network>`.

## Status

- **mainnet** and **devnet**: built and runnable configless.
- **devnet** roots at its fork genesis `devnet-545434-3NLT7n4L` (genesis ledger: the fork's
  state dump `devnet-state-dump-3NLYmfj4…` from `gs://o1labs-gitops-infrastructure/devnet/`);
  it indexes from that fork forward, not full history. `DEVNET_CHAIN_ID` is a placeholder — the real
  chain id only affects the REST chain-id endpoint, not indexing.
