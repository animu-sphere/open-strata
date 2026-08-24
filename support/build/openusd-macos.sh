#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != Darwin || "$(uname -m)" != arm64 ]]; then
  echo "openusd-macos.sh requires macOS arm64" >&2
  exit 2
fi
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
exec pwsh -NoProfile -File "${script_dir}/publish-openusd-runtimes.ps1" \
  -Version "${1:?OpenUSD version is required}" \
  -Variant "${2:?variant is required}" \
  "${@:3}"
