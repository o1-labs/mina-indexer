# Releasing

This document gives the release process for the mina-indexer. Each release goes to
devnet as a release candidate first. The final tag is put on the same commit only after
the devnet check passes.

## Terms

- **Release candidate (rc):** a tag `vX.Y.Z-rc.N` on the release commit. It builds the
  images and the Debian package, but it does not move `latest-<network>` and it publishes
  the package to the `unstable` component.
- **Final release:** the tag `vX.Y.Z` on the same commit as the last rc. The Nix image
  build is reproducible, so the final images have the same digests as the rc images.
- **Store version:** `IndexerStoreVersion::{MAJOR,MINOR,PATCH}` in
  `rust/src/store/version/mod.rs`. A database or a snapshot with another store version
  does not start. A change of the store version is a **store change**.

## What the tags build

| Tag | OCI images (`ghcr.io/o1-labs/mina-indexer`) | Debian package (`packages.o1test.net`, `bookworm`) |
|---|---|---|
| `vX.Y.Z-rc.N` | `:vX.Y.Z-rc.N-<network>` | `X.Y.Z~rc.N` in `unstable` |
| `vX.Y.Z` | `:vX.Y.Z-<network>` and `:latest-<network>` | `X.Y.Z` in `stable` |

`<network>` is `mainnet` or `devnet`. The workflows are `.github/workflows/oci-image.yml`
and `.github/workflows/debian.yml`. Both stop with an error if the tag does not agree with
the `version` in `rust/Cargo.toml`. The binary reports that version.

## Procedure

### 1. Prepare the release commit

1. Merge all the changes for the release into `main`.
2. Open a PR `chore(release): X.Y.Z` that changes `version` in `rust/Cargo.toml` (and
   `rust/Cargo.lock`). Merge it.
3. If the release has a store change, write down which data changes and why a rebuild
   is necessary. The release page needs this text (step 6).

### 2. Tag the release candidate

```sh
git tag -a vX.Y.Z-rc.1 <release commit> -m "vX.Y.Z-rc.1"
git push origin vX.Y.Z-rc.1
```

Wait until the "OCI image" and "Debian package" workflows pass. Write down the image
digests:

```sh
skopeo inspect docker://ghcr.io/o1-labs/mina-indexer:vX.Y.Z-rc.1-devnet | jq -r .Digest
```

### 3. Store change only: publish a new devnet snapshot

The image restores `devnet-latest.tar` on first boot. `database restore` rejects a snapshot
with another store version and then builds the database from blocks. The public block
bucket keeps only recent devnet blocks, so a build from blocks cannot complete, and the
pod does not become ready. Thus, before the deploy:

1. Build a database with the rc binary from the fork root. The old blocks are in the
   private block archive `gs://mina-network-block-data-archive`.
2. Make a snapshot (`mina-indexer database snapshot`) and check its manifest: store
   version, genesis hash, best block.
3. Upload it as `devnet/devnet-<height>-<hash>-store-<version>.{tar,manifest.json}`.
4. Copy it to `devnet/devnet-latest.{tar,manifest.json}` immediately before the deploy
   (step 4). After the copy, an older release on devnet cannot do a cold start.

Do the same for mainnet before the mainnet deploy (step 7).

### 4. Deploy the rc to devnet

1. In `o1-labs/gitops-infrastructure`, set `MINA_INDEXER_TAG` and `MINA_INDEXER_DIGEST`
   together, in the ApplicationSet element of `mina-indexer-devnet`. Merge, then sync in
   Argo CD.
2. Make sure that the StatefulSet uses the new image **before** you delete the volume.
3. Store change only: delete the PVC `data-mina-indexer-0` and the pod. The first boot
   restores the snapshot and then catches up.

### 5. Check the rc on devnet

The rc passes only if all of these are true:

- The pod is ready and has 0 restarts. The log has no `ERROR` lines.
- `/healthz` and `/readyz` return 200. The tip is the same block as the daemon's tip.
- The version in GraphQL is `X.Y.Z-<commit>`.
- The parity check against the archive-node-api passes
  ([o1-labs/devnet-integration-tests](https://github.com/o1-labs/devnet-integration-tests),
  `mina-indexer/parity/parity.py`, all targets, with `--compare-txhash`).
- The ledger fidelity check is exact (`rake fidelity`, at least 100 blocks below the
  tip).

If a check fails, fix it on `main`, change nothing else, and go back to step 2 with
`rc.N+1`.

### 6. Tag the final release and publish the release page

```sh
git tag -a vX.Y.Z <release commit> -m "vX.Y.Z"
git push origin vX.Y.Z
```

Make sure the final digests are the same as the rc digests. Then publish a GitHub
Release for `vX.Y.Z` with these sections:

- **Store version:** the value, whether it changed, why, and that a rebuild is necessary.
  Give the published snapshot for each network.
- **Fixes:** one line for each PR.
- **Known limits:** what is still wrong, with issue links.
- **Artifacts:** the image references with digests, and the Debian package.
- **Devnet check:** the results of step 5.
- **Deploy:** the steps for an operator, including the volume wipe for a store change.

### 7. Mainnet

Deploy `vX.Y.Z` to mainnet with the same steps as step 4. For a store change, publish the
mainnet snapshot first (step 3).

## Rules

- Do not skip the devnet check.
- Never move a tag. A new rc number is cheap.
- Pin the image digest together with the tag, and change both together.
