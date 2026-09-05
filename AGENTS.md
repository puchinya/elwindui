# AGENTS.md

This file is the repository-wide instruction source for AI coding agents, including Codex and Claude Code. Claude Code enters through [`CLAUDE.md`](CLAUDE.md), which must defer to this file for shared rules.

## Mandatory task bootstrap

Before agent-local planning, creating a task list, performing broad repository investigation, or editing any repository-controlled file, classify the user's requested end result.

If fulfilling the request is expected to modify code, documentation, tests, configuration, scripts, workflows, or other repository-controlled files, treat it as a repository-changing task from the beginning. Preliminary investigation for such a task does not make it research-only.

For every repository-changing task:

1. If the user already identified an Issue or Pull Request, use it. Otherwise, perform only the minimal GitHub Issue / Pull Request lookup required to determine whether an existing item already owns the request.
2. If no Issue owns the request, immediately read [`docs/agent-workflow/requirements.md`](docs/agent-workflow/requirements.md) as the no-Issue bootstrap entry point, create the Issue with `gh issue create`, assign `phase:requirements`, and run the platform-appropriate `scripts/agent/ensure-version-milestone.* <issue-number>` script.
3. If an Issue already owns the request, determine its active phase from Issue labels / PR state and read only the workflow document for that phase.
4. Only after the task is associated with an Issue and the required workflow entry step is complete may normal planning, task decomposition, detailed repository investigation, design work, or repository editing begin.

This bootstrap gate takes precedence over agent-local planning workflows. Entering Planning Mode or Plan Mode, generating an implementation plan, or creating a TODO list does not bypass or postpone it.

For a new repository-changing request with no existing Issue, `phase:requirements` is always the workflow entry point.

Research-only means the requested deliverable itself is analysis, explanation, code reading, or exploratory discussion and no repository modification has been requested or approved. If the user later requests or approves a repository change, run this bootstrap at that point before planning or continuing repository-changing work.

## Instruction input modes

Repository-changing work supports two input modes through the same Issue-driven workflow:

- **Direct request**: Follow the normal mandatory bootstrap and requirements/design/implementation/review phases.
- **Supplied Implementation Contract**: Treat the contract as a compressed implementation handoff, not as a replacement for the repository workflow. Before editing, associate it with the owning Issue and check it against the approved Issue, normative specifications, and durable design. If it conflicts with repository authority or contains a material unapproved requirement or design decision, return to the appropriate requirements/design gate. When it is compatible with the approved Issue, treat its architecture and behavioral decisions as resolved inputs; inspect referenced artifacts for implementation details, but do not broadly re-derive the architecture unless new evidence conflicts with the contract.

Both input modes use the same branch, verification, delivery, and review requirements.

## Communication

When asking the user a question (clarifying questions, plan checkpoints, or approval requests), always ask in Japanese.

## Issue-driven development workflow

- Every repository-changing task must be associated with one GitHub Issue.
- Do not create an Issue for research-only work unless explicitly requested.
- Use the GitHub CLI (`gh`) for GitHub Issue, label, milestone, Pull Request, comment, review, and Actions operations. Use `git` for local branch, staging, commit, and push operations. Do not switch to another GitHub integration unless the user explicitly requests it or `gh` cannot perform the required operation.
- For a new repository-changing request with no existing Issue, use [`docs/agent-workflow/requirements.md`](docs/agent-workflow/requirements.md) as the bootstrap entry point and create a `phase:requirements` Issue before normal planning or detailed investigation.
- For an existing Issue, determine the active phase from Issue labels / PR state and read only the required workflow document:
  - `phase:requirements`: [`docs/agent-workflow/requirements.md`](docs/agent-workflow/requirements.md)
  - `phase:design`: [`docs/agent-workflow/design.md`](docs/agent-workflow/design.md)
  - `phase:ready` / `phase:implementation`: [`docs/agent-workflow/implementation.md`](docs/agent-workflow/implementation.md)
  - `phase:review` / open PR: [`docs/agent-workflow/review.md`](docs/agent-workflow/review.md)
- Read [`docs/agent-workflow/checkpoint.md`](docs/agent-workflow/checkpoint.md) only when pausing/resuming and [`docs/agent-workflow/evidence.md`](docs/agent-workflow/evidence.md) only when capturing evidence.
- Any Rust-affecting task must pass the mandatory Rust verification gate defined in [`docs/agents/testing.md`](docs/agents/testing.md) before Pull Request delivery. A failed or skipped mandatory check prevents reporting the implementation as complete.

## Delivery completion gate

An implementation task is not implementation-phase complete until all of the following have succeeded:

- the changes are committed;
- the working branch is pushed;
- a Pull Request exists and contains `Closes #<issue-number>`;
- the Issue has transitioned to `phase:review` and the review workflow has been entered.

Do not report implementation completion at edit, test, commit, or push. If Pull Request creation or the phase transition fails, report the result as blocked with the exact blocker and relevant command/error. Overall Issue completion remains governed by [`docs/agent-workflow/review.md`](docs/agent-workflow/review.md).

## Host-context live verification

Platform live-runtime verification whose result depends on authentic operating-system
security, package registration, desktop-session, native-GUI, or similar host semantics
must run outside any agent sandbox that can alter those semantics.

Sandbox execution remains valid for build/static checks and sandbox-safe tests. A
sandbox-only live-runtime pass or failure is diagnostic evidence only and cannot satisfy
or fail final platform acceptance until reproduced in host context.

If host-context execution is unavailable, report the live gate as blocked rather than
substituting sandbox evidence. Final runtime acceptance should use the normal,
non-elevated host user unless the behavior under test explicitly requires elevation.

See [`docs/agents/testing.md`](docs/agents/testing.md) for evidence validity and
[`docs/agents/winui3.md`](docs/agents/winui3.md) for the Windows/WinUI3 command boundary.

## Document authority

| Source | Authority | Question answered |
|---|---|---|
| [`docs/specs/`](docs/specs/) | normative public contract | What must ElwindUI do? |
| [`docs/design/`](docs/design/) | durable internal architecture | How is the contract implemented? |
| Source code | current implementation | What code exists now? |
| Tests | executable implementation evidence | What behavior is demonstrated? |
| [`docs/status/`](docs/status/) | current implementation/verification summary | What is implemented or missing now? |
| [`docs/agents/`](docs/agents/) | technical working rules | What must an implementing agent preserve? |
| [`docs/agent-workflow/`](docs/agent-workflow/) | Issue phase workflow | How is repository work advanced? |

The dependency direction is:

```text
specs -> design -> code -> status
```

Do not use current code or status to mechanically redefine a normative specification. When sources disagree, classify the difference as an implementation gap, stale design/status, or an explicitly approved specification change. Return to requirements/design approval when the desired contract or architecture is not determined.

## Code and documentation synchronization

Before changing code, use [`docs/README.md`](docs/README.md) and the category README files to select only the relevant spec, design, source, and—when current-state context is needed—status document.

| Change | Required order |
|---|---|
| Public API, DSL, property/event/binding/lifecycle semantics, public validation | relevant spec -> design when architecture changes -> code -> status |
| Internal architecture, ownership, cache, parser/codegen pipeline, backend internals | relevant design -> code -> status when state changes |
| Implementing an already-approved spec/design gap | code -> status |
| Bug fix where code violates an existing contract | code -> status when state/verification changes |
| Verification only | tests/evidence -> status |

Rules:

- Do not change a spec to match a bug.
- Do not update design when the architecture did not change.
- Do not use status as input for deciding desired behavior or architecture.
- If implementation reveals a missing/contradictory contract or a durable architecture decision, stop and follow the Issue requirements/design approval gate before deciding it in code.
- After code changes, check whether the relevant spec, design, status, Agent invariant, commands, and document paths remain synchronized.

## Context minimization and routing

- Keep an Issue-scoped working set containing only:
  - the owning Issue or Pull Request;
  - the active phase workflow document;
  - the relevant documentation router/category README;
  - only the required specification, design, and status sections;
  - target source/test symbols and directly relevant dependencies;
  - the current relevant diff.
- Start at [`docs/README.md`](docs/README.md), then one category README, then the smallest relevant document set.
- Start code investigation from target symbols. This is an investigation strategy, not a source-of-truth rule for desired behavior.
- Search headings/symbols first and read only relevant ranges. Do not begin by scanning all specs, designs, or status files.
- Do not inspect sibling backend code unless cross-backend parity or shared behavior is in scope.
- After the working set is established, do not restart broad repository scans unless new evidence shows that the approved scope or architecture is insufficient.
- Do not repeatedly reread unchanged whole documents or source files merely to reconstruct context; reopen only the specific ranges or symbols needed.
- Use bounded, scoped search and diff output. Use the narrowest relevant check or test during iteration, then run the required broad verification once the change is stable.
- When full or high-volume command output must be retained, store it under `.agent-state/issues/<issue-number>/logs/` and inspect only bounded failure or relevant excerpts.
- Never paste full build or test logs into the active conversation, Issue, Pull Request, or checkpoint.
- Use the existing checkpoint mechanism when pausing, resuming, or creating a compact continuation or handoff.
- Do not create a separate persistent context document that duplicates Issue, contract, or checkpoint state.
- Context reduction must never suppress required errors, acceptance evidence, or final verification.
- Do not load `docs_only_human/` during ordinary Agent tasks. It is a human overview and cannot be the only source of a required contract, architecture invariant, command, or current status.

## Technical domain guides

When implementing or editing code, read only the relevant guide:

- General Rust: [`docs/agents/common.md`](docs/agents/common.md)
- DSL / Codegen: [`docs/agents/codegen.md`](docs/agents/codegen.md)
- Class hierarchy and `#[class]`: [`docs/agents/class-model.md`](docs/agents/class-model.md)
- Backend-common layering: [`docs/agents/backend-common.md`](docs/agents/backend-common.md)
- AppKit: [`docs/agents/appkit.md`](docs/agents/appkit.md)
- WinUI 3: [`docs/agents/winui3.md`](docs/agents/winui3.md)
- Windows host setup shared by tools: [`docs/agents/windows.md`](docs/agents/windows.md)
- Testing and verification: [`docs/agents/testing.md`](docs/agents/testing.md)
