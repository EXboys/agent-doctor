#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

CLI_PACKAGES=(-p agent-doctor-core -p agent-doctor)
DESKTOP_PACKAGE=(-p agent-doctor-desktop)

usage() {
  cat <<'EOF'
Usage: scripts/check.sh [command]

Commands:
  fmt             Run rustfmt (write)
  fmt-check       Check rustfmt (CI)
  clippy-cli      Clippy for core + CLI
  clippy-desktop  Clippy for Tauri crate (needs GTK on Linux)
  test-cli        Test core + CLI
  build-cli       Release-build CLI
  frontend        Build desktop frontend (npm)
  cli             fmt-check + clippy-cli + test-cli + build-cli
  desktop         clippy-desktop
  release-preflight
                  fmt-check + clippy-cli + test-cli (+ desktop clippy when available)
                  Run this before tagging v*; Release CI re-checks the same gates.
  all             cli + frontend; desktop rust on macOS or AGENT_DOCTOR_CHECK_DESKTOP=1
  help            Show this message

Examples:
  ./scripts/check.sh cli
  ./scripts/check.sh release-preflight
  AGENT_DOCTOR_CHECK_DESKTOP=1 ./scripts/check.sh all
EOF
}

run_fmt() {
  cargo fmt --all
}

run_fmt_check() {
  cargo fmt --all -- --check
}

run_clippy_cli() {
  cargo clippy "${CLI_PACKAGES[@]}" --all-targets -- -D warnings
}

run_clippy_desktop() {
  cargo clippy "${DESKTOP_PACKAGE[@]}" --all-targets -- -D warnings
}

run_test_cli() {
  cargo test "${CLI_PACKAGES[@]}"
}

run_build_cli() {
  cargo build --release -p agent-doctor
}

run_frontend() {
  (cd desktop && npm ci && npm run build)
}

should_check_desktop() {
  [[ "${AGENT_DOCTOR_CHECK_DESKTOP:-}" == "1" ]] && return 0
  [[ "$(uname -s)" == "Darwin" ]] && return 0
  return 1
}

run_cli_suite() {
  run_fmt_check
  run_clippy_cli
  run_test_cli
  run_build_cli
}

run_release_preflight() {
  echo "==> release-preflight: fmt + clippy + tests (must pass before tagging v*)"
  run_fmt_check
  run_clippy_cli
  run_test_cli
  if should_check_desktop; then
    run_desktop_rust
  else
    echo "Skipping desktop clippy (set AGENT_DOCTOR_CHECK_DESKTOP=1 on Linux with GTK deps)."
    echo "Release CI still runs desktop clippy on macOS."
  fi
  echo "==> release-preflight OK — wait for green CI on main, then tag."
}

run_desktop_rust() {
  if [[ "$(uname -s)" == "Linux" ]]; then
    echo "Note: desktop Rust checks on Linux require GTK/WebKit dev packages."
    echo "      See docs/development.md if pkg-config fails."
  fi
  run_clippy_desktop
}

command="${1:-all}"

case "$command" in
  fmt) run_fmt ;;
  fmt-check) run_fmt_check ;;
  clippy-cli) run_clippy_cli ;;
  clippy-desktop) run_clippy_desktop ;;
  test-cli) run_test_cli ;;
  build-cli) run_build_cli ;;
  frontend) run_frontend ;;
  cli) run_cli_suite ;;
  desktop) run_desktop_rust ;;
  release-preflight) run_release_preflight ;;
  all)
    run_cli_suite
    run_frontend
    if should_check_desktop; then
      run_desktop_rust
    else
      echo "Skipping desktop Rust checks (set AGENT_DOCTOR_CHECK_DESKTOP=1 on Linux with GTK deps)."
    fi
    ;;
  help | -h | --help) usage ;;
  *)
    echo "Unknown command: $command" >&2
    usage >&2
    exit 1
    ;;
esac
