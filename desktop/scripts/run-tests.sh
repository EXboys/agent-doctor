#!/usr/bin/env bash
# Run every desktop/tests/*.test.ts and fail if any of them fails.
# Each file is a standalone script (no test framework); a non-zero exit is a failure.
# Usage: bash scripts/run-tests.sh [tests/one.test.ts ...]
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR/.."

if (($# > 0)); then
  files=("$@")
else
  files=(tests/*.test.ts)
fi

failed=()
for file in "${files[@]}"; do
  echo "# $file"
  # node --import tsx, not the tsx CLI: the CLI opens a local pipe that sandboxes refuse.
  if ! node --import tsx "$file"; then
    failed+=("$file")
  fi
done

echo
if ((${#failed[@]} > 0)); then
  echo "${#failed[@]} of ${#files[@]} test files failed:"
  printf '  %s\n' "${failed[@]}"
  exit 1
fi
echo "all ${#files[@]} test files passed"
