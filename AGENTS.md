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

- Start at [`docs/README.md`](docs/README.md), then one category README, then the smallest relevant document set.
- Start code investigation from target symbols. This is an investigation strategy, not a source-of-truth rule for desired behavior.
- Search headings/symbols first and read only relevant ranges. Do not begin by scanning all specs, designs, or status files.
- Do not inspect sibling backend code unless cross-backend parity or shared behavior is in scope.
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
