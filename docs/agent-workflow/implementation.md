# Implementation phase

Read this file only while the associated Issue is in `phase:ready` or `phase:implementation`.

## Starting work

Before editing:

1. Re-read the approved Issue specification and acceptance criteria.
2. Confirm that no newer comment or linked decision supersedes the Issue body.
3. For any source-code change, create or switch to a dedicated feature branch:
   - name it `feature/<issue-number>-<short-slug>`;
   - create it from the current remote default branch;
   - never edit source code directly on the default branch;
   - use `scripts/agent/start-feature-branch.sh <issue-number> <short-description>` on macOS/Linux or `scripts/agent/start-feature-branch.ps1 <issue-number> <short-description>` in PowerShell.
4. Documentation-only or workflow-only changes may use a `docs/` or `agent/` branch instead.
5. Replace `phase:ready` with `phase:implementation`.

The approved Issue is the implementation contract.

## Implementation rules

- Keep the change within the approved scope.
- Do not mix unrelated refactoring, cleanup, or formatting.
- Preserve the architectural rules in the root `AGENTS.md`.
- Do not expose backend-specific types through common APIs unless explicitly approved.
- Do not introduce a new dependency without recording and justifying the decision.
- Add or update tests that verify behavior and acceptance criteria, not only implementation details.
- Update durable documentation when the public API, behavior, or architecture changes.

## When implementation invalidates the design

Stop implementation and return to `phase:design` before continuing when any of the following becomes necessary:

- changing an approved public API;
- expanding a non-goal into scope;
- breaking compatibility;
- changing the backend boundary, ownership model, or thread model;
- adding a major dependency;
- changing acceptance criteria.

Record the discovery and proposed resolution in the Issue. Do not let code and the approved Issue diverge.

## Verification

Use the relevant commands and platform-specific verification rules already defined in the root `AGENTS.md`. Do not duplicate that command catalog here.

Record honestly:

- commands run;
- successful checks;
- failed checks;
- checks not run and why;
- environments or backends not available;
- residual risk.

Passing compilation alone is not sufficient when the change requires tests, rust-analyzer verification, runtime behavior, or visual confirmation.

## Self-review

Before creating the Pull Request, inspect the complete diff and verify:

- every acceptance criterion is satisfied or explicitly reported as incomplete;
- implementation matches the approved design;
- no unrelated changes are present;
- tests cover important normal, boundary, and failure behavior;
- public API and documentation are consistent;
- error handling and unsafe assumptions are justified;
- generated files or lockfile changes are intentional.

## Transition to review

After implementation and verification:

1. Update the Issue acceptance checklist and add only a concise implementation status.
2. Put detailed changes, verification results, risks, and reviewer guidance in the Pull Request.
3. Create the Pull Request with `Closes #<issue-number>`.
4. Replace `phase:implementation` with `phase:review`.
5. Read `docs/agent-workflow/review.md`.
