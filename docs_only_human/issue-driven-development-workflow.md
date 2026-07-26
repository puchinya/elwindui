# Issue-driven AI development workflow

This document explains the development workflow for human maintainers.

Agent-facing instructions are intentionally split across `AGENTS.md` and
`docs/agent-workflow/`. Humans may read this document as a single overview.
AI agents should not load this file during ordinary work unless a user
explicitly asks for the human-facing explanation.

## Purpose

The workflow is designed to keep AI-assisted development traceable without
placing every rule in the always-loaded `AGENTS.md`.

It separates:

- the request and approved specification in a GitHub Issue;
- the implementation and verification evidence in a Pull Request;
- project-wide rules in `AGENTS.md`;
- phase-specific agent instructions in `docs/agent-workflow/`.

This reduces context-window use and prevents requirements, design,
implementation, and review responsibilities from being mixed.

## Lifecycle

```text
Request
  -> Issue created or located
  -> Requirements
  -> Design
  -> Approval
  -> Ready
  -> Implementation and verification
  -> Pull Request and review
  -> Merge
  -> Issue closed
```

## Phase labels

Exactly one of the following phase labels should be active:

- `phase:requirements`
- `phase:design`
- `phase:ready`
- `phase:implementation`
- `phase:review`

The following labels are supplementary and may coexist with a phase:

- `blocked`
- `needs-user-decision`

`blocked` is not a phase. It means work cannot continue because of an external
or technical dependency.

`needs-user-decision` means a product, API, compatibility, scope, or
architecture decision is required from the requester or maintainer.

## When to create an Issue

Create or locate an Issue before changing source code or documentation.

Do not create an Issue automatically for:

- explanation;
- code reading;
- research;
- exploratory design discussion;
- comparison with another framework.

Create an Issue when:

- a repository change is requested;
- work should be tracked;
- an exploratory discussion is approved as planned work;
- a bug, feature, refactoring, test, or documentation change will be made.

## Initial Issue

The first Issue does not need a complete specification.

A minimal Issue may contain:

```markdown
## Initial request

The request as received.

## Planning state

Requirements and design are being discussed.
The approved specification will be added after approval.
```

During a short planning session, draft reasoning stays in the conversation.

If planning spans multiple sessions, add one concise checkpoint comment with:

- decisions already made;
- unresolved questions;
- next action.

Do not rewrite the Issue body after every conversation turn.

## Requirements phase

The requirements phase determines what will be built.

The Issue should ultimately identify:

- background;
- objective;
- functional requirements;
- non-goals;
- constraints;
- verifiable acceptance criteria;
- unresolved questions.

A requirement should describe externally observable behavior or a testable
condition. It should not merely say that a type or function was implemented.

## Design phase

The design phase determines how the requirements will be implemented.

Relevant topics may include:

- public API;
- type and module responsibilities;
- ownership and lifetime model;
- data and event flow;
- backend boundary;
- threading and async behavior;
- error handling;
- compatibility;
- performance constraints;
- test strategy;
- alternatives and rejection reasons.

Ordinary changes can keep the design in the Issue.

Create a proposal or ADR only when the decision should remain useful beyond
the Issue, such as a substantial public API, architecture, ownership,
threading, compatibility, or dependency decision.

## Approval

Nontrivial implementation begins after requirements and design are approved.

After approval:

1. Replace the planning text in the Issue with the approved requirements,
   non-goals, design summary, and acceptance criteria.
2. Set `phase:ready`.
3. Begin implementation.

A narrowly scoped bug fix or documentation correction may treat the original
implementation request as approval when existing behavior already defines the
solution and no architecture or public API decision is required.

## Feature branches

Any change to source code must be made on a dedicated feature branch.

Branch format:

```text
feature/<issue-number>-<short-slug>
```

Examples:

```text
feature/123-graphics-path
feature/245-pointer-capture
```

Create the branch from the current remote default branch, not from an arbitrary
local branch. Do not edit source code directly on `master`, `main`, or another
default branch.

Use:

```bash
scripts/agent/start-feature-branch.sh 123 "graphics path"
```

Documentation-only or workflow-only changes may use a `docs/` or `agent/`
branch instead.

## Implementation and verification

When work starts:

1. Create or switch to the Issue feature branch.
2. Set `phase:implementation`.
3. Treat the approved Issue as the implementation contract.
4. Keep unrelated refactoring out of the change.
5. Run the repository checks documented in `AGENTS.md`.
6. Record checks that were not run and why.

If implementation reveals that the approved design must materially change,
return the Issue to `phase:design` before continuing.

Examples include:

- changing an approved public API;
- expanding a non-goal into scope;
- breaking compatibility;
- changing backend, ownership, or threading boundaries;
- adding a major dependency;
- changing acceptance criteria.

## Pull Request and review

The Pull Request should contain:

- purpose and impact;
- main changes;
- important implementation decisions;
- verification commands and results;
- untested environments;
- compatibility and residual risks;
- reviewer guidance;
- `Closes #<issue-number>`.

Detailed implementation and verification evidence belongs primarily in the
Pull Request. The Issue remains the approved specification.

Set `phase:review` after opening the Pull Request.

The Issue is not complete merely because the Pull Request is approved. It is
complete only after required CI succeeds and the Pull Request is merged into
the default branch.

## Issue and Pull Request responsibilities

### Issue

The Issue owns:

- background and objective;
- requirements;
- non-goals and constraints;
- approved design summary;
- acceptance criteria;
- material approved specification changes;
- link to the Pull Request.

### Pull Request

The Pull Request owns:

- actual code and documentation changes;
- implementation-specific decisions;
- test and verification evidence;
- residual risks;
- reviewer guidance;
- review discussion.

## Rust version milestone

For a Rust repository, each tracked Issue is assigned to the GitHub Milestone
matching the version in the root `Cargo.toml`.

Resolution order:

1. `[workspace.package].version`
2. `[package].version`

The version string is used exactly as the Milestone title.

Examples:

```text
0.1.0
1.4.0-beta.2
```

Do not add a `v` prefix.

Use:

```bash
scripts/agent/ensure-version-milestone.sh <issue-number>
```

The helper:

- reads the root Cargo version;
- finds an exact-title Milestone;
- creates it when absent;
- assigns the Issue when an Issue number is supplied;
- rejects duplicate exact-title Milestones;
- stops when the exact-title Milestone exists but is closed.

A closed matching Milestone usually means either the Milestone must be
reopened intentionally or `Cargo.toml` should move to the next development
version. The helper does not make that release decision automatically.

## Agent instruction files

Agents load only the file for the current phase:

| Effective phase | Agent instruction |
|---|---|
| `phase:requirements` | `docs/agent-workflow/requirements.md` |
| `phase:design` | `docs/agent-workflow/design.md` |
| `phase:ready`, `phase:implementation` | `docs/agent-workflow/implementation.md` |
| `phase:review`, open associated PR | `docs/agent-workflow/review.md` |

The complete human explanation is this file. It is not part of the normal
agent phase-routing path.

## Manual operational example

Create an Issue:

```bash
ISSUE_URL="$(
  gh issue create \
    --title "Add GraphicsPath API" \
    --body-file /tmp/issue.md
)"
ISSUE_NUMBER="${ISSUE_URL##*/}"
scripts/agent/ensure-version-milestone.sh "$ISSUE_NUMBER"
```

After requirements and design are approved:

```bash
gh issue edit "$ISSUE_NUMBER" \
  --remove-label "phase:design" \
  --add-label "phase:ready"
```

When implementation starts:

```bash
scripts/agent/start-feature-branch.sh \
  "$ISSUE_NUMBER" \
  "graphics path"

gh issue edit "$ISSUE_NUMBER" \
  --remove-label "phase:ready" \
  --add-label "phase:implementation"
```

When the Pull Request is opened:

```bash
gh issue edit "$ISSUE_NUMBER" \
  --remove-label "phase:implementation" \
  --add-label "phase:review"
```

The Pull Request body should contain:

```text
Closes #<issue-number>
```

After merge, GitHub closes the Issue automatically.
