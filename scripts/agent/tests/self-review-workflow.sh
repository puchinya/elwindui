#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
TMP="$(mktemp -d)"
STUB_BIN="$(mktemp -d)"
trap 'rm -rf "$TMP" "$STUB_BIN"' EXIT
export SELF_REVIEW_TEST_ROOT="$TMP"

mkdir -p "$TMP/scripts/agent" "$TMP/.agent-state"
cp "$ROOT/scripts/agent/prepare-self-review.sh" "$TMP/scripts/agent/"
cp "$ROOT/scripts/agent/validate-self-review.sh" "$TMP/scripts/agent/"
cp "$ROOT/scripts/agent/prepare-self-review.ps1" "$TMP/scripts/agent/"
cp "$ROOT/scripts/agent/validate-self-review.ps1" "$TMP/scripts/agent/"
cp "$ROOT/scripts/agent/prepare-work-evidence.sh" "$TMP/scripts/agent/"
cp "$ROOT/scripts/agent/prepare-work-evidence.ps1" "$TMP/scripts/agent/"
cat > "$STUB_BIN/gh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
case "$1 $2" in
  "auth status") exit 0 ;;
  "repo view") printf '%s\n' 'test/repository' ;;
  "issue view") cat "$SELF_REVIEW_TEST_ROOT/.agent-state/issue.json" ;;
  *) echo "unexpected gh invocation: $*" >&2; exit 1 ;;
esac
EOF
chmod +x "$STUB_BIN/gh" "$TMP/scripts/agent"/*.sh
export PATH="$STUB_BIN:$PATH"

cd "$TMP"
git init -q
git config user.email test@example.invalid
git config user.name self-review-test
printf '%s\n' baseline > tracked.txt
git add tracked.txt scripts
git commit -qm baseline

set_issue_body() {
  python3 - "$TMP/.agent-state/issue.json" "$1" <<'PY'
import json
import sys
from pathlib import Path
Path(sys.argv[1]).write_text(json.dumps({"number": 123, "body": sys.argv[2]}), encoding="utf-8")
PY
}

write_contract() {
  printf '%s\n' "$1" > "$TMP/.agent-state/issues/123/implementation-contract.md"
  python3 - .agent-state/issues/123/implementation-contract.md .agent-state/issues/123/implementation-contract.sha256 <<'PY'
import hashlib
import sys
from pathlib import Path

path = Path(sys.argv[1])
Path(sys.argv[2]).write_text(
    f"{hashlib.sha256(path.read_bytes()).hexdigest()}  implementation-contract.md\n",
    encoding="utf-8",
)
PY
}

reset_state() {
  rm -rf "$TMP/.agent-state/issues/123"
  "$TMP/scripts/agent/prepare-work-evidence.sh" 123 >/dev/null
}

fresh_review() {
  rm -f "$TMP/.agent-state/issues/123/reviewer-checklist.md"
  rm -f "$TMP/.agent-state/issues/123/reviewer-checklist.sha256"
  rm -f "$TMP/.agent-state/issues/123/self-review.md"
  "$TMP/scripts/agent/prepare-self-review.sh" 123 >/dev/null
}

expect_fail() {
  local output
  if output=$("$@" 2>&1); then
    echo "expected failure but succeeded: $*" >&2
    echo "$output" >&2
    exit 1
  fi
}

assert_contains() {
  [[ "$1" == *"$2"* ]] || {
    echo "missing '$2' in: $1" >&2
    exit 1
  }
}

failure_class() {
  sed -n 's/.*error\[\([^]]*\)\].*/\1/p' <<<"$1" | tail -n 1
}

expect_parity_prepare_failure() {
  local expected="$1"
  local posix_output ps_output
  if posix_output=$("$TMP/scripts/agent/prepare-self-review.sh" 123 2>&1); then
    echo "expected POSIX prepare failure [$expected]" >&2
    exit 1
  fi
  if ps_output=$(pwsh -NoProfile -File "$TMP/scripts/agent/prepare-self-review.ps1" 123 2>&1); then
    echo "expected PowerShell prepare failure [$expected]" >&2
    exit 1
  fi
  [[ "$(failure_class "$posix_output")" == "$expected" ]]
  [[ "$(failure_class "$ps_output")" == "$expected" ]]
}

expect_parity_validate_failure() {
  local expected="$1"
  local posix_output ps_output
  if posix_output=$("$TMP/scripts/agent/validate-self-review.sh" 123 2>&1); then
    echo "expected POSIX validation failure [$expected]" >&2
    exit 1
  fi
  if ps_output=$(pwsh -NoProfile -File "$TMP/scripts/agent/validate-self-review.ps1" 123 2>&1); then
    echo "expected PowerShell validation failure [$expected]" >&2
    exit 1
  fi
  [[ "$(failure_class "$posix_output")" == "$expected" ]]
  [[ "$(failure_class "$ps_output")" == "$expected" ]]
}

fill_pass_review() {
  python3 - "$TMP/.agent-state/issues/123/self-review.md" "$(git rev-parse HEAD)" <<'PY'
import re
import sys
from pathlib import Path
path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
text = re.sub(r"^Reviewed-HEAD:.*$", f"Reviewed-HEAD: {sys.argv[2]}", text, flags=re.M)
text = re.sub(r"\| PENDING \|", "| PASS | Evidence: test:fixture", text)
path.write_text(text, encoding="utf-8")
PY
}

mutate_review() {
  python3 - "$TMP/.agent-state/issues/123/self-review.md" "$1" <<'PY'
import sys
from pathlib import Path
path = Path(sys.argv[1])
mode = sys.argv[2]
lines = path.read_text(encoding="utf-8").splitlines()
if mode == "pending":
    lines = [line.replace("| PASS | Evidence: test:fixture", "| PENDING |") if line.startswith("- ") else line for line in lines]
elif mode == "fail":
    for index, line in enumerate(lines):
        if line.startswith("- ") and "| PASS | Evidence:" in line:
            lines[index] = line.replace("| PASS | Evidence: test:fixture", "| FAIL | Evidence: fixture failure", 1)
            break
elif mode == "no-evidence":
    for index, line in enumerate(lines):
        if line.startswith("- ") and "| PASS | Evidence:" in line:
            lines[index] = line.replace("| PASS | Evidence: test:fixture", "| PASS |", 1)
            break
elif mode == "prose-evidence":
    for index, line in enumerate(lines):
        if line.startswith("- ") and "| PASS | Evidence:" in line:
            lines[index] = line.replace("| PASS | Evidence: test:fixture", "| PASS | Evidence: reviewed", 1)
            break
elif mode == "no-reason":
    for index, line in enumerate(lines):
        if line.startswith("- ") and "| PASS | Evidence:" in line:
            lines[index] = line.replace("| PASS | Evidence: test:fixture", "| N/A |", 1)
            break
elif mode == "unknown":
    lines.append("- X999 | PASS | Evidence: test:unknown")
elif mode == "duplicate":
    lines.append(next(line for line in lines if line.startswith("- C001 ")))
elif mode == "missing":
    lines = [line for line in lines if not line.startswith("- C001 ")]
path.write_text("\n".join(lines) + "\n", encoding="utf-8")
PY
}

direct_body=$'## Reviewer Checklist\n\n- [ ] resize preserves adjacent column minimums\n- [x] repeated drag cancellation releases capture\n\n## Notes\nNot a checklist item.'
set_issue_body "$direct_body"
reset_state
out="$("$TMP/scripts/agent/prepare-self-review.sh" 123)"
assert_contains "$out" 'checklist_changed=1'
assert_contains "$out" 'items=2'
grep -q -- '- I001 | issue | resize preserves adjacent column minimums' .agent-state/issues/123/reviewer-checklist.md
echo 'T1 direct extraction: PASS'

python3 - .agent-state/issues/123/self-review.md <<'PY'
from pathlib import Path
p = Path(__import__("sys").argv[1])
p.write_text(p.read_text().replace("| PENDING |", "| PASS | Evidence: test:partial"), encoding="utf-8")
PY
out="$("$TMP/scripts/agent/prepare-self-review.sh" 123)"
assert_contains "$out" 'checklist_changed=0'
grep -q -- 'I001 | PASS | Evidence: test:partial' .agent-state/issues/123/self-review.md
echo 'T8 idempotent preparation: PASS'

set_issue_body $'## Reviewer Checklist\n\n- [ ] resize preserves adjacent column minimums\n- [ ] repeated drag cancellation releases capture\n- [ ] keyboard resize preserves focus'
out="$("$TMP/scripts/agent/prepare-self-review.sh" 123)"
assert_contains "$out" 'checklist_changed=1'
grep -q -- '- I003 | PENDING |' .agent-state/issues/123/self-review.md
! grep -q -- 'I001 | PASS | Evidence: test:partial' .agent-state/issues/123/self-review.md
echo 'T9 source-change invalidation: PASS'

set_issue_body $'## Purpose\nNo checklist.'
reset_state
expect_fail "$TMP/scripts/agent/prepare-self-review.sh" 123
echo 'T5 missing checklist: PASS'

set_issue_body $'## Reviewer Checklist\n\nNo checkbox here.'
reset_state
expect_fail "$TMP/scripts/agent/prepare-self-review.sh" 123
echo 'T6 empty checklist: PASS'

set_issue_body $'## Reviewer Checklist\n\n- [ ] same obligation\n- [x] same   obligation'
reset_state
expect_fail "$TMP/scripts/agent/prepare-self-review.sh" 123
echo 'T7 duplicate checklist: PASS'

set_issue_body $'## Reviewer Checklist\n\n- [ ] Foo Bar\n- [ ] foo   bar'
reset_state
out=$("$TMP/scripts/agent/prepare-self-review.sh" 123 2>&1 || true)
assert_contains "$out" 'error[duplicate-checklist-item]'
echo 'T3 ASCII duplicate normalization: PASS'

set_issue_body $'## Reviewer Checklist\n\n- [ ] straße\n- [ ] STRASSE'
reset_state
out=$("$TMP/scripts/agent/prepare-self-review.sh" 123)
assert_contains "$out" 'items=2'
posix_non_ascii_sha="$(sed -n 's/^review_checklist_sha256=//p' <<<"$out")"
grep -q -- '- I001 | issue | straße' .agent-state/issues/123/reviewer-checklist.md
grep -q -- '- I002 | issue | STRASSE' .agent-state/issues/123/reviewer-checklist.md
if command -v pwsh >/dev/null 2>&1; then
  ps_non_ascii_sha="$(pwsh -NoProfile -File "$TMP/scripts/agent/prepare-self-review.ps1" 123 | sed -n 's/^review_checklist_sha256=//p')"
  [[ "$ps_non_ascii_sha" == "$posix_non_ascii_sha" ]]
fi
echo 'T4 non-ASCII deterministic distinction: PASS'

set_issue_body $'# 40. Reviewer Checklist\n\n- [ ] alternate heading works\n\n# Boundary\n- [ ] outside section is ignored'
reset_state
out="$("$TMP/scripts/agent/prepare-self-review.sh" 123)"
assert_contains "$out" 'items=1'
grep -q -- '- I001 | issue | alternate heading works' .agent-state/issues/123/reviewer-checklist.md
echo 'T3 alternate heading: PASS'

reset_state
contract_body=$'## 9. Reviewer Checklist\n\n- [ ] contract obligation one\n- [x] contract obligation two\n\n## End'
printf '%s\n' "$contract_body" > .agent-state/issues/123/implementation-contract.md
python3 - .agent-state/issues/123/implementation-contract.md .agent-state/issues/123/implementation-contract.sha256 <<'PY'
import hashlib
import sys
from pathlib import Path
p = Path(sys.argv[1])
Path(sys.argv[2]).write_text(f"{hashlib.sha256(p.read_bytes()).hexdigest()}  implementation-contract.md\n", encoding="utf-8")
PY
set_issue_body $'## Purpose\nContract supplies the checklist.'
out="$("$TMP/scripts/agent/prepare-self-review.sh" 123)"
assert_contains "$out" 'items=2'
grep -q -- '- C001 | contract | contract obligation one' .agent-state/issues/123/reviewer-checklist.md
echo 'T2 contract extraction: PASS'

canonical_contract=$'transport heading\nELWINDUI_REVIEWER_CHECKLIST_V1_BEGIN\n\nREVIEW_ITEM: canonical obligation one\nREVIEW_ITEM: canonical obligation two\n\nELWINDUI_REVIEWER_CHECKLIST_V1_END\n'
set_issue_body $'## Purpose\nCanonical contract supplies the checklist.'
reset_state
write_contract "$canonical_contract"
out="$($TMP/scripts/agent/prepare-self-review.sh 123)"
assert_contains "$out" 'items=2'
canonical_sha="$(sed -n 's/^review_checklist_sha256=//p' <<<"$out")"
grep -q -- '- C001 | contract | canonical obligation one' .agent-state/issues/123/reviewer-checklist.md
grep -q -- '- C002 | contract | canonical obligation two' .agent-state/issues/123/reviewer-checklist.md
echo 'T22 raw canonical contract: PASS'

fenced_canonical=$'```\nELWINDUI_REVIEWER_CHECKLIST_V1_BEGIN\nREVIEW_ITEM: canonical obligation one\nREVIEW_ITEM: canonical obligation two\nELWINDUI_REVIEWER_CHECKLIST_V1_END\n```\n'
write_contract "$fenced_canonical"
out="$($TMP/scripts/agent/prepare-self-review.sh 123)"
assert_contains "$out" 'items=2'
[[ "$(sed -n 's/^review_checklist_sha256=//p' <<<"$out")" == "$canonical_sha" ]]
echo 'T23 fenced canonical contract: PASS'

rendered_copy=$'10. Reviewer Checklist\n☐ rendered presentation item\n\nELWINDUI_REVIEWER_CHECKLIST_V1_BEGIN\nREVIEW_ITEM: canonical obligation one\nREVIEW_ITEM: canonical obligation two\nELWINDUI_REVIEWER_CHECKLIST_V1_END\n'
write_contract "$rendered_copy"
out="$($TMP/scripts/agent/prepare-self-review.sh 123)"
assert_contains "$out" 'items=2'
[[ "$(sed -n 's/^review_checklist_sha256=//p' <<<"$out")" == "$canonical_sha" ]]
! grep -q -- 'rendered presentation item' .agent-state/issues/123/reviewer-checklist.md
echo 'T24 rendered-copy simulation: PASS'

if command -v pwsh >/dev/null 2>&1; then
  ps_out="$(pwsh -NoProfile -File "$TMP/scripts/agent/prepare-self-review.ps1" 123)"
  [[ "$(sed -n 's/^review_checklist_sha256=//p' <<<"$ps_out")" == "$canonical_sha" ]]
  grep -q -- '- C001 | contract | canonical obligation one' .agent-state/issues/123/reviewer-checklist.md
  grep -q -- '- C002 | contract | canonical obligation two' .agent-state/issues/123/reviewer-checklist.md
  echo 'T32 canonical POSIX/PowerShell success parity: PASS'
else
  echo 'T32 canonical POSIX/PowerShell success parity: NOT RUN (pwsh unavailable)'
fi

precedence_contract=$'# Reviewer Checklist\n\n- [ ] legacy Markdown obligation\n\nELWINDUI_REVIEWER_CHECKLIST_V1_BEGIN\nREVIEW_ITEM: canonical obligation one\nREVIEW_ITEM: canonical obligation two\nELWINDUI_REVIEWER_CHECKLIST_V1_END\n'
write_contract "$precedence_contract"
out="$($TMP/scripts/agent/prepare-self-review.sh 123)"
assert_contains "$out" 'items=2'
! grep -q -- 'legacy Markdown obligation' .agent-state/issues/123/reviewer-checklist.md
echo 'T25 canonical precedence: PASS'

expect_canonical_failure() {
  local expected="$1"
  local body="$2"
  set_issue_body $'## Purpose\nCanonical failure fixture.'
  reset_state
  write_contract "$body"
  expect_parity_prepare_failure "$expected"
}

expect_canonical_failure canonical-checklist-malformed $'ELWINDUI_REVIEWER_CHECKLIST_V1_BEGIN\nREVIEW_ITEM: missing end'
echo 'T26 canonical begin without end: PASS'
expect_canonical_failure canonical-checklist-multiple $'ELWINDUI_REVIEWER_CHECKLIST_V1_BEGIN\nREVIEW_ITEM: one\nELWINDUI_REVIEWER_CHECKLIST_V1_END\nELWINDUI_REVIEWER_CHECKLIST_V1_BEGIN\nREVIEW_ITEM: two\nELWINDUI_REVIEWER_CHECKLIST_V1_END'
echo 'T27 multiple canonical blocks: PASS'
expect_canonical_failure canonical-checklist-empty $'ELWINDUI_REVIEWER_CHECKLIST_V1_BEGIN\n\nELWINDUI_REVIEWER_CHECKLIST_V1_END'
echo 'T28 empty canonical block: PASS'
expect_canonical_failure canonical-checklist-empty $'ELWINDUI_REVIEWER_CHECKLIST_V1_BEGIN\nREVIEW_ITEM:\nELWINDUI_REVIEWER_CHECKLIST_V1_END'
echo 'T29 empty canonical item: PASS'
expect_canonical_failure canonical-checklist-malformed $'ELWINDUI_REVIEWER_CHECKLIST_V1_BEGIN\nunexpected line\nELWINDUI_REVIEWER_CHECKLIST_V1_END'
echo 'T30 unexpected canonical line: PASS'

set_issue_body $'## Purpose\nContract supplies the checklist.'
reset_state
write_contract "$contract_body"
out="$($TMP/scripts/agent/prepare-self-review.sh 123)"
assert_contains "$out" 'items=2'
echo 'T31 legacy Markdown-only compatibility: PASS'

set_issue_body $'## Reviewer Checklist\n\n- [ ] Issue supplemental one\n- [ ] Issue supplemental two'
out="$("$TMP/scripts/agent/prepare-self-review.sh" 123)"
assert_contains "$out" 'items=4'
grep -q -- '- C002 | contract | contract obligation two' .agent-state/issues/123/reviewer-checklist.md
grep -q -- '- I001 | issue | Issue supplemental one' .agent-state/issues/123/reviewer-checklist.md
echo 'T4 contract plus Issue supplemental: PASS'

set_issue_body $'## Reviewer Checklist\n\n- [ ] contract obligation one'
expect_fail "$TMP/scripts/agent/prepare-self-review.sh" 123
echo 'T20 duplicate across sources: PASS'

printf '%s\n' changed >> .agent-state/issues/123/implementation-contract.md
expect_fail "$TMP/scripts/agent/prepare-self-review.sh" 123
echo 'T20 invalid contract integrity: PASS'

printf '%s\n' "$contract_body" > .agent-state/issues/123/implementation-contract.md
python3 - .agent-state/issues/123/implementation-contract.md .agent-state/issues/123/implementation-contract.sha256 <<'PY'
import hashlib
import sys
from pathlib import Path
p = Path(sys.argv[1])
Path(sys.argv[2]).write_text(f"{hashlib.sha256(p.read_bytes()).hexdigest()}  implementation-contract.md\n", encoding="utf-8")
PY
set_issue_body $'## Reviewer Checklist\n\n- [ ] Issue supplemental one\n- [ ] Issue supplemental two'
"$TMP/scripts/agent/prepare-self-review.sh" 123 >/dev/null
fill_pass_review
out="$("$TMP/scripts/agent/validate-self-review.sh" 123)"
assert_contains "$out" 'self_review_status=pass'
assert_contains "$out" 'pass=4'
echo 'T10 complete PASS validation: PASS'

for token in \
  'path:scripts/agent/validate-self-review.sh' \
  'symbol:scripts/agent/validate-self-review.sh::evidence-parser' \
  'test:T6' \
  'cmd:bash scripts/agent/tests/self-review-workflow.sh' \
  'artifact:.agent-state/issues/123/self-review.md' \
  'issue:#243' \
  'pr:#244'; do
  fresh_review
  fill_pass_review
  python3 - .agent-state/issues/123/self-review.md "$token" <<'PY'
from pathlib import Path
import sys
p = Path(sys.argv[1])
token = sys.argv[2]
p.write_text(p.read_text().replace("Evidence: test:fixture", f"Evidence: {token}"), encoding="utf-8")
PY
  out=$("$TMP/scripts/agent/validate-self-review.sh" 123)
  assert_contains "$out" 'self_review_status=pass'
done
echo 'T6 structured evidence prefixes: PASS'

fresh_review
fill_pass_review
python3 - .agent-state/issues/123/self-review.md <<'PY'
from pathlib import Path
p = Path(__import__("sys").argv[1])
p.write_text(p.read_text().replace("| PASS | Evidence: test:fixture", "| N/A | Reason: this fixture does not exercise the platform-only path", 1), encoding="utf-8")
PY
out="$("$TMP/scripts/agent/validate-self-review.sh" 123)"
assert_contains "$out" 'pass=3'
assert_contains "$out" 'na=1'
echo 'T10 valid N/A validation: PASS'

for mode in pending fail no-evidence prose-evidence no-reason unknown duplicate missing; do
  fresh_review
  fill_pass_review
  mutate_review "$mode"
  expect_fail "$TMP/scripts/agent/validate-self-review.sh" 123
  case "$mode" in
    pending) echo 'T11 PENDING rejection: PASS' ;;
    fail) echo 'T12 FAIL rejection: PASS' ;;
    no-evidence) echo 'T13 missing PASS evidence: PASS' ;;
    prose-evidence) echo 'T5 prose-only evidence rejection: PASS' ;;
    no-reason) echo 'T14 missing N/A reason: PASS' ;;
    unknown) echo 'T16 unknown ID: PASS' ;;
    duplicate) echo 'T16 duplicate result ID: PASS' ;;
    missing) echo 'T15 missing result ID: PASS' ;;
  esac
done

printf '%s\n' changed >> .agent-state/issues/123/implementation-contract.md
expect_fail "$TMP/scripts/agent/validate-self-review.sh" 123
echo 'T20 validator contract integrity: PASS'
printf '%s\n' "$contract_body" > .agent-state/issues/123/implementation-contract.md
python3 - .agent-state/issues/123/implementation-contract.md .agent-state/issues/123/implementation-contract.sha256 <<'PY'
import hashlib
import sys
from pathlib import Path
p = Path(sys.argv[1])
Path(sys.argv[2]).write_text(f"{hashlib.sha256(p.read_bytes()).hexdigest()}  implementation-contract.md\n", encoding="utf-8")
PY
fresh_review
fill_pass_review

set_issue_body $'## Reviewer Checklist\n\n- [ ] changed source obligation'
expect_fail "$TMP/scripts/agent/validate-self-review.sh" 123
echo 'T19 stale source rejection: PASS'
set_issue_body $'## Reviewer Checklist\n\n- [ ] Issue supplemental one\n- [ ] Issue supplemental two'
fresh_review
fill_pass_review
printf '%s\n' dirty >> tracked.txt
expect_fail "$TMP/scripts/agent/validate-self-review.sh" 123
git checkout -- tracked.txt
echo 'T18 dirty worktree rejection: PASS'

git commit --allow-empty -qm remediation
expect_fail "$TMP/scripts/agent/validate-self-review.sh" 123
echo 'T17 stale HEAD rejection: PASS'
"$TMP/scripts/agent/prepare-self-review.sh" 123 >/dev/null
fill_pass_review

if command -v pwsh >/dev/null 2>&1; then
  rm -f .agent-state/issues/123/implementation-contract.md .agent-state/issues/123/implementation-contract.sha256
  set_issue_body $'## Purpose\nNo checklist.'
  reset_state
  expect_parity_prepare_failure missing-checklist

  set_issue_body $'## Reviewer Checklist\n\nNo checkbox here.'
  reset_state
  expect_parity_prepare_failure empty-checklist

  set_issue_body $'## Reviewer Checklist\n\n- [ ] Foo Bar\n- [ ] foo   bar'
  reset_state
  expect_parity_prepare_failure duplicate-checklist-item

  set_issue_body $'## Purpose\nContract integrity fixture.'
  reset_state
  printf '%s\n' "$contract_body" > .agent-state/issues/123/implementation-contract.md
  printf '%s  implementation-contract.md\n' deadbeef > .agent-state/issues/123/implementation-contract.sha256
  expect_parity_prepare_failure contract-integrity

  printf '%s\n' "$contract_body" > .agent-state/issues/123/implementation-contract.md
  python3 - .agent-state/issues/123/implementation-contract.md .agent-state/issues/123/implementation-contract.sha256 <<'PY'
import hashlib
import sys
from pathlib import Path
p = Path(sys.argv[1])
Path(sys.argv[2]).write_text(f"{hashlib.sha256(p.read_bytes()).hexdigest()}  implementation-contract.md\n", encoding="utf-8")
PY
  set_issue_body $'## Reviewer Checklist\n\n- [ ] Issue supplemental one\n- [ ] Issue supplemental two'

  for mode in pending fail no-evidence prose-evidence no-reason unknown duplicate missing; do
    fresh_review
    fill_pass_review
    mutate_review "$mode"
    case "$mode" in
      pending) expected=pending-item ;;
      fail) expected=failed-item ;;
      no-evidence|prose-evidence) expected=missing-evidence ;;
      no-reason) expected=missing-na-reason ;;
      unknown) expected=unknown-result-id ;;
      duplicate) expected=duplicate-result-id ;;
      missing) expected=missing-result-id ;;
    esac
    expect_parity_validate_failure "$expected"
  done

  fresh_review
  fill_pass_review
  set_issue_body $'## Reviewer Checklist\n\n- [ ] changed source obligation'
  expect_parity_validate_failure stale-checklist
  set_issue_body $'## Reviewer Checklist\n\n- [ ] Issue supplemental one\n- [ ] Issue supplemental two'

  fresh_review
  fill_pass_review
  printf '%s\n' changed >> .agent-state/issues/123/implementation-contract.md
  expect_parity_validate_failure contract-integrity
  printf '%s\n' "$contract_body" > .agent-state/issues/123/implementation-contract.md
  python3 - .agent-state/issues/123/implementation-contract.md .agent-state/issues/123/implementation-contract.sha256 <<'PY'
import hashlib
import sys
from pathlib import Path
p = Path(sys.argv[1])
Path(sys.argv[2]).write_text(f"{hashlib.sha256(p.read_bytes()).hexdigest()}  implementation-contract.md\n", encoding="utf-8")
PY

  fresh_review
  fill_pass_review
  printf '%s\n' dirty >> tracked.txt
  expect_parity_validate_failure dirty-worktree
  git checkout -- tracked.txt

  fresh_review
  fill_pass_review
  git commit --allow-empty -qm parity-stale-head
  expect_parity_validate_failure stale-head
  "$TMP/scripts/agent/prepare-self-review.sh" 123 >/dev/null
  fill_pass_review
  echo 'T7 POSIX/PowerShell failure-class parity: PASS'

  pwsh -NoProfile -File "$TMP/scripts/agent/prepare-self-review.ps1" 123 > "$STUB_BIN/pwsh-prepare.txt"
  ps_sha="$(sed -n 's/^review_checklist_sha256=//p' "$STUB_BIN/pwsh-prepare.txt")"
  posix_sha="$(sed -n 's/^Checklist-SHA256: //p' .agent-state/issues/123/reviewer-checklist.md)"
  [[ "$ps_sha" == "$posix_sha" ]]
  pwsh -NoProfile -File "$TMP/scripts/agent/validate-self-review.ps1" 123 > "$STUB_BIN/pwsh-validate.txt"
  grep -q '^self_review_status=pass$' "$STUB_BIN/pwsh-validate.txt"
  echo 'T21 POSIX/PowerShell parity: PASS'
else
  echo 'T21 POSIX/PowerShell parity: NOT RUN (pwsh unavailable)'
fi

echo 'self-review-workflow: PASS'
