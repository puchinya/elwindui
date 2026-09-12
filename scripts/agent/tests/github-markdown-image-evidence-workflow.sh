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

FORBIDDEN_PATH_RE='^(crates|examples|docs/(specs|design|status))/'

committed_scope_check() {
  local repo_root="$1"
  local base_ref="${2:-${ELWINDUI_TEST_BASE_REF:-origin/master}}"
  local base
  local paths

  if ! base="$(git -C "$repo_root" merge-base "$base_ref" HEAD)"; then
    return 2
  fi
  if ! paths="$(git -C "$repo_root" diff --name-only "$base"...HEAD)"; then
    return 2
  fi
  if [[ -n "$paths" ]] && printf '%s\n' "$paths" | grep -E -q "$FORBIDDEN_PATH_RE"; then
    return 1
  fi
  return 0
}

run_committed_scope_regression() {
  local fixture
  local repo
  local base
  local scope_status

  fixture="$(mktemp -d)"
  repo="$fixture/repo"
  mkdir -p "$repo"
  git -C "$repo" init -q
  git -C "$repo" config user.email test@example.invalid
  git -C "$repo" config user.name github-markdown-image-evidence-test
  printf '%s\n' baseline > "$repo/README.md"
  git -C "$repo" add README.md
  git -C "$repo" commit -qm baseline
  base="$(git -C "$repo" rev-parse HEAD)"
  git -C "$repo" update-ref refs/remotes/origin/master "$base"
  git -C "$repo" checkout -q -b fixture

  mkdir -p "$repo/docs/agent-workflow"
  printf '%s\n' allowed > "$repo/docs/agent-workflow/fixture.md"
  git -C "$repo" add docs/agent-workflow/fixture.md
  git -C "$repo" commit -qm allowed
  if ! committed_scope_check "$repo" origin/master; then
    rm -rf "$fixture"
    fail "allowed committed workflow path was rejected"
  fi
  [[ -z "$(git -C "$repo" status --short)" ]] || {
    rm -rf "$fixture"
    fail "allowed committed fixture is not clean"
  }
  echo "R1 committed allowed scope: PASS"

  mkdir -p "$repo/docs/design"
  printf '%s\n' forbidden > "$repo/docs/design/forbidden.md"
  git -C "$repo" add docs/design/forbidden.md
  git -C "$repo" commit -qm forbidden
  [[ -z "$(git -C "$repo" status --short)" ]] || {
    rm -rf "$fixture"
    fail "forbidden committed fixture is not clean"
  }
    if committed_scope_check "$repo" origin/master; then
      rm -rf "$fixture"
      fail "forbidden committed path was not rejected"
    else
      scope_status=$?
      if [[ "$scope_status" -ne 1 ]]; then
        rm -rf "$fixture"
        fail "forbidden fixture scope check failed unexpectedly with status $scope_status"
      fi
    fi
  echo "R2 committed forbidden clean-worktree scope: PASS"

  rm -rf "$fixture"
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
if ! committed_scope_check "$ROOT"; then
  fail "product or normative documentation path changed"
fi
run_committed_scope_regression
echo "T7 scope: PASS"

echo "github-markdown-image-evidence-workflow: PASS"
