#!/bin/bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
if [[ "$(uname -s)" != Darwin || "$(uname -m)" != arm64 ]]; then
  echo 'This build currently targets Apple Silicon macOS.' >&2
  exit 2
fi
profile=debug
if [[ "${1:-}" == --release && $# == 1 ]]; then
  profile=release
elif [[ $# != 0 ]]; then
  echo 'Usage: scripts/build-macos.sh [--release]' >&2
  exit 2
fi
command -v cargo >/dev/null || { echo 'Rust cargo is required (install a current stable Rust toolchain).' >&2; exit 2; }
command -v npm >/dev/null || { echo 'Node.js >=20.19 and npm are required.' >&2; exit 2; }
if [[ ! -d packages/explorer-ui/node_modules ]]; then
  npm --prefix packages/explorer-ui ci
fi
npm --prefix packages/explorer-ui run typecheck
npm --prefix packages/explorer-ui run build
if [[ "$profile" == release ]]; then
  cargo build --locked -p code-codex --bin code-codex --release
else
  cargo build --locked -p code-codex --bin code-codex
fi
"target/$profile/code-codex" --version
