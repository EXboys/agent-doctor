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

IS_WINDOWS=0
if [[ "${OS:-}" == "Windows_NT" || "$OSTYPE" == "msys" || "$OSTYPE" == "cygwin" ]]; then
  IS_WINDOWS=1
fi

echo "Building agent-doctor CLI (release)…"
if [[ "$IS_WINDOWS" -eq 1 ]]; then
  # Static VCRuntime so bundled agent-doctor-cli.exe runs on fresh Windows
  # without asking users to install Visual C++ Redistributable.
  export RUSTFLAGS="${RUSTFLAGS:-} -C target-feature=+crt-static"
fi
cargo build -p agent-doctor --release

if [[ "$IS_WINDOWS" -eq 1 ]]; then
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
