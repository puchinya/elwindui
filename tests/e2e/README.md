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

## Shared case compilation contract

The durable backend-neutral case is the source for a future deterministic compiler and reusable
plan. Declared case, runner, backend, driver, and compiler dependencies determine plan
invalidation; an unrelated repository HEAD change does not invalidate an otherwise unchanged
plan. A cached plan permits reuse of plan structure only: every run reacquires runtime identifiers
such as PID, window identity, geometry, DPI/monitor state, and AX/UIA element identity, and creates
fresh immutable evidence bound to the tested HEAD and runtime environment. Visual parameters and
assertions must use explicit bounded checkpoints rather than implicit image rediscovery. The
architecture and current-vs-planned boundary are defined in
[`native_e2e_orchestration_design.md`](../../docs/design/tools/native_e2e_orchestration_design.md).

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

No durable case is defined here yet. `tools/windows-ui-driver/tests/theme-demo-e2e.ps1` (a
Windows-only, product-specific smoke script) was removed rather than kept as a placeholder --
future durable cases are designed so AppKit and WinUI3 can consume the same scenario definition,
which a Windows-only script cannot represent. See
[`docs/agents/winui3-e2e.md`](../../docs/agents/winui3-e2e.md) and
[`docs/agents/appkit-e2e.md`](../../docs/agents/appkit-e2e.md) for the platform tester procedures a
future case here will be executed through.
