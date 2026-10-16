#!/usr/bin/env bash
# cargo-tauri runner for macOS development builds.
#
# TCC attributes mic/speech privacy prompts to the *responsible* process. When
# `tauri dev` exec's the bare binary (especially under Cursor/IDE), TCC checks
# that parent's Info.plist — not ours — and aborts with SIGABRT for missing
# NSSpeechRecognitionUsageDescription. Fix: wrap the binary in a signed .app
# and launch via Launch Services (`open -W`) so we are the responsible process.
if [ -z "${BASH_VERSION:-}" ]; then
  exec /usr/bin/env bash "$0" "$@"
fi

set -euo pipefail

if [ "$#" -lt 1 ]; then
  echo "usage: macos-dev-app-runner.sh <binary>|run [cargo/app args...]" >&2
  exit 64
fi

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
DESKTOP_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
WORKSPACE_DIR="$(cd "$DESKTOP_DIR/.." && pwd)"
SRC_TAURI="$DESKTOP_DIR/src-tauri"
BINARY_NAME="agent-doctor-desktop"

edition="${AGENT_DOCTOR_EDITION:-personal}"
case "$edition" in
  team)
    bundle_id="com.agentdoctor.team.dev"
    bundle_name="Agent Doctor Team Dev"
    ;;
  *)
    bundle_id="com.agentdoctor.app.dev"
    bundle_name="Agent Doctor Dev"
    ;;
esac

if [ -f "$1" ]; then
  case "$1" in
    *.app/Contents/MacOS/*)
      echo "pass the cargo binary (target/<profile>/$BINARY_NAME), not the one inside the .app: $1" >&2
      exit 64
      ;;
  esac
  binary="$1"
  shift
  app_args=("$@")
else
  if [ "$1" != "run" ]; then
    echo "unsupported Tauri runner command: $1" >&2
    exit 64
  fi
  shift

  build_args=(build --manifest-path "$SRC_TAURI/Cargo.toml")
  app_args=()
  profile=debug
  target_triple=""
  in_app_args=false

  while [ "$#" -gt 0 ]; do
    if [ "$in_app_args" = true ]; then
      app_args+=("$1")
      shift
      continue
    fi

    case "$1" in
      --)
        in_app_args=true
        shift
        ;;
      --release)
        profile=release
        build_args+=("$1")
        shift
        ;;
      --target)
        if [ "$#" -lt 2 ]; then
          echo "missing value for --target" >&2
          exit 64
        fi
        build_args+=("$1" "$2")
        target_triple="$2"
        shift 2
        ;;
      --target=*)
        build_args+=("$1")
        target_triple="${1#--target=}"
        shift
        ;;
      *)
        build_args+=("$1")
        shift
        ;;
    esac
  done

  cargo "${build_args[@]}"

  target_dir="${CARGO_TARGET_DIR:-$WORKSPACE_DIR/target}"
  if [[ "$target_dir" != /* ]]; then
    target_dir="$WORKSPACE_DIR/$target_dir"
  fi

  if [ -n "$target_triple" ]; then
    binary="$target_dir/$target_triple/$profile/$BINARY_NAME"
  else
    binary="$target_dir/$profile/$BINARY_NAME"
  fi
fi

if [ ! -f "$binary" ]; then
  echo "built binary not found: $binary" >&2
  exit 66
fi

binary_dir="$(cd "$(dirname "$binary")" && pwd)"
bundle_dir="${AGENT_DOCTOR_DEV_APP_BUNDLE:-$binary_dir/${bundle_name}.app}"
contents_dir="$bundle_dir/Contents"
macos_dir="$contents_dir/MacOS"
resources_dir="$contents_dir/Resources"
bundle_executable="$macos_dir/$BINARY_NAME"

mkdir -p "$macos_dir" "$resources_dir"
rm -f "$bundle_executable"
cp "$binary" "$bundle_executable"
chmod +x "$bundle_executable"

# Merge privacy strings from src-tauri/Info.plist into a full app Info.plist.
python3 - "$contents_dir/Info.plist" "$bundle_id" "$bundle_name" "$BINARY_NAME" "$SRC_TAURI/Info.plist" <<'PY'
import plistlib
import sys
from pathlib import Path

out, bundle_id, bundle_name, exe, privacy_path = sys.argv[1:6]
info = {
    "CFBundleDevelopmentRegion": "en",
    "CFBundleExecutable": exe,
    "CFBundleIdentifier": bundle_id,
    "CFBundleInfoDictionaryVersion": "6.0",
    "CFBundleName": bundle_name,
    "CFBundleDisplayName": bundle_name,
    "CFBundlePackageType": "APPL",
    "CFBundleShortVersionString": "0.0.0-dev",
    "CFBundleVersion": "0",
    "LSMinimumSystemVersion": "11.0",
    "NSHighResolutionCapable": True,
}
privacy = Path(privacy_path)
if privacy.is_file():
    with privacy.open("rb") as f:
        extra = plistlib.load(f)
    if isinstance(extra, dict):
        info.update(extra)
# Fallbacks if Info.plist is incomplete
info.setdefault(
    "NSMicrophoneUsageDescription",
    "需要用麦克风听你说话，才能把话写进问答框。",
)
info.setdefault(
    "NSSpeechRecognitionUsageDescription",
    "需要用语音识别，才能把你说的话写成文字，方便提问。",
)
with open(out, "wb") as f:
    plistlib.dump(info, f, sort_keys=False)
PY

if command -v xattr >/dev/null 2>&1; then
  xattr -dr com.apple.quarantine "$bundle_dir" 2>/dev/null || true
fi

entitlements="$SRC_TAURI/Entitlements.plist"
# Ad-hoc signing (`-`) changes every rebuild, so macOS forgets Accessibility.
# A stable Apple Development identity keeps that allow across relaunches.
sign_identity="-"
if command -v security >/dev/null 2>&1; then
  found=$(security find-identity -v -p codesigning 2>/dev/null | awk '/Apple Development/ { print $2; exit }')
  if [ -n "${found:-}" ]; then
    sign_identity="$found"
  fi
fi
if command -v codesign >/dev/null 2>&1; then
  echo "▸ Signing $bundle_dir ($sign_identity)"
  if [ -f "$entitlements" ]; then
    codesign --force --deep --sign "$sign_identity" --entitlements "$entitlements" "$bundle_dir"
  else
    codesign --force --deep --sign "$sign_identity" "$bundle_dir"
  fi
fi

# The app outlives this script (launchd is its parent), so its stdout/stderr
# must be regular files: a FIFO whose reader is gone blocks the app's main
# thread on the next log write and freezes the window.
log_dir="${TMPDIR:-/tmp}/agent-doctor-dev"
mkdir -p "$log_dir"
stdout_log="$log_dir/${bundle_id}.stdout.log"
stderr_log="$log_dir/${bundle_id}.stderr.log"
touch "$stdout_log" "$stderr_log"

tail -n 0 -F "$stdout_log" 2>/dev/null &
tail_stdout_pid=$!
tail -n 0 -F "$stderr_log" >&2 2>/dev/null &
tail_stderr_pid=$!

cleanup() {
  if [ -n "${open_pid:-}" ] && kill -0 "$open_pid" 2>/dev/null; then
    kill "$open_pid" 2>/dev/null || true
  fi
  kill "$tail_stdout_pid" "$tail_stderr_pid" 2>/dev/null || true
}
trap cleanup EXIT
trap 'exit 130' INT TERM

env_args=()
for var in AGENT_DOCTOR_EDITION RUST_LOG RUST_BACKTRACE TAURI_ENV_DEBUG TAURI_DEV_HOST; do
  if [ -n "${!var:-}" ]; then
    env_args+=(--env "$var=${!var}")
  fi
done
# Forward any remaining TAURI_* / AGENT_DOCTOR_* without duplicating known keys.
while IFS= read -r var; do
  case "$var" in
    AGENT_DOCTOR_EDITION|RUST_LOG|RUST_BACKTRACE|TAURI_ENV_DEBUG|TAURI_DEV_HOST) continue ;;
    AGENT_DOCTOR_*|TAURI_*)
      if [ -n "${!var:-}" ]; then
        env_args+=(--env "$var=${!var}")
      fi
      ;;
  esac
done < <(compgen -e)

# `open` only brings an already-running copy forward, so a rebuild would keep
# serving the old backend. Stop it first.
if pkill -f "$bundle_executable" 2>/dev/null; then
  for _ in $(seq 1 50); do
    pgrep -f "$bundle_executable" >/dev/null 2>&1 || break
    sleep 0.1
  done
  pkill -9 -f "$bundle_executable" 2>/dev/null || true
fi
: > "$stdout_log"
: > "$stderr_log"

echo "▸ Launching $bundle_dir via Launch Services (TCC-safe)"
echo "▸ App logs: $stderr_log"
open_cmd=(open -W -a "$bundle_dir" --stdout "$stdout_log" --stderr "$stderr_log")
if [ "${#env_args[@]}" -gt 0 ]; then
  open_cmd+=("${env_args[@]}")
fi
if [ "${#app_args[@]}" -gt 0 ]; then
  open_cmd+=(--args "${app_args[@]}")
fi
"${open_cmd[@]}" &
open_pid=$!

exit_code=0
wait "$open_pid" || exit_code=$?
sleep 0.3

exit "$exit_code"
