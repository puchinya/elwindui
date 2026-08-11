# Requirements phase

Read this file only while the associated Issue is in `phase:requirements`.

## Goal

Turn the initial request into a bounded, testable problem statement without starting implementation.

## Required actions

Use `gh` for every GitHub Issue, label, milestone, comment, and Pull Request operation in this workflow. The root [`AGENTS.md`](../../AGENTS.md) is authoritative for GitHub tooling and document synchronization.

1. Search for an existing Issue, Pull Request, implementation, and relevant specification before creating anything new.
2. If this is a repository-changing task and no Issue exists, create one before editing source or documentation.
3. For a Rust repository, derive the target milestone from the root `Cargo.toml`:
   - prefer `[workspace.package].version`;
   - otherwise use `[package].version`;
   - use the version string exactly as the GitHub Milestone title, without adding a `v` prefix;
   - create the Milestone when no exact-title Milestone exists;
   - assign the Issue to that Milestone.
   Use `scripts/agent/ensure-version-milestone.sh <issue-number>` on macOS/Linux or `scripts/agent/ensure-version-milestone.ps1 <issue-number>` in PowerShell.
4. Keep the initial Issue small:
   - original request;
   - current planning state;
   - known links or affected areas;
   - unresolved questions that block progress.
   When the request includes a file that defines or materially constrains the work (for example,
   an implementation directive), attach the original file to the Issue before implementation.
5. Inspect the relevant code and only the relevant sections of long specification documents. Treat existing normative specifications in [`docs/specs/`](../specs/) as the authoritative baseline for requirements unless the request explicitly proposes a specification change.
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

Do not create an Issue for explanation, code reading, research, or exploratory design discussion unless the user explicitly requests tracking.

Create or locate an Issue when the user requests a repository change, asks that work be tracked, or approves an exploratory discussion as planned work.

## Completion criteria

The requirements phase is complete when:

- the objective and scope are unambiguous enough to design;
- non-goals prevent obvious scope expansion;
- acceptance criteria are observable or testable;
- unresolved questions do not prevent design work.

Then replace `phase:requirements` with `phase:design` and read `docs/agent-workflow/design.md`.
