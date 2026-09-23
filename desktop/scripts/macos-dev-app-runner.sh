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
if command -v codesign >/dev/null 2>&1; then
  echo "▸ Signing $bundle_dir"
  if [ -f "$entitlements" ]; then
    codesign --force --deep --sign - --entitlements "$entitlements" "$bundle_dir"
  else
    codesign --force --deep --sign - "$bundle_dir"
  fi
fi

# FIFO pair so `tauri dev` still streams app stdout/stderr.
log_dir="$(mktemp -d)"
stdout_fifo="$log_dir/stdout"
stderr_fifo="$log_dir/stderr"
mkfifo "$stdout_fifo" "$stderr_fifo"

cleanup() {
  rm -rf "$log_dir"
  if [ -n "${open_pid:-}" ] && kill -0 "$open_pid" 2>/dev/null; then
    kill "$open_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT
trap 'kill $open_pid 2>/dev/null || true; exit 130' INT TERM

cat "$stdout_fifo" &
cat_stdout_pid=$!
cat "$stderr_fifo" >&2 &
cat_stderr_pid=$!

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

echo "▸ Launching $bundle_dir via Launch Services (TCC-safe)"
open_cmd=(open -W -a "$bundle_dir" --stdout "$stdout_fifo" --stderr "$stderr_fifo")
if [ "${#env_args[@]}" -gt 0 ]; then
  open_cmd+=("${env_args[@]}")
fi
if [ "${#app_args[@]}" -gt 0 ]; then
  open_cmd+=(--args "${app_args[@]}")
fi
"${open_cmd[@]}" &
open_pid=$!

wait "$open_pid"
exit_code=$?

wait "$cat_stdout_pid" "$cat_stderr_pid" 2>/dev/null || true

exit "$exit_code"
