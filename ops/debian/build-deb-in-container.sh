#!/usr/bin/env bash
#
# Build a portable mina-indexer .deb inside a clean distro container, so the
# binary links that distro's glibc (a Nix-built binary is NOT portable). This is
# the local equivalent of .github/workflows/debian.yml.
#
# Usage:
#   ops/debian/build-deb-in-container.sh <codename> <image>
#   e.g. ops/debian/build-deb-in-container.sh noble ubuntu:24.04
#
# Output: dist/<codename>/*.deb  (under the repo root; override with OUT=)
set -euo pipefail

CODENAME="${1:?usage: build-deb-in-container.sh <codename> <image>}"
IMAGE="${2:?usage: build-deb-in-container.sh <codename> <image>}"

REPO="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="${OUT:-$REPO/dist/$CODENAME}"
mkdir -p "$OUT"
VER="${DEB_VERSION:-$(grep -m1 '^version' "$REPO/rust/Cargo.toml" | awk -F'"' '{print $2}')~${CODENAME}}"

echo ">> Building mina-indexer .deb for $CODENAME ($IMAGE), version $VER" >&2

# Share a cargo cache across codenames to avoid re-downloading crates.
docker volume create mina-indexer-cargo-cache >/dev/null

docker volume create "mina-indexer-target-$CODENAME" >/dev/null

docker run --rm \
  -v "$REPO":/work \
  -v "$OUT":/out \
  -v mina-indexer-cargo-cache:/cargo \
  -v "mina-indexer-target-$CODENAME":/target \
  -e CODENAME="$CODENAME" -e VER="$VER" \
  "$IMAGE" bash -euo pipefail -c '
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq
    apt-get install -y -qq --no-install-recommends \
      ca-certificates curl git build-essential pkg-config libclang-dev >/dev/null
    git config --global --add safe.directory /work || true

    # Rust toolchain (pinned by rust-toolchain.toml), cached in the shared volume.
    export RUSTUP_HOME=/cargo/rustup CARGO_HOME=/cargo/home
    curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path --default-toolchain none --profile minimal
    . "$CARGO_HOME/env"
    ( cd /work/rust && rustup show >/dev/null )

    # Build with the system gcc toolchain (override the clang+mold linker), into
    # a container-local target dir so the host target/ (Nix artifacts) is untouched.
    export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=cc RUSTFLAGS=""
    export CARGO_TARGET_DIR=/target
    export LIBCLANG_PATH="$(dirname "$(find /usr/lib /usr/lib64 -name "libclang.so*" 2>/dev/null | head -1)")"
    export GIT_COMMIT_HASH="$(git -C /work rev-parse --short=8 HEAD 2>/dev/null || echo local)"

    cd /work/rust
    cargo build --release --bin mina-indexer

    # cargo-deb needs a newer rustc than the project pins (1.85.1); build it with
    # stable. Cached in the shared cargo volume, so only built once.
    if ! command -v cargo-deb >/dev/null; then
      rustup toolchain install stable --profile minimal
      cargo +stable install cargo-deb --locked
    fi
    cargo deb --no-build --fast --deb-version "$VER"
    cp /target/debian/*.deb /out/
    chmod 0644 /out/*.deb
  '

echo ">> Done:" >&2
ls -la "$OUT"/*.deb
