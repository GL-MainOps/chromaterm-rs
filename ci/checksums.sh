#!/usr/bin/env bash
# Write SHA256SUMS for every release binary in a directory: ci/checksums.sh [dir]
set -euo pipefail
cd "${1:-dist}"
sha256sum ct-* | grep -v '\.sha256' > SHA256SUMS
cat SHA256SUMS
