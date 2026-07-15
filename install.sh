#!/bin/sh
# Install the latest rust-star-history release binary for this platform.
#
#   curl -fsSL https://raw.githubusercontent.com/Flux159/rust-star-history/main/install.sh | sh
#
# Installs to ~/.local/bin by default; override with INSTALL_DIR:
#   curl -fsSL .../install.sh | INSTALL_DIR=/usr/local/bin sh
set -eu

REPO="Flux159/rust-star-history"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"

case "$(uname -m)" in
  x86_64 | amd64) arch="x86_64" ;;
  aarch64 | arm64) arch="aarch64" ;;
  *)
    echo "error: unsupported architecture $(uname -m); build from source instead:" >&2
    echo "  cargo install --git https://github.com/${REPO}" >&2
    exit 1
    ;;
esac

case "$(uname -s)" in
  Linux) triple="${arch}-unknown-linux-gnu" ;;
  Darwin)
    if [ "$arch" != "aarch64" ]; then
      echo "error: no prebuilt binary for Intel macs; build from source instead:" >&2
      echo "  cargo install --git https://github.com/${REPO}" >&2
      exit 1
    fi
    triple="${arch}-apple-darwin"
    ;;
  MINGW* | MSYS* | CYGWIN*) triple="${arch}-pc-windows-msvc" ;;
  *)
    echo "error: unsupported OS $(uname -s)" >&2
    exit 1
    ;;
esac

asset="rust-star-history-${triple}.tar.gz"
url="https://github.com/${REPO}/releases/latest/download/${asset}"

mkdir -p "$INSTALL_DIR"
echo "Downloading ${url}"
tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT
curl -fsSL -o "$tmp" "$url" || {
  echo "error: download failed — the latest release may still be publishing its assets; retry shortly" >&2
  exit 1
}
tar -xzf "$tmp" -C "$INSTALL_DIR"

echo "Installed $("$INSTALL_DIR/rust-star-history" --version) to ${INSTALL_DIR}"
case ":${PATH}:" in
  *":${INSTALL_DIR}:"*) ;;
  *) echo "note: ${INSTALL_DIR} is not on your PATH — add it, e.g.: export PATH=\"${INSTALL_DIR}:\$PATH\"" ;;
esac
