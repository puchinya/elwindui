# Review phase

Read this file only while the associated Issue is in `phase:review` or an associated Pull Request is open.

## Pull Request content

The Pull Request description must contain:

- purpose and user/developer impact;
- main changes;
- important implementation decisions;
- verification commands and results;
- untested platforms or configurations;
- compatibility and residual risks;
- focused reviewer guidance;
- `Closes #<issue-number>`.

Create and inspect the Pull Request, comments, reviews, labels, and Actions checks with `gh`.

The Issue remains the approved specification. The Pull Request describes the actual implementation and evidence.

## Review handling

1. Inspect all review submissions, inline threads, and required CI checks.
   Also verify the root `AGENTS.md` synchronization order: public changes have an approved spec, architecture changes have an updated design, current-state changes have an updated status, and no removed document path remains.
2. For each actionable comment:
   - implement the change;
   - explain why no change is appropriate;
   - or create a follow-up Issue when the work is valid but outside scope.
3. Re-run checks affected by review changes.
4. Do not resolve a review thread until the concern has been addressed or answered.
5. Keep unrelated follow-up work out of the current Pull Request.

If review requires a material requirements or design change:

1. Update the Issue.
2. Replace `phase:review` with `phase:design`.
3. Obtain approval for the revised design when required.
4. Return through implementation and verification before requesting review again.

## Completion

Do not close the Issue merely because the Pull Request is approved.

The work is complete only when:

- required reviews are approved;
- required CI checks pass;
- acceptance criteria are satisfied;
- required documentation is updated;
- the Pull Request is merged into the default branch.

`Closes #<issue-number>` should close the Issue automatically on merge. After merge, verify that the Issue is closed. If automatic closure did not occur, close it manually only after confirming the merge.

Create follow-up Issues for deferred work before declaring completion.
