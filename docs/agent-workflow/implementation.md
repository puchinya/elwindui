# Implementation phase

Read this file only while the associated Issue is in `phase:ready` or `phase:implementation`.

## Starting work

Before editing:

1. Re-read the approved Issue specification and acceptance criteria.
2. Confirm that no newer comment or linked decision supersedes the Issue body.
3. If the task was supplied as an Implementation Contract, follow the mirror gate below.
4. For source-code changes, create/switch through `scripts/agent/start-feature-branch.sh <issue> <short-description>` or the PowerShell equivalent. Its branch name is authoritative for source work.
5. An actual helper-driven branch switch must keep its mandatory `cargo clean`; this disk-space invariant must not be optimized away.
6. Replace `phase:ready` with `phase:implementation`.
7. Confirm required upstream spec/design updates are approved before code editing.

Documentation/workflow-only changes may use the existing `docs/` or `agent/` branch allowance.

The approved Issue defines repository task scope. A compatible supplied contract is a compressed decision-complete handoff, not an override of normative specs/design.

## Supplied Implementation Contract mirror

After Issue ownership exists and before detailed implementation work, save the exact supplied contract:

```bash
scripts/agent/save-implementation-contract.sh <issue-number> <contract-file>
```

or on PowerShell:

```powershell
.\scripts\agent\save-implementation-contract.ps1 <issue-number> <contract-file>
```

The mirror lives at:

```text
.agent-state/issues/<issue-number>/implementation-contract.md
.agent-state/issues/<issue-number>/implementation-contract.sha256
```

The helper is immutable-on-conflict and idempotent-on-match. Never silently replace a different existing contract.

If the mirror exists, re-read it:

- after context compaction;
- after session resume/handoff;
- before the final complete-diff self-review.

Do not duplicate the contract into another persistent context summary.

## Context-efficient execution

Use `scripts/agent/agent-context.* <issue-number>` for compact routing/bootstrap state.

Keep only the Issue-scoped working set described in `AGENTS.md`. Inspect referenced files/symbols for implementation details, but do not broadly re-derive architecture already resolved by an approved compatible contract unless repository evidence conflicts.

Keep large output in `.agent-state/issues/<issue>/logs/` and inspect bounded excerpts.

## Implementation rules

- Stay inside approved scope.
- Do not mix unrelated refactoring/cleanup.
- Preserve repository authority and architectural invariants.
- Do not expose backend-specific types through common APIs without approval.
- Do not add dependencies without an approved reason.
- Add/update tests for behavior and acceptance criteria.
- Synchronize only documentation whose responsibility actually changed:
  - public contract -> `docs/specs/`
  - durable architecture -> `docs/design/`
  - concise current implementation/gap/verification state -> `docs/status/`
- Keep evidence/history in Issue/PR/evidence artifacts, not `docs/status/`.

If code/spec/design conflict requires a material public API, compatibility, ownership, backend-boundary, threading, dependency, non-goal, or acceptance change, stop and return to `phase:design`.

## Verification

`docs/agents/testing.md` is the sole Rust verification command authority.

During the edit/debug loop, use the narrowest relevant check/test. Do not repeatedly run the complete workspace/final gate merely as a progress probe.

Once a Rust-affecting change is stable, run the complete mandatory Rust gate before PR delivery. If Rust-affecting review remediation occurs, use focused checks while editing, then rerun the complete gate once the remediation is stable.

Record commands/results honestly, including untested environments.

## Self-review

Before creating/updating the PR:

1. If a contract mirror exists, verify its SHA/integrity with `scripts/agent/agent-context.*` and re-read the exact contract.
2. Inspect the complete diff.
3. Verify every acceptance criterion is satisfied or explicitly incomplete.
4. Verify implementation matches approved Issue/design/contract decisions.
5. Verify no unrelated changes.
6. Verify tests cover important normal/boundary/failure behavior.
7. Verify specs/design/status/Agent paths remain synchronized.
8. Verify status contains current state rather than PR/evidence history.
9. Verify error handling, unsafe assumptions, generated files, and lockfile changes are intentional.

## Implementation completion gate

Before reporting implementation-phase completion:

- changes are committed;
- branch is pushed;
- PR exists and contains `Closes #<issue-number>`;
- Issue transitioned to `phase:review`;
- `docs/agent-workflow/review.md` has been entered.

If PR creation or phase transition fails, report the task as blocked with the exact command/error.

## Transition to review

1. Update the Issue acceptance checklist and only a concise implementation status.
2. Put delta/evidence/risk/reviewer guidance in the PR; do not restate the whole Issue/contract.
3. Create the PR with `Closes #<issue-number>`.
4. Replace `phase:implementation` with `phase:review`.
5. Read `docs/agent-workflow/review.md`.

Use `gh` for GitHub operations and `git` for local branch/commit/push.
