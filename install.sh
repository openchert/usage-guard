#!/usr/bin/env bash
set -euo pipefail

echo "UsageGuard publishes Linux desktop builds as GitHub release assets." >&2
echo "Install the latest Debian/Ubuntu package (recommended for launcher integration and Start on Login):" >&2
echo "  sudo apt install ./UsageGuard_*_amd64.deb" >&2
echo "Run the portable AppImage:" >&2
echo "  chmod +x ./UsageGuard_*.AppImage && ./UsageGuard_*.AppImage" >&2
echo "Windows one-line installer:" >&2
echo "  irm https://raw.githubusercontent.com/openchert/usage-guard/main/install.ps1 | iex" >&2
