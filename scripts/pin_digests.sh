#!/usr/bin/env bash
# Resolve the CURRENT digests of the base images and write docker/digests.env.
# Commit that file: every later build then uses exactly these bytes.
set -euo pipefail
cd "$(dirname "$0")/.."
RUST_TAG="${RUST_TAG:-rust:1.90-bookworm}"
RUNTIME_TAG="${RUNTIME_TAG:-debian:bookworm-slim}"
OPA_TAG="${OPA_TAG:-openpolicyagent/opa:1.4.2}"
resolve() { docker pull -q "$1" >/dev/null; docker inspect --format='{{index .RepoDigests 0}}' "$1"; }
{
  echo "RUST_IMAGE=$(resolve "$RUST_TAG")"
  echo "RUNTIME_IMAGE=$(resolve "$RUNTIME_TAG")"
  echo "OPA_IMAGE=$(resolve "$OPA_TAG")"
} > docker/digests.env
cat docker/digests.env
echo "Wrote docker/digests.env - commit it."
