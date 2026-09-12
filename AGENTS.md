# AGENTS.md

This is the repository-wide entry point for AI coding agents, including Codex and Claude Code. `CLAUDE.md` is only a Claude Code router and must defer to this file for shared rules.

## Mandatory task bootstrap

Before agent-local planning, TODO creation, broad repository investigation, or editing repository-controlled files, classify the user's requested end result.

For every repository-changing task:

1. If the user identified an Issue or Pull Request, use it. Otherwise perform only the minimal GitHub lookup needed to find an existing owner.
2. If no Issue owns the request, read `docs/agent-workflow/requirements.md`, create the Issue with `phase:requirements`, and run the platform `scripts/agent/ensure-version-milestone.* <issue-number>` helper before normal planning or detailed investigation.
3. If an Issue exists, determine the active phase from Issue labels / PR state and read only that phase workflow document.
4. Only after Issue ownership and the required workflow entry step may normal planning, detailed investigation, design, or repository editing begin.

Research-only work does not require an Issue unless requested. If research becomes an approved repository change, run this bootstrap before continuing repository-changing work.

Use `gh` for GitHub Issue/PR/label/milestone/comment/review/Actions operations and `git` for local branch/staging/commit/push operations.

GitHub Markdown transport invariant:
- Multiline Markdown MUST use `--body-file` when supported.
- The body source MUST contain real newline characters.
- `--body` is only for genuinely single-line text.
- Never encode intended line breaks as literal \n.

Issue image evidence:
- Useful/required image evidence SHOULD be attached to the owning Issue with `gh --attach`.
- This workflow uses Issue attachments for images only.
- Required image uploads must be verified after publication.

## Instruction input modes

Repository-changing work has two input modes:

- **Direct request**: follow the normal requirements -> design -> implementation -> review flow.
- **Supplied Implementation Contract**: treat it as a compressed handoff, not a workflow bypass. Associate it with the owning Issue and verify compatibility with approved repository authority. After Issue ownership exists and before detailed implementation work, save the exact contract with `scripts/agent/save-implementation-contract.*`. Do not broadly re-derive decisions already resolved by a compatible approved contract unless repository evidence conflicts.

If a local contract mirror exists, re-read it after context compaction/session resume and before final complete-diff self-review. The Issue/spec/design hierarchy remains authoritative if the contract conflicts.

## Compact workflow routing

Read only the active workflow document:

| State | Workflow |
|---|---|
| new request / `phase:requirements` | `docs/agent-workflow/requirements.md` |
| `phase:design` | `docs/agent-workflow/design.md` |
| `phase:ready` / `phase:implementation` | `docs/agent-workflow/implementation.md` |
| `phase:review` / open PR | `docs/agent-workflow/review.md` |

Read `docs/agent-workflow/checkpoint.md` only for pause/resume and `docs/agent-workflow/evidence.md` only for evidence capture.

For compact Git/GitHub routing state, use `scripts/agent/agent-context.* <issue-number>` instead of reconstructing the same state through many separate commands.

## Document authority

| Source | Authority |
|---|---|
| `docs/specs/` | normative public contract |
| `docs/design/` | durable internal architecture |
| source code | current implementation |
| tests | executable implementation evidence |
| `docs/status/` | concise current implementation / verification state |
| `docs/agents/` | technical working rules |
| `docs/agent-workflow/` | Issue phase workflow |

Dependency direction is:

```text
specs -> design -> code -> status
```

Do not change a spec to match a bug. Do not use status to decide desired behavior or architecture. If implementation exposes a missing/contradictory contract or a material architecture decision, return to the Issue requirements/design gate before deciding it in code.

Start document lookup at `docs/README.md`, then one category README, then only the relevant document sections/symbols.

## Context invariant

Keep an Issue-scoped working set: owning Issue/PR, active phase workflow, relevant routed spec/design/status sections, target symbols/tests/dependencies, and current relevant diff.

Use bounded search/ranges/diffs. Do not scan all specs/design/status, repeatedly reread unchanged whole files, inspect sibling backends without cross-backend need, or paste full logs into active context. Put retained raw logs under `.agent-state/issues/<issue>/logs/`.

`docs/status/` is current-state navigation, not an evidence/history archive. Detailed investigation and historical verification belong in Issue/PR/evidence artifacts.

Do not load `docs_only_human/` during ordinary Agent work; it is human overview material and is not an Agent authority.

Provider-specific context compressors such as RTK or context-mode may be used, but repository correctness must not depend on them.

## Branch and disk-space invariant

For source-code changes, use `scripts/agent/start-feature-branch.sh` or `.ps1`; its output is the source branch naming authority. Do not handcraft an alternate source branch merely to choose another prefix.

**An actual branch switch MUST run `cargo clean`.** This is a deliberate disk-space constraint for the development machines and must not be removed, weakened, made optional, or replaced by `sccache`/persistent shared build caches. When already on the requested branch, do not clean solely for branch setup.

Documentation/workflow-only tasks may use the repository's existing `docs/` or `agent/` branch allowance.

## Verification and live-runtime boundary

For every Rust-affecting task, the complete mandatory Rust verification gate in `docs/agents/testing.md` must pass before PR delivery and after stable Rust-affecting review remediation.

Use focused checks during iteration and the complete mandatory gate once the Rust change is stable; focused iteration never replaces the final gate.

Host-semantic native GUI/package/security/runtime acceptance must follow the host-context rules in `docs/agents/testing.md` and the relevant backend guide. Do not treat sandbox-only native runtime evidence as final host acceptance.

## Delivery gate

Implementation is not complete at edit, test, commit, or push.

Before implementation-phase completion is reported, changes must be committed and pushed, a PR with `Closes #<issue-number>` must exist, the Issue must be in `phase:review`, and `docs/agent-workflow/review.md` must have been entered. If delivery/transition fails, report the exact blocker rather than claiming completion.

Overall Issue completion remains merge-gated by the review workflow.

Every implementation has a task-specific effective Reviewer Checklist fixed before editing. Final delivery requires item-by-item PASS/FAIL/N/A results with concrete evidence bound to the reviewed committed HEAD; pending, failed, stale, or incomplete self-review blocks delivery. Details live in `docs/agent-workflow/implementation.md`.

## Technical guides

Read only the guide relevant to the task:

- General Rust: `docs/agents/common.md`
- DSL / codegen: `docs/agents/codegen.md`
- Class model: `docs/agents/class-model.md`
- Backend-common: `docs/agents/backend-common.md`
- AppKit: `docs/agents/appkit.md`
- WinUI 3: `docs/agents/winui3.md`
- Windows host setup: `docs/agents/windows.md`
- Testing / verification: `docs/agents/testing.md`

## Communication

Ask user questions in Japanese.
