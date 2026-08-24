#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != Linux || "$(uname -m)" != x86_64 ]]; then
  echo "openusd-linux.sh requires Linux x86_64" >&2
  exit 2
fi
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
exec pwsh -NoProfile -File "${script_dir}/publish-openusd-runtimes.ps1" \
  -Version "${1:?OpenUSD version is required}" \
  -Variant "${2:?variant is required}" \
  "${@:3}"
