#!/usr/bin/env bash
# Fail unless the tag (v<version>) matches Cargo.toml: ci/check-version.sh <tag>
set -euo pipefail
tag=${1:?usage: $0 <tag>}
version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$(dirname "$0")/../Cargo.toml" | head -1)
if [[ "$tag" != "v${version}" ]]; then
    echo "error: tag $tag does not match Cargo.toml version $version (expected v${version})" >&2
    exit 1
fi
echo "tag $tag matches Cargo.toml"
