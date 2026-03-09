#!/usr/bin/env bash
set -euo pipefail

echo "UsageGuard currently publishes one-line installer support for Windows x64 only." >&2
echo "Use PowerShell on Windows:" >&2
echo "  irm https://raw.githubusercontent.com/openchert/usage-guard/main/install.ps1 | iex" >&2
exit 1
