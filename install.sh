#!/bin/sh
# Installs the TranscribeTube CLI (tube, transcribetube-mcp) on macOS or Linux.
#
#   curl -fsSL https://raw.githubusercontent.com/hicay/transcribetube-cli/main/install.sh | sh
#
# Options (environment):
#   TUBE_VERSION      version to install, e.g. 0.1.0 (default: latest release)
#   TUBE_INSTALL_DIR  where to put the binaries (default: ~/.local/bin)
set -eu

REPO="hicay/transcribetube-cli"
INSTALL_DIR="${TUBE_INSTALL_DIR:-$HOME/.local/bin}"

fail() {
    echo "error: $*" >&2
    exit 1
}

command -v curl >/dev/null 2>&1 || fail "curl is required"
command -v tar >/dev/null 2>&1 || fail "tar is required"

case "$(uname -s)" in
    Darwin) os="apple-darwin" ;;
    Linux) os="unknown-linux-musl" ;;
    *) fail "unsupported system: $(uname -s). On Windows, download the zip from https://github.com/$REPO/releases" ;;
esac
case "$(uname -m)" in
    arm64 | aarch64) arch="aarch64" ;;
    x86_64 | amd64) arch="x86_64" ;;
    *) fail "unsupported processor: $(uname -m)" ;;
esac
target="$arch-$os"

version="${TUBE_VERSION:-}"
if [ -z "$version" ]; then
    # The "latest" page redirects to .../releases/tag/vX.Y.Z.
    latest="$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest")" ||
        fail "could not reach GitHub"
    version="${latest##*/v}"
    case "$version" in
        [0-9]*) ;;
        *) fail "could not find the latest release of $REPO" ;;
    esac
fi
version="${version#v}"

archive="tube-v$version-$target.tar.gz"
base="https://github.com/$REPO/releases/download/v$version"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "Downloading tube $version for $target..."
curl -fsSL -o "$tmp/$archive" "$base/$archive" || fail "no release file $archive for version $version"

if curl -fsSL -o "$tmp/SHA256SUMS" "$base/SHA256SUMS" 2>/dev/null; then
    expected="$(grep " $archive\$" "$tmp/SHA256SUMS" | cut -d' ' -f1)"
    if command -v sha256sum >/dev/null 2>&1; then
        actual="$(sha256sum "$tmp/$archive" | cut -d' ' -f1)"
    elif command -v shasum >/dev/null 2>&1; then
        actual="$(shasum -a 256 "$tmp/$archive" | cut -d' ' -f1)"
    else
        actual=""
    fi
    if [ -n "$expected" ] && [ -n "$actual" ] && [ "$expected" != "$actual" ]; then
        fail "checksum mismatch for $archive"
    fi
fi

tar -xzf "$tmp/$archive" -C "$tmp"
mkdir -p "$INSTALL_DIR"
for name in tube transcribetube-mcp; do
    cp "$tmp/tube-v$version-$target/$name" "$INSTALL_DIR/$name"
    chmod 755 "$INSTALL_DIR/$name"
done

echo "Installed tube and transcribetube-mcp to $INSTALL_DIR"
case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *)
        echo
        echo "$INSTALL_DIR is not on your PATH. Add this line to ~/.zshrc or ~/.bashrc:"
        echo "  export PATH=\"$INSTALL_DIR:\$PATH\""
        ;;
esac
echo
echo "Next: tube login"
