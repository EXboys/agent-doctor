#!/usr/bin/env bash
# The version declarations that ship together must agree. Cargo.toml is the
# source of truth; a bump that misses one of the others would release the
# desktop app or the npm manifest under a stale number without any CI failure.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

truth=$(sed -nE 's/^version = "([^"]+)"$/\1/p' "$ROOT/Cargo.toml" | head -n1)
if [[ -z "$truth" ]]; then
  echo "check-versions: could not read [workspace.package] version from Cargo.toml" >&2
  exit 1
fi

# Top-level "version" keys sit at two-space indent in both JSON manifests;
# nested ones are deeper, so the anchor does not pick them up.
json_version() {
  sed -nE 's/^  "version": "([^"]+)",?$/\1/p' "$1" | head -n1
}

# Cargo.lock lists the workspace crates; the CLI crate carries the version.
lock_version=$(awk '/^name = "agent-doctor"$/ { getline; sub(/^version = "/, ""); sub(/"$/, ""); print; exit }' "$ROOT/Cargo.lock")

status=0
check() {
  local label="$1" value="$2"
  if [[ -z "$value" ]]; then
    echo "check-versions: could not read a version from $label" >&2
    status=1
  elif [[ "$value" != "$truth" ]]; then
    echo "check-versions: $label is $value but Cargo.toml is $truth" >&2
    status=1
  fi
}

check "Cargo.lock (agent-doctor)" "$lock_version"
check "desktop/package.json" "$(json_version "$ROOT/desktop/package.json")"
check "desktop/package-lock.json" "$(json_version "$ROOT/desktop/package-lock.json")"
check "desktop/src-tauri/tauri.conf.json" "$(json_version "$ROOT/desktop/src-tauri/tauri.conf.json")"

if (( status != 0 )); then
  echo "check-versions: bump every manifest together (Cargo.toml, Cargo.lock, desktop/package.json, desktop/package-lock.json, desktop/src-tauri/tauri.conf.json)" >&2
  exit "$status"
fi
echo "check-versions: all manifests agree on $truth"
