#!/usr/bin/env bash
set -euo pipefail

ISSUE_NUMBER="${1:-}"
[[ "$ISSUE_NUMBER" =~ ^[1-9][0-9]*$ ]] || {
  echo "usage: $0 <issue-number>" >&2
  exit 1
}

for name in git gh python3; do
  command -v "$name" >/dev/null 2>&1 || {
    echo "error: required command not found: $name" >&2
    exit 1
  }
done

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
ISSUE_JSON="$(gh issue view "$ISSUE_NUMBER" --repo "$REPOSITORY" --json number,body)"
BASE=".agent-state/issues/$ISSUE_NUMBER"

python3 - "$ISSUE_JSON" "$ISSUE_NUMBER" "$BASE" <<'PY'
from __future__ import annotations

import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path


issue_json, issue_number_text, base_text = sys.argv[1:]
issue_number = int(issue_number_text)
base = Path(base_text)
contract = base / "implementation-contract.md"
contract_sha = base / "implementation-contract.sha256"
checklist_path = base / "reviewer-checklist.md"
checklist_sha_path = base / "reviewer-checklist.sha256"
self_review_path = base / "self-review.md"


def fail(message: str) -> "NoReturn":
    raise SystemExit(f"error: {message}")


try:
    issue = json.loads(issue_json)
except json.JSONDecodeError as exc:
    fail(f"Issue metadata is not valid JSON: {exc}")
if issue.get("number") != issue_number:
    fail(f"Issue metadata does not match #{issue_number}")
issue_body = issue.get("body")
if not isinstance(issue_body, str):
    fail(f"Issue #{issue_number} has no readable body")

heading_re = re.compile(
    r"^(#{1,6})[ \t]+(?:[0-9]+[.)][ \t]+)?Reviewer Checklist[ \t]*#*[ \t]*$"
)
generic_heading_re = re.compile(r"^(#{1,6})(?:[ \t]+.*)?$")
checkbox_re = re.compile(r"^[ \t]*-[ \t]+\[[ xX]\][ \t]+(.+?)\s*$")
empty_checkbox_re = re.compile(r"^[ \t]*-[ \t]+\[[ xX]\][ \t]*$")


def extract_checklist(text: str, source: str) -> list[str]:
    lines = text.replace("\r\n", "\n").replace("\r", "\n").splitlines()
    visible: list[bool] = []
    fenced = False
    for line in lines:
        visible.append(not fenced)
        if re.match(r"^[ \t]*(```|~~~)", line):
            fenced = not fenced
    items: list[str] = []
    found = False
    for index, line in enumerate(lines):
        if not visible[index]:
            continue
        match = heading_re.fullmatch(line)
        if not match:
            continue
        found = True
        level = len(match.group(1))
        section_items: list[str] = []
        for candidate_index in range(index + 1, len(lines)):
            candidate = lines[candidate_index]
            if not visible[candidate_index]:
                continue
            next_heading = generic_heading_re.fullmatch(candidate)
            if next_heading and len(next_heading.group(1)) <= level:
                break
            if empty_checkbox_re.fullmatch(candidate):
                fail(f"{source} Reviewer Checklist has an empty checkbox item")
            checkbox = checkbox_re.fullmatch(candidate)
            if checkbox:
                item = " ".join(checkbox.group(1).split())
                if not item:
                    fail(f"{source} Reviewer Checklist has an empty item")
                section_items.append(item)
        if not section_items:
            fail(f"{source} Reviewer Checklist section has zero checkbox items")
        items.extend(section_items)
    return items if found else []


def validate_contract() -> list[str]:
    present = contract.exists() or contract_sha.exists()
    if not present:
        return []
    if not contract.is_file() or not contract_sha.is_file():
        fail("contract mirror is incomplete")
    actual = hashlib.sha256(contract.read_bytes()).hexdigest()
    recorded = contract_sha.read_text(encoding="utf-8").strip()
    match = re.fullmatch(r"([0-9a-f]{64})\s+implementation-contract\.md", recorded)
    if not match or match.group(1) != actual:
        fail("contract mirror integrity check failed")
    return extract_checklist(contract.read_text(encoding="utf-8"), "contract")


contract_items = validate_contract()
issue_items = extract_checklist(issue_body, "Issue")
if not contract_items and not issue_items:
    fail("no effective Reviewer Checklist exists")

entries: list[tuple[str, str]] = []
seen: dict[str, str] = {}


def add_items(prefix: str, items: list[str]) -> None:
    for offset, item in enumerate(items, start=1):
        key = item.casefold()
        if key in seen:
            fail(f"duplicate effective Reviewer Checklist item: {prefix}{offset:03d} duplicates {seen[key]}")
        item_id = f"{prefix}{offset:03d}"
        seen[key] = item_id
        entries.append((item_id, item))


add_items("C", contract_items)
add_items("I", issue_items)
fingerprint = hashlib.sha256(
    "".join(f"{item_id}\t{item}\n" for item_id, item in entries).encode("utf-8")
).hexdigest()

if not checklist_path.is_file() or not checklist_sha_path.is_file():
    fail("prepared Reviewer Checklist artifacts are missing")
prepared_sha = checklist_sha_path.read_text(encoding="utf-8").strip()
if not re.fullmatch(r"[0-9a-f]{64}", prepared_sha) or prepared_sha != fingerprint:
    fail("prepared Reviewer Checklist source is stale")

if not self_review_path.is_file():
    fail("self-review artifact is missing")
self_review = self_review_path.read_text(encoding="utf-8")


def metadata(name: str, pattern: str) -> str:
    matches = [line for line in self_review.splitlines() if line.startswith(f"{name}:")]
    if len(matches) != 1:
        fail(f"self-review metadata {name} is missing or duplicated")
    match = re.fullmatch(pattern, matches[0])
    if not match:
        fail(f"self-review metadata {name} is malformed")
    return match.group(1)


metadata("Issue", rf"Issue: #({issue_number})")
review_sha = metadata("Checklist-SHA256", r"Checklist-SHA256: ([0-9a-f]{64})")
if review_sha != fingerprint:
    fail("self-review checklist SHA does not match the current source")
reviewed_head = metadata("Reviewed-HEAD", r"Reviewed-HEAD: ([0-9a-f]{40})")

dirty = subprocess.run(
    ["git", "status", "--porcelain", "--untracked-files=all"],
    check=True,
    capture_output=True,
    text=True,
).stdout.strip()
if dirty:
    fail("repository-controlled worktree is dirty")

current_head = subprocess.run(
    ["git", "rev-parse", "HEAD"], check=True, capture_output=True, text=True
).stdout.strip()
if reviewed_head != current_head:
    fail("Reviewed-HEAD is stale")
try:
    subprocess.run(["git", "cat-file", "-e", f"{reviewed_head}^{{commit}}"], check=True, capture_output=True)
except subprocess.CalledProcessError:
    fail("Reviewed-HEAD is not a valid commit")

expected = {item_id for item_id, _item in entries}
results: dict[str, tuple[str, str]] = {}
for line in self_review.splitlines():
    if not line.startswith("- "):
        continue
    parts = line[2:].split("|", 2)
    if len(parts) != 3:
        fail("self-review contains a malformed result entry")
    item_id, status, detail = (part.strip() for part in parts)
    if item_id in results:
        fail(f"self-review contains duplicate result ID: {item_id}")
    if item_id not in expected:
        fail(f"self-review contains unknown result ID: {item_id}")
    results[item_id] = (status, detail)

missing = sorted(expected - results.keys())
if missing:
    fail("self-review is missing result IDs: " + ", ".join(missing))

pass_count = 0
na_count = 0
for item_id, (status, detail) in results.items():
    if status == "PASS":
        if not detail.startswith("Evidence:") or not detail[len("Evidence:") :].strip():
            fail(f"PASS item {item_id} requires concrete Evidence:")
        evidence = detail[len("Evidence:") :].strip().casefold()
        if evidence in {"looks good", "none", "n/a", "todo", "pending"}:
            fail(f"PASS item {item_id} has non-concrete Evidence:")
        pass_count += 1
    elif status == "N/A":
        if not detail.startswith("Reason:") or not detail[len("Reason:") :].strip():
            fail(f"N/A item {item_id} requires a concrete Reason:")
        na_count += 1
    elif status == "PENDING":
        fail(f"self-review item {item_id} is PENDING")
    elif status == "FAIL":
        fail(f"self-review item {item_id} is FAIL")
    else:
        fail(f"self-review item {item_id} has invalid status: {status}")

print("self_review_status=pass")
print(f"reviewed_head={reviewed_head}")
print(f"review_checklist_sha256={fingerprint}")
print(f"pass={pass_count}")
print(f"na={na_count}")
print("fail=0")
PY
