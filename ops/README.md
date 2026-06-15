# Configless per-network indexer images

Three turnkey OCI images, one per network. Each `docker run`s with **zero flags or
mounts** and self-initializes: it knows its network, genesis, and block source, then
follows the chain tip. Built reproducibly with Nix (`flake.nix`), published to a single
GHCR repo with a `-<network>` tag suffix.

| Network    | Image tag                                   | Genesis ledger            | Block source                         |
|------------|---------------------------------------------|---------------------------|--------------------------------------|
| mainnet    | `ghcr.io/o1-labs/mina-indexer:<tag>-mainnet`  | embedded (V1, in binary)  | `gs://mina_network_block_data`       |
| devnet     | `ghcr.io/o1-labs/mina-indexer:<tag>-devnet`   | baked state-dump (`.gz`)¹ | `gs://mina_network_block_data`       |
| mesa-mut   | `ghcr.io/o1-labs/mina-indexer:<tag>-mesa-mut` | baked state-dump (`.gz`)  | `gs://mesa-mut-precomputed-blocks`   |

¹ devnet is scaffolded — see "Status" below.

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
  - `block-pull` (`ops/block-pull.sh`) for mainnet/devnet — pulls from the public
    `mina_network_block_data` bucket (objects already named `<network>-<height>-<hash>.json`).
  - `mesa-pull` (`ops/mesa-mut/mesa-pull.sh`) for mesa — different bucket + prefix rewrite.
- `verify-block` — the trustless verify shim, baked but **dormant** (the images do not pass
  `--verify-block-exe`; trustless verification remains a separate opt-in + sidecar concern).
- The mesa/devnet genesis ledgers ship gzipped at `/genesis/<network>.json.gz`.

## Build locally

```bash
nix build .#dockerImage-mainnet      # or -devnet / -mesa
./result | docker load               # streamLayeredImage streams a docker-archive
```

CI (`.github/workflows/oci-image.yml`) builds the `[mainnet, devnet, mesa-mut]` matrix on
every PR (build-only) and, on a `v*` tag, pushes `:<tag>-<network>` and `:latest-<network>`.

## Status

- **mainnet** and **mesa-mut**: complete and verified end-to-end.
- **devnet**: image plumbing is in place, but two inputs are still required:
  1. the devnet genesis-ledger **state-dump URL** (`flake.nix` `devnetGenesisGz`), and
  2. a devnet **genesis-hash dispatch arm** in `rust/src/bin/mina-indexer.rs` (with
     `DEVNET_*` constants + `ChainId::devnet()` / `GenesisVersion::devnet()`), so the binary
     selects V2 parsing for devnet's genesis hash instead of falling through to mainnet/V1.
