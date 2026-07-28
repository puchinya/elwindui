#!/usr/bin/env bash
set -euo pipefail

ISSUE_NUMBER="${1:-}"
[[ "$ISSUE_NUMBER" =~ ^[1-9][0-9]*$ ]] || {
  echo "usage: $0 <issue-number>" >&2
  exit 1
}

REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || true)"
[[ -n "$REPO_ROOT" ]] || {
  echo "error: run inside a Git repository" >&2
  exit 1
}
cd "$REPO_ROOT"

EXCLUDE_FILE="$(git rev-parse --git-path info/exclude)"
mkdir -p "$(dirname "$EXCLUDE_FILE")"
touch "$EXCLUDE_FILE"
grep -qxF '.agent-state/' "$EXCLUDE_FILE" ||
  printf '%s\n' '.agent-state/' >> "$EXCLUDE_FILE"

BASE=".agent-state/issues/$ISSUE_NUMBER"
mkdir -p "$BASE/screenshots" "$BASE/logs"

printf '%s\n' \
  "$BASE/screenshots" \
  "$BASE/logs"
