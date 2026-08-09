#!/usr/bin/env bash
# Launch Agent Doctor with embedded frontend (no Vite).
# Requires Cargo.toml default feature `custom-protocol`.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT/desktop"
npm run build
cd "$ROOT"
pkill -x agent-doctor-desktop 2>/dev/null || true
sleep 0.3
cargo build -p agent-doctor-desktop --release
open "$ROOT/target/release/agent-doctor-desktop"
echo "Opened Agent Doctor (embedded UI)."
