#!/usr/bin/env bash
# Run Tauri CLI with edition env + config merge (bundle id / updater channel).
# Usage: bash scripts/tauri-with-edition.sh <personal|team> <tauri-args...>
# Example: bash scripts/tauri-with-edition.sh personal build --bundles app
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
DESKTOP_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
EDITION="${1:-}"
if [[ -z "$EDITION" ]]; then
  echo "usage: $0 <personal|team> <tauri args...>" >&2
  exit 2
fi
shift

case "$EDITION" in
  personal|team) ;;
  *)
    echo "unknown edition: $EDITION (expected personal|team)" >&2
    exit 2
    ;;
esac

CONFIG="$DESKTOP_DIR/src-tauri/tauri.${EDITION}.conf.json"
if [[ ! -f "$CONFIG" ]]; then
  echo "missing config: $CONFIG" >&2
  exit 1
fi

export AGENT_DOCTOR_EDITION="$EDITION"
cd "$DESKTOP_DIR"
echo "AGENT_DOCTOR_EDITION=$AGENT_DOCTOR_EDITION"
echo "tauri --config src-tauri/tauri.${EDITION}.conf.json $*"
exec npx tauri "$@" --config "src-tauri/tauri.${EDITION}.conf.json"
