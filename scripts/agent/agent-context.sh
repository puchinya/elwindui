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

ROOT="$(git rev-parse --show-toplevel 2>/dev/null || true)"
[[ -n "$ROOT" ]] || {
  echo "error: run inside a Git repository" >&2
  exit 1
}
cd "$ROOT"

gh auth status >/dev/null 2>&1 || {
  echo "error: GitHub CLI is not authenticated; run: gh auth login" >&2
  exit 1
}

REPOSITORY="$(gh repo view --json nameWithOwner --jq '.nameWithOwner')"
DEFAULT_BRANCH="$(gh repo view --json defaultBranchRef --jq '.defaultBranchRef.name')"
BRANCH="$(git branch --show-current)"
HEAD_COMMIT="$(git rev-parse HEAD)"
[[ -z "$(git status --porcelain)" ]] && WORKTREE="clean" || WORKTREE="dirty"

ISSUE_JSON="$(gh issue view "$ISSUE_NUMBER" --repo "$REPOSITORY" --json labels,url,closedByPullRequestsReferences)"
PHASE="$(python3 - "$ISSUE_JSON" <<'PY'
import json, sys
obj=json.loads(sys.argv[1])
labels=[x["name"] for x in obj.get("labels",[]) if x.get("name","").startswith("phase:")]
print(labels[0] if labels else "")
PY
)"

case "$PHASE" in
  phase:requirements) WORKFLOW="docs/agent-workflow/requirements.md" ;;
  phase:design) WORKFLOW="docs/agent-workflow/design.md" ;;
  phase:ready|phase:implementation) WORKFLOW="docs/agent-workflow/implementation.md" ;;
  phase:review) WORKFLOW="docs/agent-workflow/review.md" ;;
  *) WORKFLOW="" ;;
esac

PR_DETAILS="$(python3 - "$ISSUE_JSON" "$REPOSITORY" <<'PY'
import json
import subprocess
import sys

issue=json.loads(sys.argv[1])
repository=sys.argv[2]
details=[]
for reference in issue.get("closedByPullRequestsReferences", []):
    number=reference.get("number")
    if not number:
        continue
    try:
        result=subprocess.run(
            [
                "gh",
                "pr",
                "view",
                str(number),
                "--repo",
                repository,
                "--json",
                "number,url,state,updatedAt",
            ],
            check=True,
            capture_output=True,
            text=True,
        )
    except subprocess.CalledProcessError:
        continue
    details.append(json.loads(result.stdout))

print(json.dumps(details))
PY
)"
PR_FIELDS="$(python3 - "$PR_DETAILS" <<'PY'
import json, sys

prs=json.loads(sys.argv[1])
open_prs=[pr for pr in prs if pr.get("state") == "OPEN"]
merged_prs=[pr for pr in prs if pr.get("state") == "MERGED"]
candidates=open_prs or merged_prs
selected=max(candidates, key=lambda pr: pr.get("updatedAt", "")) if candidates else {}
print("{}\t{}".format(selected.get("number", ""), selected.get("url", "")))
PY
)"
IFS=$'\t' read -r PR_NUMBER PR_URL <<< "$PR_FIELDS"

BASE=".agent-state/issues/$ISSUE_NUMBER"
CONTRACT="$BASE/implementation-contract.md"
SHA_FILE="$BASE/implementation-contract.sha256"
CONTRACT_STATUS="absent"
CONTRACT_SHA=""

if [[ -e "$CONTRACT" || -e "$SHA_FILE" ]]; then
  CONTRACT_STATUS="invalid"
  if [[ -f "$CONTRACT" && -f "$SHA_FILE" ]]; then
    ACTUAL_SHA="$(python3 - "$CONTRACT" <<'PY'
from pathlib import Path
import hashlib, sys
print(hashlib.sha256(Path(sys.argv[1]).read_bytes()).hexdigest())
PY
)"
    RECORDED_SHA="$(python3 - "$SHA_FILE" <<'PY'
from pathlib import Path
import re, sys
text=Path(sys.argv[1]).read_text(encoding="utf-8").strip()
m=re.fullmatch(r"([0-9a-f]{64})\s+implementation-contract\.md", text)
print(m.group(1) if m else "")
PY
)"
    CONTRACT_SHA="$ACTUAL_SHA"
    if [[ -n "$RECORDED_SHA" && "$RECORDED_SHA" == "$ACTUAL_SHA" ]]; then
      CONTRACT_STATUS="ok"
    fi
  fi
fi

printf 'repository=%s\n' "$REPOSITORY"
printf 'issue=%s\n' "$ISSUE_NUMBER"
printf 'phase=%s\n' "$PHASE"
printf 'workflow=%s\n' "$WORKFLOW"
printf 'branch=%s\n' "$BRANCH"
printf 'head=%s\n' "$HEAD_COMMIT"
printf 'default_branch=%s\n' "$DEFAULT_BRANCH"
printf 'worktree=%s\n' "$WORKTREE"
printf 'pr_number=%s\n' "$PR_NUMBER"
printf 'pr_url=%s\n' "$PR_URL"
printf 'contract_path=%s\n' "$CONTRACT"
printf 'contract_sha256=%s\n' "$CONTRACT_SHA"
printf 'contract_status=%s\n' "$CONTRACT_STATUS"
