#!/usr/bin/env bash
# Print edition packaging metadata for CI / local scripts.
# Usage: eval "$(bash scripts/edition-meta.sh personal)"
set -euo pipefail

EDITION="${1:-personal}"
case "$EDITION" in
  personal)
    PRODUCT_NAME="Agent Doctor"
    BUNDLE_ID="com.agentdoctor.app"
    OSS_PREFIX="desktop"
    LATEST_JSON="latest.json"
    LATEST_GITHUB_JSON="latest.github.json"
    ;;
  team)
    PRODUCT_NAME="Agent Doctor Team"
    BUNDLE_ID="com.agentdoctor.team"
    OSS_PREFIX="desktop-team"
    LATEST_JSON="latest.team.json"
    LATEST_GITHUB_JSON="latest.team.github.json"
    ;;
  *)
    echo "unknown edition: $EDITION" >&2
    exit 2
    ;;
esac

# Shell-safe exports for `eval "$(…)"`.
printf "EDITION=%q\n" "$EDITION"
printf "PRODUCT_NAME=%q\n" "$PRODUCT_NAME"
printf "BUNDLE_ID=%q\n" "$BUNDLE_ID"
printf "OSS_PREFIX=%q\n" "$OSS_PREFIX"
printf "LATEST_JSON=%q\n" "$LATEST_JSON"
printf "LATEST_GITHUB_JSON=%q\n" "$LATEST_GITHUB_JSON"
