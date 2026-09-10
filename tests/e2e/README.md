# End-to-end tests

`tests/e2e/` is the canonical repository location for durable product/application E2E cases.

## Ownership boundary

- Product/application E2E cases and their durable fixtures belong under `tests/e2e/`.
- Platform UI drivers remain under `tools/macos-ui-driver/` and `tools/windows-ui-driver/`.
- Driver self-tests remain colocated with the driver and are not product E2E cases.
- Temporary Issue investigation/evidence belongs under `.agent-state/issues/<issue>/`, not here.

## Cross-platform rule

When AppKit and WinUI3 verify the same backend-neutral behavior, use one shared scenario definition.
Do not maintain separate logically equivalent platform-specific test cases merely because the
underlying driver commands differ.

Platform-specific execution glue may differ where required, but product acceptance semantics and
expected results remain shared.

## Adding a durable E2E case

A new durable case must:

1. live under `tests/e2e/`;
2. identify the product behavior being accepted;
3. define setup, actions, expected postconditions, and cleanup;
4. define which backends are required / supported;
5. use PASS / FAIL / NOT RUN / BLOCKED semantics;
6. use repository platform drivers instead of introducing another UI automation stack;
7. keep raw run evidence out of the repository unless workflow explicitly selects a small durable
   reviewer-facing subset.

Do not add permanent product E2E cases under `tools/`.

## Current state

[`self-drawn-pointer-input.md`](self-drawn-pointer-input.md) is the first durable shared product
scenario definition here: self-drawn ElwindUI content (tab selection, grid-splitter drag, docking
drag) and a genuine native control both respond correctly to real OS pointer input. WinUI3 is
currently required for it (Issue #236's regression origin); AppKit already satisfies the same
semantics per existing status evidence and can consume this same definition through
[`docs/agents/appkit-e2e.md`](../../docs/agents/appkit-e2e.md) when a native run there is
scheduled. See [`docs/agents/winui3-e2e.md`](../../docs/agents/winui3-e2e.md) for the WinUI3
tester procedure.
