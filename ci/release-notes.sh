#!/usr/bin/env bash
# Print Markdown release notes for a tag: ci/release-notes.sh <tag>
# Changes come from Conventional Commit subjects since the previous tag.
set -euo pipefail
tag=${1:?usage: $0 <tag>}
version=${tag#v}
prev=$(git describe --tags --abbrev=0 "${tag}^" 2>/dev/null || true)
range=${prev:+${prev}..}${tag}

section() { # <title> <regex>
    local lines
    lines=$(git log --no-merges --pretty='%s' "$range" | grep -E "$2" | sed 's/^/- /' || true)
    [[ -n $lines ]] && printf '### %s\n%s\n\n' "$1" "$lines"
    return 0
}

cat <<MD
## ct ${version}

| File | Platform | Notes |
|---|---|---|
| \`ct-${version}-x86_64-linux-gnu\` | x86_64 Linux, glibc ≥ 2.28 | **fastest** (recommended for most distros) |
| \`ct-${version}-aarch64-linux-gnu\` | arm64 Linux, glibc ≥ 2.28 | fastest on arm64 |
| \`ct-${version}-x86_64-linux-musl\` | x86_64, any Linux | fully static: Alpine, scratch containers, old systems |
| \`ct-${version}-aarch64-linux-musl\` | arm64, any Linux | fully static |

\`\`\`sh
curl -LO <url-of-binary> && sha256sum -c --ignore-missing SHA256SUMS
install -Dm755 ct-${version}-x86_64-linux-gnu ~/.local/bin/ct
\`\`\`

MD
section "Features" '^feat(\(.+\))?!?:'
section "Fixes" '^fix(\(.+\))?!?:'
section "Performance" '^perf(\(.+\))?!?:'
section "Other changes" '^(build|ci|docs|refactor|test|chore|style)(\(.+\))?!?:'
[[ -n $prev ]] && echo "Full changelog: ${prev}...${tag}"
exit 0
