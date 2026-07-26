#!/usr/bin/env bash
set -euo pipefail

# Ensure that a GitHub Milestone exists for the root Cargo.toml version.
# When an Issue number is supplied, also assign that Issue to the Milestone.
#
# Usage:
#   scripts/agent/ensure-version-milestone.sh
#   scripts/agent/ensure-version-milestone.sh 123
#
# stdout contains only the resolved milestone title, making the command usable
# from another script. Informational messages are written to stderr.

ISSUE_NUMBER="${1:-}"

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "error: required command not found: $1" >&2
    exit 1
  fi
}

require_command git
require_command gh
require_command python3

if [[ -n "$ISSUE_NUMBER" && ! "$ISSUE_NUMBER" =~ ^[0-9]+$ ]]; then
  echo "error: Issue number must be a positive integer: $ISSUE_NUMBER" >&2
  exit 1
fi

REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "$REPO_ROOT" ]]; then
  echo "error: run this script inside a Git repository" >&2
  exit 1
fi
cd "$REPO_ROOT"

if [[ ! -f Cargo.toml ]]; then
  echo "error: Cargo.toml was not found at the Git repository root" >&2
  exit 1
fi

if ! gh auth status >/dev/null 2>&1; then
  echo "error: GitHub CLI is not authenticated. Run: gh auth login" >&2
  exit 1
fi

REPOSITORY="$(gh repo view --json nameWithOwner --jq '.nameWithOwner')"

VERSION="$(
  python3 - "$REPO_ROOT/Cargo.toml" <<'PY'
from __future__ import annotations

import json
import re
import sys
from pathlib import Path

path = Path(sys.argv[1])
raw = path.read_bytes()

version = None

try:
    import tomllib
except ModuleNotFoundError:
    tomllib = None

if tomllib is not None:
    data = tomllib.loads(raw.decode("utf-8"))
    workspace_version = (
        data.get("workspace", {})
        .get("package", {})
        .get("version")
    )
    package_version = data.get("package", {}).get("version")

    if isinstance(workspace_version, str):
        version = workspace_version
    elif isinstance(package_version, str):
        version = package_version

if version is None:
    # Compatibility fallback for Python < 3.11. This intentionally supports
    # the ordinary Cargo forms used here and fails rather than guessing.
    current_section = None
    workspace_version = None
    package_version = None

    def parse_toml_string(value: str) -> str | None:
        value = value.strip()
        double = re.match(r'^("(?:[^"\\]|\\.)*")\s*(?:#.*)?$', value)
        if double:
            return json.loads(double.group(1))
        single = re.match(r"^'([^']*)'\s*(?:#.*)?$", value)
        if single:
            return single.group(1)
        return None

    for raw_line in raw.decode("utf-8").splitlines():
        line = raw_line.strip()
        section = re.match(r"^\[([^\]]+)\]\s*(?:#.*)?$", line)
        if section:
            current_section = section.group(1).strip()
            continue

        assignment = re.match(r"^version\s*=\s*(.+)$", line)
        if not assignment:
            continue

        parsed = parse_toml_string(assignment.group(1))
        if parsed is None:
            continue

        if current_section == "workspace.package":
            workspace_version = parsed
        elif current_section == "package":
            package_version = parsed

    version = workspace_version or package_version

if not isinstance(version, str) or not version.strip():
    raise SystemExit(
        "error: root Cargo.toml has no string [workspace.package].version "
        "or [package].version"
    )

if "\n" in version or "\r" in version:
    raise SystemExit("error: Cargo.toml version contains a newline")

print(version)
PY
)"

MILESTONES_FILE="$(mktemp)"
trap 'rm -f "$MILESTONES_FILE"' EXIT

gh api \
  --paginate \
  --slurp \
  "repos/$REPOSITORY/milestones?state=all&per_page=100" \
  > "$MILESTONES_FILE"

MILESTONE_RESULT="$(
  python3 - "$MILESTONES_FILE" "$VERSION" <<'PY'
import json
import sys
from pathlib import Path

pages = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
title = sys.argv[2]

milestones = []
for page in pages:
    if isinstance(page, list):
        milestones.extend(page)

matches = [item for item in milestones if item.get("title") == title]

if len(matches) > 1:
    numbers = ", ".join(str(item.get("number")) for item in matches)
    raise SystemExit(
        f"error: duplicate GitHub Milestones named {title!r}: {numbers}"
    )

if not matches:
    print("missing")
else:
    item = matches[0]
    print(f"{item['number']}\t{item['state']}")
PY
)"

if [[ "$MILESTONE_RESULT" == "missing" ]]; then
  MILESTONE_NUMBER="$(
    gh api \
      --method POST \
      "repos/$REPOSITORY/milestones" \
      -f "title=$VERSION" \
      --jq '.number'
  )"
  echo "Created GitHub Milestone '$VERSION' (#$MILESTONE_NUMBER)." >&2
else
  IFS=$'\t' read -r MILESTONE_NUMBER MILESTONE_STATE <<< "$MILESTONE_RESULT"

  if [[ "$MILESTONE_STATE" != "open" ]]; then
    echo "error: GitHub Milestone '$VERSION' (#$MILESTONE_NUMBER) exists but is closed" >&2
    echo "       Do not create a duplicate. Reopen it or update Cargo.toml after deciding the intended release." >&2
    exit 1
  fi

  echo "Using GitHub Milestone '$VERSION' (#$MILESTONE_NUMBER)." >&2
fi

if [[ -n "$ISSUE_NUMBER" ]]; then
  gh issue edit \
    "$ISSUE_NUMBER" \
    --repo "$REPOSITORY" \
    --milestone "$VERSION" \
    >/dev/null
  echo "Assigned Issue #$ISSUE_NUMBER to Milestone '$VERSION'." >&2
fi

printf '%s\n' "$VERSION"
