# Review phase

Read this file only while the associated Issue is in `phase:review` or an associated Pull Request is open.

The Issue remains the approved specification. The PR reports the actual implementation delta and evidence.

## Delta-oriented Pull Request content

Use this compact structure:

```text
## Purpose / impact
## Delta
## Design deviations
## Verification
## Untested / residual risk
## Reviewer focus
Closes #<issue-number>
```

Rules:

- do not restate the full approved requirements/design/Implementation Contract;
- reference the owning Issue/design and repeat only context needed to understand the delta;
- list actual changed behavior/files at a useful level, not an investigation transcript;
- record exact verification commands/results;
- state deviations/conflicts explicitly, or `None`;
- keep reviewer guidance focused on high-risk changed behavior;
- keep raw logs and long evidence out of the PR body.

Implementer self-review is evidence, not reviewer approval. The reviewer independently inspects the actual diff, Issue/spec/design/contract authority, comments, tests, and required checks. A concise self-review summary may guide focus, but a claimed `PASS` does not change review classification.

Use `gh` to inspect PR comments/reviews/threads/checks.

## Review handling

1. Inspect all actionable review submissions, inline threads, and required CI checks.
2. Also verify document synchronization: approved public changes have specs, durable architecture changes have design, current-state changes have concise status, and Agent paths/commands are current.
3. For each actionable comment:
   - implement it;
   - explain why no change is appropriate; or
   - create a follow-up Issue for valid out-of-scope work.
4. Use focused checks while remediation is in progress.
5. If remediation changes Rust-affecting files or generation semantics, once remediation is stable rerun the complete mandatory Rust verification gate in `docs/agents/testing.md`.
6. After repository-changing remediation, commit the remediation, inspect the complete diff again, redo the generic and task-specific self-review on the new HEAD, update `Reviewed-HEAD`, and rerun `validate-self-review.* <issue-number>` before considering the PR ready for another review cycle.
7. A previous self-review is stale after any new commit.
8. Do not resolve a thread until addressed/answered.
9. Keep unrelated follow-up work out of the PR.

If review requires a material requirements/design change, update the Issue, return to `phase:design`, obtain required approval, and come back through implementation/verification.

## Contract-aware review

When the task began from a supplied Implementation Contract and the local mirror is available:

- validate its integrity with `scripts/agent/agent-context.* <issue-number>`;
- re-read the exact mirror before classifying a suspected contract violation;
- repository authority still wins when the contract conflicts with approved specs/design/Issue decisions.

Do not review from a remembered or compressed paraphrase when the exact mirror exists.

## Completion

Do not close the Issue merely because the PR is approved.

Work is complete only when:

- required reviews are approved;
- required checks pass;
- acceptance criteria are satisfied;
- required documentation is synchronized;
- PR is merged into the default branch.

`Closes #<issue-number>` should close the Issue on merge. Verify closure after merge and create follow-up Issues for deferred work before declaring completion.
