#!/usr/bin/env bash
# Build, test, lint and check runtime libraries for Linux in a local Docker container,
# so Linux problems show up before pushing. Needs Docker (or OrbStack / Colima).
# Usage: tools/linux/check.sh
# Build output and the cargo registry are cached in the Docker volumes lc-target and lc-cargo.
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
docker build -q -t light-control-linux "$root/tools/linux" > /dev/null
docker run --rm \
  -v "$root":/src:ro -v lc-target:/target -v lc-cargo:/usr/local/cargo/registry \
  -e CARGO_TARGET_DIR=/target -w /src light-control-linux \
  bash -c 'set -e
    cargo clippy --release -- -D warnings
    cargo test --release
    cargo build --release
    tools/linux/check-libs.sh /target/release/light-control'
