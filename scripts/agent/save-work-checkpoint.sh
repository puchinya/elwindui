#!/usr/bin/env bash
set -euo pipefail

ISSUE_NUMBER="${1:-}"

for name in git gh python3; do
  command -v "$name" >/dev/null 2>&1 || {
    echo "error: required command not found: $name" >&2
    exit 1
  }
done

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

gh auth status >/dev/null 2>&1 || {
  echo "error: run: gh auth login" >&2
  exit 1
}

REPOSITORY="$(gh repo view --json nameWithOwner --jq '.nameWithOwner')"
gh issue view "$ISSUE_NUMBER" --repo "$REPOSITORY" >/dev/null

BRANCH="$(git branch --show-current)"
HEAD_COMMIT="$(git rev-parse HEAD)"
BASE_BRANCH="$(gh repo view --json defaultBranchRef --jq '.defaultBranchRef.name')"
STATUS="$(git status --porcelain)"
[[ -z "$STATUS" ]] && WORKING_TREE="clean" || WORKING_TREE="dirty"

PHASE="$(
  gh issue view "$ISSUE_NUMBER" \
    --repo "$REPOSITORY" \
    --json labels \
    --jq '[.labels[].name | select(startswith("phase:"))][0] // ""'
)"

PULL_REQUEST="$(
  gh pr list \
    --repo "$REPOSITORY" \
    --head "$BRANCH" \
    --state all \
    --limit 1 \
    --json number \
    --jq '.[0].number // empty'
)"

case "$(uname -s)" in
  Darwin) PLATFORM="macos" ;;
  Linux) PLATFORM="linux" ;;
  *) PLATFORM="unix" ;;
esac

"$(dirname "$0")/prepare-work-evidence.sh" "$ISSUE_NUMBER" >/dev/null

CHECKPOINT_FILE=".agent-state/issues/$ISSUE_NUMBER/checkpoint.md"

python3 - \
  "$CHECKPOINT_FILE" \
  "$REPOSITORY" \
  "$ISSUE_NUMBER" \
  "$PHASE" \
  "$BRANCH" \
  "$BASE_BRANCH" \
  "$HEAD_COMMIT" \
  "$PLATFORM" \
  "$WORKING_TREE" \
  "$PULL_REQUEST" \
  "$STATUS" <<'PY'
from __future__ import annotations

import datetime as dt
import json
import sys
from pathlib import Path

(
    checkpoint_path,
    repository,
    issue,
    phase,
    branch,
    base_branch,
    head_commit,
    platform,
    working_tree,
    pull_request,
    git_status,
) = sys.argv[1:]

path = Path(checkpoint_path)
timestamp = dt.datetime.now().astimezone().isoformat(timespec="seconds")

metadata = {
    "schema": 1,
    "repository": repository,
    "issue": int(issue),
    "phase": phase,
    "branch": branch,
    "base_branch": base_branch,
    "head_commit": head_commit,
    "updated_at": timestamp,
    "platform": platform,
    "working_tree": working_tree,
    "pull_request": pull_request,
}

frontmatter = ["---"]
for key, value in metadata.items():
    if isinstance(value, int):
        frontmatter.append(f"{key}: {value}")
    else:
        frontmatter.append(f"{key}: {json.dumps(value, ensure_ascii=False)}")
frontmatter.append("---")

default_body = """# Objective

- TODO

# Completed

- TODO

# Current state

- TODO

# Next action

- TODO: name the next file, symbol, test, investigation, or command.

# Verification

## Passed

- None

## Failed

- None

## Not run

- None

# Uncommitted work

- None

# Blockers

- None
"""

body = default_body
if path.exists():
    current = path.read_text(encoding="utf-8")
    if current.startswith("---\n"):
        closing = current.find("\n---\n", 4)
        if closing >= 0:
            body = current[closing + 5 :]
    else:
        body = current

if working_tree == "dirty":
    status_lines = [line for line in git_status.splitlines() if line.strip()]
    status_block = "\n".join(f"- `{line}`" for line in status_lines) or "- Unknown"
    marker = "# Uncommitted work\n"
    if marker in body:
        before, rest = body.split(marker, 1)
        next_heading = rest.find("\n# ")
        tail = rest[next_heading:] if next_heading >= 0 else "\n"
        body = before + marker + "\n" + status_block + "\n" + tail

content = "\n".join(frontmatter) + "\n\n" + body.lstrip()
path.write_text(content, encoding="utf-8")
PY

printf '%s\n' "$CHECKPOINT_FILE"
