#!/usr/bin/env bash
# Install repo git hooks (pre-push release gate).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HOOK_DIR="$ROOT/.git/hooks"
SRC="$ROOT/scripts/githooks/pre-push"
DST="$HOOK_DIR/pre-push"

if [[ ! -d "$ROOT/.git" ]]; then
  echo "Not a git checkout: $ROOT" >&2
  exit 1
fi

mkdir -p "$HOOK_DIR"
cp "$SRC" "$DST"
chmod +x "$DST"
echo "Installed $DST"
echo "Pushing refs/tags/v* now requires ./scripts/check.sh release-preflight (or scripts/release.sh)."
echo "Emergency bypass: AGENT_DOCTOR_SKIP_RELEASE_PREFLIGHT=1 git push --tags"
