#!/usr/bin/env bash
set -euo pipefail

ISSUE_NUMBER="${1:-}"
INPUT_PATH="${2:-}"

for name in git gh python3; do
  command -v "$name" >/dev/null 2>&1 || {
    echo "error: required command not found: $name" >&2
    exit 1
  }
done

[[ "$ISSUE_NUMBER" =~ ^[1-9][0-9]*$ ]] || {
  echo "usage: $0 <issue-number> [contract-file]" >&2
  echo "       omit contract-file to read UTF-8 contract text from stdin" >&2
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
gh issue view "$ISSUE_NUMBER" --repo "$REPOSITORY" >/dev/null

"$(dirname "$0")/prepare-work-evidence.sh" "$ISSUE_NUMBER" >/dev/null

BASE=".agent-state/issues/$ISSUE_NUMBER"
DEST="$BASE/implementation-contract.md"
SHA_FILE="$BASE/implementation-contract.sha256"

TMP="$(mktemp)"
trap 'rm -f "$TMP"' EXIT

if [[ -n "$INPUT_PATH" ]]; then
  [[ -f "$INPUT_PATH" ]] || {
    echo "error: contract file not found: $INPUT_PATH" >&2
    exit 1
  }
  cat "$INPUT_PATH" > "$TMP"
else
  cat > "$TMP"
fi

python3 - "$TMP" <<'PY'
from pathlib import Path
import sys
data = Path(sys.argv[1]).read_bytes()
try:
    data.decode("utf-8")
except UnicodeDecodeError as exc:
    raise SystemExit(f"error: contract input must be UTF-8: {exc}")
if not data:
    raise SystemExit("error: contract input is empty")
PY

NEW_SHA="$(python3 - "$TMP" <<'PY'
from pathlib import Path
import hashlib, sys
print(hashlib.sha256(Path(sys.argv[1]).read_bytes()).hexdigest())
PY
)"

if [[ -f "$DEST" ]]; then
  EXISTING_SHA="$(python3 - "$DEST" <<'PY'
from pathlib import Path
import hashlib, sys
print(hashlib.sha256(Path(sys.argv[1]).read_bytes()).hexdigest())
PY
)"
  if [[ "$EXISTING_SHA" != "$NEW_SHA" ]]; then
    echo "error: immutable contract mirror already exists with different content: $DEST" >&2
    echo "       existing sha256: $EXISTING_SHA" >&2
    echo "       supplied sha256: $NEW_SHA" >&2
    exit 1
  fi
else
  cp "$TMP" "$DEST"
fi

printf '%s  %s\n' "$NEW_SHA" "implementation-contract.md" > "$SHA_FILE"

printf 'contract_path=%s\n' "$DEST"
printf 'contract_sha256=%s\n' "$NEW_SHA"
