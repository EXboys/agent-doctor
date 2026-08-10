#!/usr/bin/env bash
# Build the Agent Doctor CLI and copy it into src-tauri/resources so the
# desktop .app / installer ships a working binary (MCP wiring needs it).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
DESKTOP_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
ROOT="$(cd "$DESKTOP_DIR/.." && pwd)"
RESOURCES="$DESKTOP_DIR/src-tauri/resources"

cd "$ROOT"
mkdir -p "$RESOURCES"

echo "Building agent-doctor CLI (release)…"
cargo build -p agent-doctor --release

if [[ "${OS:-}" == "Windows_NT" || "$OSTYPE" == "msys" || "$OSTYPE" == "cygwin" ]]; then
  SRC="$ROOT/target/release/agent-doctor.exe"
  DEST="$RESOURCES/agent-doctor-cli.exe"
else
  SRC="$ROOT/target/release/agent-doctor"
  DEST="$RESOURCES/agent-doctor-cli"
fi

if [[ ! -f "$SRC" ]]; then
  echo "ERROR: CLI binary not found at $SRC" >&2
  exit 1
fi

cp -f "$SRC" "$DEST"
chmod +x "$DEST" 2>/dev/null || true
echo "Bundled CLI: $DEST"

# macOS: sign resource binary so notarization includes it (Tauri signs main exe only).
if [[ "$OSTYPE" == "darwin"* && -n "${APPLE_SIGNING_IDENTITY:-}" ]]; then
  codesign --force --sign "$APPLE_SIGNING_IDENTITY" \
    --options runtime --timestamp \
    "$DEST"
  echo "Signed resource CLI with APPLE_SIGNING_IDENTITY"
fi
