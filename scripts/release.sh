#!/usr/bin/env bash
# Cut a release locally: preflight → annotated tag → push (main + tag).
# Usage: ./scripts/release.sh [vX.Y.Z]
# If omitted, reads version from Cargo.toml workspace.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if [[ ! -x "$ROOT/.git/hooks/pre-push" ]]; then
  echo "==> installing git hooks (release pre-push gate)"
  ./scripts/install-git-hooks.sh
fi

version_arg="${1:-}"
workspace_version="$(
  python3 - <<'PY'
from pathlib import Path
import re
text = Path("Cargo.toml").read_text()
m = re.search(r'^version\s*=\s*"([^"]+)"', text, re.M)
print(m.group(1) if m else "")
PY
)"

if [[ -z "$workspace_version" ]]; then
  echo "Could not read workspace version from Cargo.toml" >&2
  exit 1
fi

if [[ -n "$version_arg" ]]; then
  tag="${version_arg#v}"
  tag="v$tag"
  expected="${tag#v}"
  if [[ "$expected" != "$workspace_version" ]]; then
    echo "Tag $tag does not match Cargo.toml version $workspace_version" >&2
    echo "Bump manifests first (Cargo.toml, desktop/package.json, tauri.conf.json, locks)." >&2
    exit 1
  fi
else
  tag="v$workspace_version"
fi

if [[ -n "$(git status --porcelain)" ]]; then
  echo "Working tree is dirty. Commit or stash before releasing." >&2
  git status --short >&2
  exit 1
fi

branch="$(git rev-parse --abbrev-ref HEAD)"
if [[ "$branch" != "main" ]]; then
  echo "Refusing to release from branch '$branch' (expected main)." >&2
  exit 1
fi

if git rev-parse "$tag" >/dev/null 2>&1; then
  echo "Tag $tag already exists locally." >&2
  exit 1
fi

echo "==> release $tag"
./scripts/check.sh release-preflight

echo "==> creating annotated tag $tag"
git tag -a "$tag" -m "$tag"

echo "==> pushing main + $tag"
git push origin HEAD
git push origin "$tag"

echo "==> done. Watch Release workflow: gh run list --workflow Release --limit 3"
