# Requirements phase

Read this file in either of these cases:

- a new repository-changing request has no associated Issue yet; this file defines the bootstrap procedure for creating its `phase:requirements` Issue;
- the associated Issue is currently in `phase:requirements`.

For a new repository-changing request with no existing Issue, `phase:requirements` is always the workflow entry point.

## Goal

Turn the initial request into a bounded, testable problem statement without starting implementation.

## Required actions

Use `gh` for every GitHub Issue, label, milestone, comment, and Pull Request operation in this workflow. The root [`AGENTS.md`](../../AGENTS.md) is authoritative for GitHub tooling, task bootstrap order, and document synchronization.

1. If the request does not already identify an owning Issue or Pull Request, perform only the minimal GitHub lookup required to determine whether an existing Issue or Pull Request already owns the request. Do not inspect implementation or specifications as a substitute for this bootstrap lookup.
2. If this is a repository-changing task and no Issue owns the request, create one immediately with `gh issue create`, assign `phase:requirements`, and do so before agent-local planning, task-list creation, broad repository investigation, or any repository edit.
3. For a Rust repository, derive the target milestone from the root `Cargo.toml`:
   - prefer `[workspace.package].version`;
   - otherwise use `[package].version`;
   - use the version string exactly as the GitHub Milestone title, without adding a `v` prefix;
   - create the Milestone when no exact-title Milestone exists;
   - assign the Issue to that Milestone.
   Use `scripts/agent/ensure-version-milestone.sh <issue-number>` on macOS/Linux or `scripts/agent/ensure-version-milestone.ps1 <issue-number>` in PowerShell. For a newly created Issue, complete this step before normal planning or detailed repository investigation.
4. Keep the initial Issue small:
   - original request;
   - current planning state;
   - known links or affected areas;
   - unresolved questions that block progress.
   When the request includes a file that defines or materially constrains the work (for example,
   an implementation directive), attach the original file to the Issue before implementation.
5. After Issue ownership and the milestone bootstrap are established, inspect the relevant code and only the relevant sections of long specification documents. Treat existing normative specifications in [`docs/specs/`](../specs/) as the authoritative baseline for requirements unless the request explicitly proposes a specification change.
6. Separate the following explicitly:
   - background;
   - objective;
   - functional requirements;
   - non-goals;
   - constraints;
   - verifiable acceptance criteria;
   - unresolved questions.
   Classify any proposed code change as public contract, internal architecture, implementation-only, bug fix, or verification-only so the required upstream documents are known before design begins.
7. Do not silently resolve an ambiguity that would materially change public API, compatibility, architecture, supported platforms, or scope.
8. Use `needs-user-decision` when a user decision blocks progress. Use `blocked` only for an external or technical blocker.
9. Do not rewrite the Issue body after every exchange. Keep draft reasoning in the active conversation.
10. If planning must continue in another session, add one concise checkpoint comment containing only decisions, remaining questions, and the next action.

## Issue creation boundary

Classify the task from the user's requested end result, not from the agent's current activity.

If fulfilling the request is expected to modify code, documentation, tests, configuration, scripts, workflows, or other repository-controlled files, the task is repository-changing from the beginning. Preliminary investigation for such a task does not make it research-only.

Research-only means the requested deliverable itself is explanation, code reading, research, or exploratory design discussion and no repository modification has been requested or approved. Do not create an Issue for research-only work unless the user explicitly requests tracking.

Create or locate an Issue when the user requests a repository change, asks that work be tracked, or approves exploratory work as planned repository work. If a research-only conversation later becomes a repository-changing request, run the repository-changing bootstrap at that point before planning or continuing repository-changing work.

## Completion criteria

The requirements phase is complete when:

- the objective and scope are unambiguous enough to design;
- non-goals prevent obvious scope expansion;
- acceptance criteria are observable or testable;
- unresolved questions do not prevent design work.

Then replace `phase:requirements` with `phase:design` and read `docs/agent-workflow/design.md`.
