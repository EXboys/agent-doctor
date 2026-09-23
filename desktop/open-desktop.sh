#!/usr/bin/env bash
# Launch Agent Doctor with embedded frontend (no Vite).
# Requires Cargo.toml default feature `custom-protocol`.
# On macOS, wraps the binary in a .app so mic/speech TCC prompts work.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT/desktop"
npm run build
cd "$ROOT"
pkill -x agent-doctor-desktop 2>/dev/null || true
sleep 0.3
cargo build -p agent-doctor-desktop --release

BIN="$ROOT/target/release/agent-doctor-desktop"
if [[ "$(uname -s)" == "Darwin" ]]; then
  export AGENT_DOCTOR_EDITION="${AGENT_DOCTOR_EDITION:-personal}"
  # Reuse the same Launch Services wrapper as tauri:dev.
  exec bash "$ROOT/desktop/scripts/macos-dev-app-runner.sh" "$BIN"
fi

open "$BIN"
echo "Opened Agent Doctor (embedded UI)."
