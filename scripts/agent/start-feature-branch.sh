#!/usr/bin/env bash
set -euo pipefail

# Create or switch to a source-code feature branch associated with one Issue.
#
# Usage:
#   scripts/agent/start-feature-branch.sh 123 "graphics path"
#
# The branch is created from the current remote default branch:
#   feature/123-graphics-path
#
# stdout contains only the resolved branch name.

ISSUE_NUMBER="${1:-}"
shift || true
DESCRIPTION="${*:-}"

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "error: required command not found: $1" >&2
    exit 1
  fi
}

require_command git
require_command gh
require_command python3

if [[ ! "$ISSUE_NUMBER" =~ ^[1-9][0-9]*$ ]]; then
  echo "error: Issue number must be a positive integer" >&2
  echo "usage: $0 <issue-number> <short-description>" >&2
  exit 1
fi

if [[ -z "${DESCRIPTION//[[:space:]]/}" ]]; then
  echo "error: short description is required" >&2
  echo "usage: $0 <issue-number> <short-description>" >&2
  exit 1
fi

REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "$REPO_ROOT" ]]; then
  echo "error: run this script inside a Git repository" >&2
  exit 1
fi
cd "$REPO_ROOT"

if ! gh auth status >/dev/null 2>&1; then
  echo "error: GitHub CLI is not authenticated. Run: gh auth login" >&2
  exit 1
fi

if [[ -n "$(git status --porcelain)" ]]; then
  echo "error: working tree is not clean. Commit, stash, or discard existing changes first." >&2
  git status --short >&2
  exit 1
fi

REPOSITORY="$(gh repo view --json nameWithOwner --jq '.nameWithOwner')"
DEFAULT_BRANCH="$(gh repo view --json defaultBranchRef --jq '.defaultBranchRef.name')"

if [[ -z "$DEFAULT_BRANCH" || "$DEFAULT_BRANCH" == "null" ]]; then
  echo "error: could not determine the repository default branch" >&2
  exit 1
fi

if ! gh issue view "$ISSUE_NUMBER" --repo "$REPOSITORY" >/dev/null 2>&1; then
  echo "error: Issue #$ISSUE_NUMBER does not exist or is not accessible in $REPOSITORY" >&2
  exit 1
fi

SLUG="$(
  python3 - "$DESCRIPTION" <<'PY'
from __future__ import annotations

import re
import sys
import unicodedata

value = unicodedata.normalize("NFKD", sys.argv[1]).encode("ascii", "ignore").decode("ascii")
value = value.lower()
value = re.sub(r"[^a-z0-9]+", "-", value)
value = value.strip("-")
value = re.sub(r"-{2,}", "-", value)
value = value[:48].rstrip("-")

if not value:
    raise SystemExit(
        "error: description could not be converted to an ASCII branch slug; "
        "provide a short English description"
    )

print(value)
PY
)"

BRANCH_NAME="feature/${ISSUE_NUMBER}-${SLUG}"

git fetch origin "$DEFAULT_BRANCH"
git fetch origin "$BRANCH_NAME" 2>/dev/null || true

CURRENT_BRANCH="$(git branch --show-current)"
BRANCH_CHANGED=0

if [[ "$CURRENT_BRANCH" == "$BRANCH_NAME" ]]; then
  :
elif git show-ref --verify --quiet "refs/heads/$BRANCH_NAME"; then
  git switch "$BRANCH_NAME"
  BRANCH_CHANGED=1
elif git show-ref --verify --quiet "refs/remotes/origin/$BRANCH_NAME"; then
  git switch --track -c "$BRANCH_NAME" "origin/$BRANCH_NAME"
  BRANCH_CHANGED=1
else
  git switch -c "$BRANCH_NAME" "origin/$DEFAULT_BRANCH"
  BRANCH_CHANGED=1
fi

# Branch switches accumulate stale target/ build artifacts across feature
# branches; clean them here rather than leaving disk usage to grow unbounded.
if [[ "$BRANCH_CHANGED" -eq 1 ]] && command -v cargo >/dev/null 2>&1; then
  cargo clean
fi

printf '%s\n' "$BRANCH_NAME"
