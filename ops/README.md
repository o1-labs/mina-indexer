# Configless per-network indexer images

Turnkey OCI images, one per network. Each `docker run`s with **zero flags or
mounts** and self-initializes: it knows its network, genesis, and block source, then
follows the chain tip. Built reproducibly with Nix (`flake.nix`), published to a single
GHCR repo with a `-<network>` tag suffix.

| Network    | Image tag                                   | Genesis ledger            | Block source                         |
|------------|---------------------------------------------|---------------------------|--------------------------------------|
| mainnet    | `ghcr.io/o1-labs/mina-indexer:<tag>-mainnet`  | embedded (V1, in binary)  | `gs://mina_network_block_data`       |
| mesa-mut   | `ghcr.io/o1-labs/mina-indexer:<tag>-mesa-mut` | baked state-dump (`.gz`)  | `gs://mesa-mut-precomputed-blocks`   |

> **devnet** is a hardfork bring-up (its own genesis ledger + emptied genesis block +
> `new_devnet` wiring); tracked as a separate issue.

## Run

```bash
docker run -d --name indexer -p 8080:8080 \
  -v indexer-mainnet:/data \
  ghcr.io/o1-labs/mina-indexer:<tag>-mainnet
```

Nothing else is required. A volume on `/data` is optional but recommended so the DB and
fetched blocks survive restarts. The first boot of the devnet/mesa images decompresses the
baked genesis ledger to `/data` (mesa is ~900 MB; takes a few seconds).

## What's inside

- `mina-indexer` — the (network-agnostic) indexer binary.
- A network entrypoint (`ops/entrypoints/<network>.sh`) that runs the indexer configless.
- A block fetcher wired to `--fetch-new-blocks-exe` / `--missing-block-recovery-exe`:
  - `block-pull` (`ops/block-pull.sh`) for the public networks — pulls from the
    `mina_network_block_data` bucket (objects already named `<network>-<height>-<hash>.json`).
  - `mesa-pull` (`ops/mesa-mut/mesa-pull.sh`) for mesa — different bucket + prefix rewrite.
- `verify-block` — the trustless verify shim, baked but **dormant** (the images do not pass
  `--verify-block-exe`; trustless verification remains a separate opt-in + sidecar concern).
- The mesa genesis ledger ships gzipped at `/genesis/mesa.json.gz`.

## Build locally

```bash
nix build .#dockerImage-mainnet      # or -mesa
./result | docker load               # streamLayeredImage streams a docker-archive
```

CI (`.github/workflows/oci-image.yml`) builds the `[mainnet, mesa-mut]` matrix on every PR
(build-only) and, on a `v*` tag, pushes `:<tag>-<network>` and `:latest-<network>`.

## Status

- **mainnet** and **mesa-mut**: complete and verified end-to-end.
- **devnet**: tracked separately — it's a hardfork bring-up needing its own genesis ledger
  (state dumps exist at `gs://o1labs-gitops-infrastructure/devnet/`), an emptied genesis
  block, and mesa-style `new_devnet` wiring (`ChainId`/`GenesisVersion`/dispatch +
  `state/mod.rs` staking arm). `block-pull` already handles the `devnet-*` objects.
