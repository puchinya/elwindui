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
scripts/agent/prepare-work-evidence.sh "$ISSUE_NUMBER" >/dev/null

BASE=".agent-state/issues/$ISSUE_NUMBER"

python3 - "$ISSUE_JSON" "$ISSUE_NUMBER" "$BASE" <<'PY'
from __future__ import annotations

import hashlib
import json
import re
import sys
import unicodedata
from pathlib import Path
from typing import Optional


issue_json, issue_number_text, base_text = sys.argv[1:]
issue_number = int(issue_number_text)
base = Path(base_text)
contract = base / "implementation-contract.md"
contract_sha = base / "implementation-contract.sha256"
checklist_path = base / "reviewer-checklist.md"
checklist_sha_path = base / "reviewer-checklist.sha256"
self_review_path = base / "self-review.md"


def fail(message: str, failure_class: str = "invalid-input") -> "NoReturn":
    raise SystemExit(f"error[{failure_class}]: {message}")


def normalize_checklist_text(text: str) -> str:
    collapsed = re.sub(r"\s+", " ", text).strip()
    return unicodedata.normalize("NFC", collapsed)


def normalize_duplicate_key(text: str) -> str:
    normalized = normalize_checklist_text(text)
    return "".join(
        chr(ord(char) + 32) if "A" <= char <= "Z" else char
        for char in normalized
    )


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
canonical_begin = "ELWINDUI_REVIEWER_CHECKLIST_V1_BEGIN"
canonical_end = "ELWINDUI_REVIEWER_CHECKLIST_V1_END"


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
                fail(f"{source} Reviewer Checklist has an empty checkbox item", "empty-checklist")
            checkbox = checkbox_re.fullmatch(candidate)
            if checkbox:
                item = normalize_checklist_text(checkbox.group(1))
                if not item:
                    fail(f"{source} Reviewer Checklist has an empty item", "empty-checklist")
                section_items.append(item)
        if not section_items:
            fail(f"{source} Reviewer Checklist section has zero checkbox items", "empty-checklist")
        items.extend(section_items)
    return items if found else []


def extract_canonical_checklist(text: str, source: str) -> Optional[list[str]]:
    lines = text.replace("\r\n", "\n").replace("\r", "\n").splitlines()
    begin_indices = [index for index, line in enumerate(lines) if line.strip() == canonical_begin]
    end_indices = [index for index, line in enumerate(lines) if line.strip() == canonical_end]
    if not begin_indices and not end_indices:
        return None
    if len(begin_indices) > 1 or len(end_indices) > 1:
        fail(f"{source} canonical checklist has multiple blocks", "canonical-checklist-multiple")
    if len(begin_indices) != 1 or len(end_indices) != 1 or end_indices[0] <= begin_indices[0]:
        fail(f"{source} canonical checklist markers are incomplete", "canonical-checklist-malformed")

    items: list[str] = []
    for line in lines[begin_indices[0] + 1 : end_indices[0]]:
        stripped = line.strip()
        if not stripped:
            continue
        if not stripped.startswith("REVIEW_ITEM:"):
            fail(f"{source} canonical checklist contains an unexpected line", "canonical-checklist-malformed")
        item = normalize_checklist_text(stripped[len("REVIEW_ITEM:") :])
        if not item:
            fail(f"{source} canonical checklist contains an empty REVIEW_ITEM", "canonical-checklist-empty")
        items.append(item)
    if not items:
        fail(f"{source} canonical checklist block is empty", "canonical-checklist-empty")
    return items


def extract_contract_checklist(text: str, source: str) -> list[str]:
    canonical_items = extract_canonical_checklist(text, source)
    return canonical_items if canonical_items is not None else extract_checklist(text, source)


def validate_contract() -> list[str]:
    present = contract.exists() or contract_sha.exists()
    if not present:
        return []
    if not contract.is_file() or not contract_sha.is_file():
        fail("contract mirror is incomplete", "contract-integrity")
    actual = hashlib.sha256(contract.read_bytes()).hexdigest()
    recorded = contract_sha.read_text(encoding="utf-8").strip()
    match = re.fullmatch(r"([0-9a-f]{64})\s+implementation-contract\.md", recorded)
    if not match or match.group(1) != actual:
        fail("contract mirror integrity check failed", "contract-integrity")
    return extract_contract_checklist(contract.read_text(encoding="utf-8"), "contract")


contract_items = validate_contract()
issue_items = extract_checklist(issue_body, "Issue")
if not contract_items and not issue_items:
    fail("no effective Reviewer Checklist exists", "missing-checklist")

entries: list[tuple[str, str, str]] = []
seen: dict[str, str] = {}


def add_items(prefix: str, source: str, items: list[str]) -> None:
    for offset, item in enumerate(items, start=1):
        key = normalize_duplicate_key(item)
        if key in seen:
            fail(
                "duplicate effective Reviewer Checklist item: "
                f"{source} item {prefix}{offset:03d} duplicates {seen[key]}",
                "duplicate-checklist-item",
            )
        item_id = f"{prefix}{offset:03d}"
        seen[key] = item_id
        entries.append((item_id, source, item))


add_items("C", "contract", contract_items)
add_items("I", "issue", issue_items)

canonical = "".join(f"{item_id}\t{item}\n" for item_id, _source, item in entries)
fingerprint = hashlib.sha256(canonical.encode("utf-8")).hexdigest()

snapshot = [
    "# Reviewer Checklist",
    "",
    f"Issue: #{issue_number}",
    f"Checklist-SHA256: {fingerprint}",
    "",
]
snapshot.extend(f"- {item_id} | {source} | {item}" for item_id, source, item in entries)
checklist_path.write_text("\n".join(snapshot) + "\n", encoding="utf-8")

old_fingerprint = checklist_sha_path.read_text(encoding="utf-8").strip() if checklist_sha_path.exists() else ""
changed = old_fingerprint != fingerprint or not self_review_path.exists()
checklist_sha_path.write_text(fingerprint + "\n", encoding="utf-8")

if changed:
    review = [
        "# Self-review",
        "",
        f"Issue: #{issue_number}",
        f"Checklist-SHA256: {fingerprint}",
        "Reviewed-HEAD: ",
        "",
    ]
    review.extend(f"- {item_id} | PENDING |" for item_id, _source, _item in entries)
    self_review_path.write_text("\n".join(review) + "\n", encoding="utf-8")

print(f"review_checklist_path={checklist_path}")
print(f"review_checklist_sha256={fingerprint}")
print(f"self_review_path={self_review_path}")
print(f"checklist_changed={1 if changed else 0}")
print(f"items={len(entries)}")
PY
