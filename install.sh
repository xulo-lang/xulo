#!/bin/sh
# Install a prebuilt Xulo release from GitHub.
#
#   curl -fsSL https://github.com/xulo-lang/xulo/releases/latest/download/install.sh | sh
#
# Environment variables:
#   XULO_VERSION   release version to install (default: latest), e.g. 0.1.0
#   XULO_PREFIX    install prefix (default: $HOME/.local)

set -eu

REPO="xulo-lang/xulo"
VERSION="${XULO_VERSION:-latest}"
PREFIX="${XULO_PREFIX:-$HOME/.local}"

err() {
    echo "install.sh: $*" >&2
    exit 1
}

command -v curl >/dev/null 2>&1 || err "curl is required"
command -v tar >/dev/null 2>&1 || err "tar is required"

os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
    Linux)
        case "$arch" in
            x86_64) triple="x86_64-unknown-linux-gnu" ;;
            *) err "unsupported architecture: $arch" ;;
        esac
        ;;
    Darwin)
        case "$arch" in
            x86_64) triple="x86_64-apple-darwin" ;;
            arm64) triple="aarch64-apple-darwin" ;;
            *) err "unsupported architecture: $arch" ;;
        esac
        ;;
    *) err "unsupported OS: $os" ;;
esac

if [ "$VERSION" = "latest" ]; then
    tag="$(curl -fsSLI -o /dev/null -w '%{url_effective}' \
        "https://github.com/${REPO}/releases/latest" | sed 's#.*/tag/##')"
    case "$tag" in
        v[0-9]*) ;;
        *) err "could not determine the latest release" ;;
    esac
else
    case "$VERSION" in
        v*) tag="$VERSION" ;;
        *) tag="v$VERSION" ;;
    esac
fi

version="${tag#v}"
tarball="xulo-${version}-${triple}.tar.gz"
base="https://github.com/${REPO}/releases/download/${tag}"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

echo "Downloading Xulo ${version}..."
curl -fsSL "${base}/${tarball}" -o "${tmp}/${tarball}"
curl -fsSL "${base}/SHA256SUMS" -o "${tmp}/SHA256SUMS"

if command -v sha256sum >/dev/null 2>&1; then
    check="sha256sum"
elif command -v shasum >/dev/null 2>&1; then
    check="shasum -a 256"
else
    err "sha256sum or shasum is required"
fi
(cd "$tmp" && grep " ${tarball}\$" SHA256SUMS | $check -c - >/dev/null) \
    || err "checksum verification failed"

tar -xzf "${tmp}/${tarball}" -C "$tmp"

mkdir -p "${PREFIX}/bin" "${PREFIX}/lib/xulo"
for bin in xulo xupm xulo-analyzer; do
    src="${tmp}/xulo-${version}-${triple}/bin/${bin}"
    [ -f "$src" ] || continue
    cp "$src" "${PREFIX}/bin/${bin}"
    chmod 755 "${PREFIX}/bin/${bin}"
done
cp "${tmp}/xulo-${version}-${triple}/lib/xulo/libxulo_runtime.a" "${PREFIX}/lib/xulo/"
chmod 644 "${PREFIX}/lib/xulo/libxulo_runtime.a"

echo "Installed Xulo ${version} to ${PREFIX}/bin"
case ":${PATH}:" in
    *":${PREFIX}/bin:"*) ;;
    *) echo "Add ${PREFIX}/bin to PATH to use the Xulo tools." ;;
esac
