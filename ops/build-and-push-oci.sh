#!/usr/bin/env bash
#
# Build the mina-indexer OCI image with Nix and push it to a container
# registry. Intended for incorporating the indexer into the o1labs stack.
#
# The image itself is defined in flake.nix (`dockerImage`, built reproducibly
# with pkgs.dockerTools — no Dockerfile/daemon needed to BUILD). This wrapper
# adds tagging + push to an external registry.
#
# Usage:
#   REGISTRY=europe-west3-docker.pkg.dev/o1labs-192920/euro-docker-repo \
#   IMAGE=mina-indexer \
#   [TAG=<git-short-sha>] \
#   ops/build-and-push-oci.sh
#
# Auth (run once in CI before this script):
#   gcloud auth configure-docker europe-west3-docker.pkg.dev
#   # or, for skopeo: skopeo login europe-west3-docker.pkg.dev
#
# Requires x86_64-linux to build the image (see README "Generating OCI Images").
set -euo pipefail

IMAGE="${IMAGE:-mina-indexer}"
TAG="${TAG:-$(git rev-parse --short=8 HEAD)}"
# REGISTRY is only required for an actual push; build-only/DRY_RUN works without it.
if [[ "${DRY_RUN:-0}" != "1" ]]; then
  : "${REGISTRY:?set REGISTRY, e.g. europe-west3-docker.pkg.dev/o1labs-192920/euro-docker-repo}"
fi
REF="${REGISTRY:-local}/${IMAGE}:${TAG}"

echo ">> Building OCI image with Nix (.#dockerImage)" >&2
nix build .#dockerImage --print-build-logs
# streamLayeredImage: ./result is an executable that streams a docker-archive
# tarball to stdout (not a tarball itself).
tarball="$(mktemp --suffix=.tar)"
trap 'rm -f "$tarball"' EXIT
./result > "$tarball"

if [[ "${DRY_RUN:-0}" == "1" ]]; then
  echo ">> DRY_RUN=1, built $(du -h "$tarball" | cut -f1) image, not pushing. Would push: ${REF}" >&2
  exit 0
fi

echo ">> Pushing ${REF}" >&2
if command -v skopeo >/dev/null 2>&1; then
  # Daemonless push (preferred in CI).
  skopeo copy "docker-archive:${tarball}" "docker://${REF}"
else
  # Fallback via the Docker daemon.
  loaded="$(docker load < "$tarball" | sed -n 's/^Loaded image: //p' | head -1)"
  docker tag "${loaded:-mina-indexer:$(git rev-parse --short=8 HEAD)}" "${REF}"
  docker push "${REF}"
fi

echo ">> Pushed ${REF}" >&2
