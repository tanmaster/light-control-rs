#!/usr/bin/env bash
# Fail if the Linux binary links a shared library that a stock Ubuntu desktop does not ship.
# Usage: tools/linux/check-libs.sh [path/to/light-control]   (used by CI and tools/linux/check.sh)
set -euo pipefail
bin=${1:-target/release/light-control}
allowed='^(libc|libm|libgcc_s|libudev|libxcb|libglib-2\.0|libgobject-2\.0|libgio-2\.0|libgdk-3|libgtk-3|libgdk_pixbuf-2\.0|libcairo|libpipewire-0\.3)\.so'
needed=$(readelf -d "$bin" | sed -n 's/.*Shared library: \[\(.*\)\]/\1/p')
echo "$needed"
bad=$(echo "$needed" | grep -Ev "$allowed" || true)
if [ -n "$bad" ]; then
  echo "::error::unexpected runtime libraries: $bad"
  exit 1
fi
