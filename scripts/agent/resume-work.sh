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

CHECKPOINT_FILE=".agent-state/issues/$ISSUE_NUMBER/checkpoint.md"
[[ -f "$CHECKPOINT_FILE" ]] || {
  echo "error: checkpoint not found: $CHECKPOINT_FILE" >&2
  exit 1
}

REPOSITORY="$(gh repo view --json nameWithOwner --jq '.nameWithOwner')"
BRANCH="$(git branch --show-current)"
HEAD_COMMIT="$(git rev-parse HEAD)"
[[ -z "$(git status --porcelain)" ]] && WORKING_TREE="clean" || WORKING_TREE="dirty"

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

python3 - \
  "$CHECKPOINT_FILE" \
  "$REPOSITORY" \
  "$ISSUE_NUMBER" \
  "$PHASE" \
  "$BRANCH" \
  "$HEAD_COMMIT" \
  "$WORKING_TREE" \
  "$PULL_REQUEST" <<'PY'
from __future__ import annotations

import json
import sys
from pathlib import Path

(
    checkpoint_path,
    repository,
    issue,
    phase,
    branch,
    head_commit,
    working_tree,
    pull_request,
) = sys.argv[1:]

path = Path(checkpoint_path)
text = path.read_text(encoding="utf-8")
metadata = {}

if text.startswith("---\n"):
    closing = text.find("\n---\n", 4)
    if closing >= 0:
        for line in text[4:closing].splitlines():
            if ":" not in line:
                continue
            key, raw = line.split(":", 1)
            raw = raw.strip()
            try:
                value = json.loads(raw)
            except json.JSONDecodeError:
                value = raw
            metadata[key.strip()] = value

current = {
    "repository": repository,
    "issue": int(issue),
    "phase": phase,
    "branch": branch,
    "head_commit": head_commit,
    "working_tree": working_tree,
    "pull_request": pull_request,
}

differences = []
for key, value in current.items():
    if metadata.get(key) != value:
        differences.append(
            f"- {key}: checkpoint={metadata.get(key)!r}, current={value!r}"
        )

print(text.rstrip())
print("\n---")
if differences:
    print("Checkpoint status: STALE OR CHANGED")
    print("\n".join(differences))
    print("Refresh the checkpoint from current Git/GitHub state before editing.")
else:
    print("Checkpoint status: CURRENT")
PY
