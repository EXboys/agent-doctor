#!/usr/bin/env bash
# Install repo git hooks (hard local fmt/clippy + release tag gate).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HOOK_DIR="$ROOT/.git/hooks"
SRC_DIR="$ROOT/scripts/githooks"

if [[ ! -d "$ROOT/.git" ]]; then
  echo "Not a git checkout: $ROOT" >&2
  exit 1
fi

mkdir -p "$HOOK_DIR"

for hook in pre-commit pre-push; do
  src="$SRC_DIR/$hook"
  dst="$HOOK_DIR/$hook"
  if [[ ! -f "$src" ]]; then
    echo "Missing hook source: $src" >&2
    exit 1
  fi
  cp "$src" "$dst"
  chmod +x "$dst"
  echo "Installed $dst"
done

cat <<'EOF'
Local gates (hard):
  pre-commit  — fmt-check + clippy when a commit touches Rust/Cargo
  pre-push    — fmt-check + clippy on EVERY push; v* tags also need release-preflight

Manual: ./scripts/check.sh lint
Bypass (emergency only):
  AGENT_DOCTOR_SKIP_LINT=1 git commit …
  AGENT_DOCTOR_SKIP_LINT=1 git push …
  AGENT_DOCTOR_SKIP_RELEASE_PREFLIGHT=1 git push --tags
EOF
