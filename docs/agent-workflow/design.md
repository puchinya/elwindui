# Design phase

Read this file only while the associated Issue is in `phase:design`.

## Goal

Define how the approved requirements will be implemented, with enough precision that implementation does not require unreviewed architectural decisions.

Issue-level design may propose changes to public behavior, but once approved:

- normative public contracts belong in [`docs/specs/`](../specs/);
- durable implementation architecture belongs in [`docs/design/`](../design/);
- current implementation progress belongs in [`docs/status/`](../status/).

Follow the synchronization order in the root [`AGENTS.md`](../../AGENTS.md): establish an approved public contract before architecture, and establish durable architecture before code. Do not derive either from status.

## Required design topics

Cover only the topics relevant to the change:

- public API and externally visible behavior;
- type and module responsibilities;
- ownership and lifetime model;
- data and event flow;
- backend abstraction and backend-specific behavior;
- thread and async model;
- error representation and recovery;
- compatibility and migration impact;
- performance or caching constraints;
- test strategy;
- alternatives considered and why they were rejected.

Do not duplicate stable project-wide rules already present in the root `AGENTS.md` or authoritative specification documents. Link to them and record only decisions specific to the Issue.

## Reviewer Checklist approval gate

Before `phase:ready`, approve one effective task-specific Reviewer Checklist for implementation self-review.

- Direct requests require an Issue `## Reviewer Checklist` with at least one concrete checkbox item. It must cover task-specific risks that acceptance criteria alone do not make safe to infer, without generic repository-wide boilerplate.
- Supplied contracts use the checklist from the valid exact local contract mirror. Do not copy a complete usable contract checklist into the Issue.
- Add an Issue `## Reviewer Checklist` only for approved repository-specific supplemental obligations not represented by the contract. Preserve source identity and reject duplicate normalized item text across sources.
- If the supplied contract has no usable checklist, the Issue checklist is mandatory before `phase:ready`.
- The effective checklist must be decision-complete enough to judge normal, boundary, failure, synchronization, and platform/runtime risks relevant to the task. A material requirement discovered later returns to requirements/design rather than being silently added during implementation.

## Design artifacts

Keep ordinary changes in the Issue.

Create a repository design document, proposal, or ADR only when the decision is expected to remain useful beyond the Issue, such as:

- substantial public API changes;
- cross-backend architecture changes;
- ownership, threading, or persistence model changes;
- compatibility-breaking changes;
- major dependency introductions.

A durable design document may explain how an approved public API change is implemented, but the normative public contract itself must be reflected in the relevant [`docs/specs/`](../specs/) document.

## Approval gate

Do not begin a nontrivial implementation until the user or responsible maintainer approves the requirements and design.

A narrowly scoped bug fix, test addition, or documentation correction may treat the original implementation request as approval when:

- behavior is already defined by existing code or specifications;
- no public API or architecture decision is required;
- acceptance criteria are clear.

When approval is obtained:

1. Update the Issue body with the approved requirements, non-goals, design summary, and acceptance criteria.
2. Remove stale planning text that conflicts with the approved specification.
3. Replace `phase:design` with `phase:ready`.
4. Remove `needs-user-decision` if no decision remains.

Perform Issue and label updates with `gh`.

If approval changes the requirements materially, remain in `phase:design` until the revised design is consistent.

Implementation instructions are in `docs/agent-workflow/implementation.md`.
