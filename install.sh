#!/usr/bin/env bash
set -euo pipefail

REPO="openchert/usage-guard"
API_URL="https://api.github.com/repos/${REPO}/releases/latest"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || { echo "Missing required command: $1" >&2; exit 1; }
}

need_cmd curl
need_cmd tar
# tar.gz assets only (no unzip required)

OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Linux) os_key="linux" ;;
  Darwin) os_key="macos" ;;
  *) echo "Unsupported OS: $OS" >&2; exit 1 ;;
esac

case "$ARCH" in
  x86_64|amd64) arch_key="x64" ;;
  arm64|aarch64)
    if [[ "$os_key" == "macos" ]]; then
      arch_key="arm64"
    else
      echo "Unsupported architecture for current release assets: $ARCH on $OS" >&2
      echo "Available now: Linux x64, Windows x64, macOS arm64" >&2
      exit 1
    fi
    ;;
  *) echo "Unsupported architecture: $ARCH" >&2; exit 1 ;;
esac

asset_basename="usage-guard-${os_key}-${arch_key}"

echo "Fetching latest release metadata..."
release_json="$(curl -fsSL "$API_URL")"
asset_url="$(printf '%s' "$release_json" | grep -oE "https://[^\"[:space:]]*${asset_basename}\\.tar\\.gz" | head -n1 || true)"

if [[ -z "$asset_url" ]]; then
  echo "Could not find asset: ${asset_basename}.tar.gz in latest release." >&2
  echo "Check https://github.com/${REPO}/releases" >&2
  exit 1
fi

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT
archive_path="$tmp_dir/${asset_basename}.tar.gz"

echo "Downloading $asset_basename..."
curl -fL "$asset_url" -o "$archive_path"

echo "Extracting..."
tar -xzf "$archive_path" -C "$tmp_dir"

mkdir -p "$INSTALL_DIR"
install -m 0755 "$tmp_dir/usageguard" "$INSTALL_DIR/usageguard"
install -m 0755 "$tmp_dir/usageguard-desktop" "$INSTALL_DIR/usageguard-desktop"

echo "Installed to $INSTALL_DIR"
if [[ ":$PATH:" != *":$INSTALL_DIR:"* ]]; then
  echo ""
  echo "Add to PATH (bash/zsh):"
  echo "  echo 'export PATH=\"$INSTALL_DIR:\$PATH\"' >> ~/.bashrc"
  echo "  source ~/.bashrc"
fi

echo ""
echo "Try:"
echo "  usageguard demo"
echo "  usageguard-desktop"
