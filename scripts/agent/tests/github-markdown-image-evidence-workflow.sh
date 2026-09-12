#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
AGENTS="$ROOT/AGENTS.md"
REQUIREMENTS="$ROOT/docs/agent-workflow/requirements.md"
IMPLEMENTATION="$ROOT/docs/agent-workflow/implementation.md"
REVIEW="$ROOT/docs/agent-workflow/review.md"
HUMAN="$ROOT/docs_only_human/issue-driven-development-workflow.md"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

contains() {
  local file="$1"
  local text="$2"
  grep -Fq -- "$text" "$file" || fail "$file does not contain: $text"
}

not_contains() {
  local file="$1"
  local text="$2"
  ! grep -Fq -- "$text" "$file" || fail "$file contains forbidden guidance: $text"
}

for file in "$AGENTS" "$REQUIREMENTS" "$IMPLEMENTATION" "$REVIEW" "$HUMAN"; do
  [[ -f "$file" ]] || fail "missing workflow file: $file"
done

# T1: the root invariant covers both safe multiline transport and one-line --body.
contains "$AGENTS" "GitHub Markdown transport invariant:"
contains "$AGENTS" "Multiline Markdown MUST use"
contains "$AGENTS" "--body-file"
contains "$AGENTS" "real newline characters"
contains "$AGENTS" "--body"
contains "$AGENTS" "genuinely single-line text"
contains "$AGENTS" "literal \n"
echo "T1 root newline invariant: PASS"

# T2: phase-specific coverage is explicit.
contains "$REQUIREMENTS" "multiline Issue bodies and checkpoint/comments"
contains "$REQUIREMENTS" "--body-file"
contains "$IMPLEMENTATION" "multiline Issue updates/comments"
contains "$IMPLEMENTATION" "Pull Request bodies"
contains "$REVIEW" "multiline PR comments and review bodies"
echo "T2 phase coverage: PASS"

# T3: no active workflow guidance recommends shell escape decoding.
for file in "$AGENTS" "$REQUIREMENTS" "$IMPLEMENTATION" "$REVIEW" "$HUMAN"; do
  not_contains "$file" "echo -e"
  not_contains "$file" "printf %b"
  not_contains "$file" "eval"
done
echo "T3 no unsafe newline decoding: PASS"

# T4: attachment guidance is explicitly image-only and Issue-owned.
contains "$AGENTS" "Issue image evidence:"
contains "$AGENTS" "owning Issue with"
contains "$AGENTS" "images only"
contains "$IMPLEMENTATION" "screenshots"
contains "$IMPLEMENTATION" "owning Issue with"
contains "$IMPLEMENTATION" "images only"
contains "$REVIEW" "image evidence"
contains "$REVIEW" "owning Issue with"
contains "$REVIEW" "images only"
echo "T4 image-only evidence rule: PASS"

# T5: required image publication has a read-back gate.
contains "$AGENTS" "verified after publication"
contains "$IMPLEMENTATION" "checking the resulting Issue/comment"
contains "$IMPLEMENTATION" "partial upload cannot be reported as PASS"
contains "$REVIEW" "verified after publication"
contains "$REVIEW" "partial uploads cannot be reported as PASS"
echo "T5 required image verification: PASS"

# T6: contracts and logs remain local artifacts, not attachment payloads.
contains "$IMPLEMENTATION" "local Issue-scoped state"
contains "$IMPLEMENTATION" "raw logs"
contains "$REVIEW" "contracts, logs, text, generic files"
contains "$HUMAN" ".agent-state/"
echo "T6 context efficiency: PASS"

# T7: the active guidance keeps history and product scope out of this change.
contains "$IMPLEMENTATION" "historical Issue descriptions or comments"
contains "$IMPLEMENTATION" "Stay inside approved scope."
contains "$REVIEW" "Keep unrelated follow-up work out of the PR."
if git -C "$ROOT" diff --name-only | grep -E '^(crates|examples|docs/(specs|design|status))/' >/dev/null; then
  fail "product or normative documentation path changed"
fi
echo "T7 scope: PASS"

echo "github-markdown-image-evidence-workflow: PASS"
