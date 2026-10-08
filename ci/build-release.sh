#!/usr/bin/env bash
# Build, verify and stage one release binary.
#
#   ci/build-release.sh <rust-target> [out-dir]
#
# Targets:
#   *-unknown-linux-musl  static binary (runs anywhere). Plain cargo, no C toolchain.
#   *-unknown-linux-gnu   glibc binary (faster, see README). Built with
#                         cargo-zigbuild against glibc $GLIBC_VERSION (default
#                         2.28), so it runs on RHEL 8 / Debian 10 / Ubuntu 20.04+.
#                         Zig also handles the aarch64 cross-link.
#
# Used by .gitlab-ci.yml and .github/workflows/release.yml, and works locally.
set -euo pipefail

target=${1:?usage: $0 <rust-target> [out-dir]}
out=${2:-dist}
glibc=${GLIBC_VERSION:-2.28}
zigbuild_pkgs=${ZIGBUILD_PKGS:-"cargo-zigbuild==0.23.4 ziglang==0.16.0"}

cd "$(dirname "$0")/.."
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
arch=${target%%-*}
libc=${target##*-}
name="ct-${version}-${arch}-linux-${libc}"

ensure_zigbuild() {
    command -v cargo-zigbuild >/dev/null && return
    local venv=${ZIGBUILD_VENV:-${HOME}/.cache/ct-zigbuild}
    if [[ ! -x "$venv/bin/cargo-zigbuild" ]]; then
        echo ">> installing $zigbuild_pkgs into $venv"
        if ! python3 -m venv "$venv" 2>/dev/null; then
            # Debian/Ubuntu ship ensurepip separately.
            if [[ $(id -u) == 0 ]] && command -v apt-get >/dev/null; then
                apt-get update -qq && apt-get install -y -qq python3-venv >/dev/null
                python3 -m venv "$venv"
            else
                echo "error: python3 venv support is required for glibc builds" >&2
                exit 1
            fi
        fi
        # shellcheck disable=SC2086
        "$venv/bin/pip" install --quiet --disable-pip-version-check $zigbuild_pkgs
    fi
    export PATH="$venv/bin:$PATH"
}

rustup target add "$target" >/dev/null 2>&1 || true

case "$libc" in
    musl)
        cargo build --release --locked --target "$target"
        ;;
    gnu)
        ensure_zigbuild
        cargo zigbuild --release --locked --target "${target}.${glibc}"
        ;;
    *)
        echo "unsupported target: $target" >&2
        exit 2
        ;;
esac

mkdir -p "$out"
bin="$out/$name"
cp "target/$target/release/ct" "$bin"

echo ">> verifying $bin"
info=$(file -b "$bin" 2>/dev/null || true)
[[ -n "$info" ]] && echo "   $info"
if [[ $libc == musl ]]; then
    if [[ -n "$info" && $info != *static* ]]; then
        echo "error: musl binary is not statically linked" >&2
        exit 1
    fi
else
    if command -v objdump >/dev/null; then
        need=$(objdump -T "$bin" | grep -o 'GLIBC_[0-9.]*' | sed 's/GLIBC_//' | sort -Vu | tail -1)
        echo "   requires glibc >= $need"
        if [[ $(printf '%s\n%s\n' "$need" "$glibc" | sort -V | tail -1) != "$glibc" ]]; then
            echo "error: binary needs glibc $need > $glibc" >&2
            exit 1
        fi
    fi
fi
echo "   size: $(wc -c <"$bin") bytes"

# Smoke test when the binary can run here.
if [[ $arch == "$(uname -m)" ]]; then
    "$bin" --version
    printf 'ERROR from 10.0.0.1 in 15ms\n' | "$bin" -N --rgb
    "$bin" -N config check >/dev/null
fi
echo ">> $bin"
