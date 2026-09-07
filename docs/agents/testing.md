# Testing & Verification Guidelines

Guidelines for AI agents verifying code changes in `elwindui`.

## Iterative versus final verification

Verification has two execution scopes.

### Edit/debug loop

While the change is still being developed:

- run the narrowest relevant test/check first;
- do not repeatedly run the complete workspace/final gate merely as a progress probe;
- widen only when a focused failure or dependency impact requires it;
- do not suppress diagnostics or omit relevant failure evidence.

### Stable change / final gate

Once a Rust-affecting change is stable, run the complete canonical Rust gate below before Pull Request delivery.

For Rust-affecting review remediation, use focused checks during the remediation loop, then rerun the complete canonical gate once the remediation is stable. A pass obtained before the remediation is insufficient.

Focused iteration is a speed optimization only. It never replaces or weakens the final gate.

## Canonical Rust verification gate

This file is the sole command authority for Rust verification. The gate applies to every task that changes Rust source (`*.rs`), Cargo/build configuration affecting Rust compilation, proc-macro/codegen behavior, or generated Rust API/output semantics.

Run from repository root:

1. Apply and retain repository formatting:

   ```text
   cargo fmt --all
   ```

2. Verify formatter idempotence:

   ```text
   cargo fmt --all -- --check
   ```

3. Run actual rust-analyzer diagnostics:

   ```text
   rust-analyzer diagnostics .
   ```

   `cargo check` and `RUSTFLAGS="--cfg rust_analyzer" cargo check --workspace` do not replace this command.

4. Fix every rust-analyzer `Error`, `Warning`, and non-exempt `WeakWarning`.

   The only permitted `WeakWarning` is `Ra("inactive-code", WeakWarning)` when code is inactive solely because of intentional repository `#[cfg(...)]` conditional compilation, including test, target, feature, and debug/release branches.

   Completion requires:

   - zero `Error`;
   - zero `Warning`;
   - zero non-exempt `WeakWarning`.

   Allowed inactive-code records must still be counted/reported.

5. A mandatory formatter/analyzer step that is skipped, unavailable, or failed blocks verification completion.

6. Record exact commands/results in the PR. Stable Rust-affecting review remediation requires the same complete gate again.

Do not manufacture a pass by disabling diagnostics, hiding them in settings, adding repository-wide ignore lists, blanket `#[allow(...)]`, severity downgrades, or substituting Cargo compilation for actual rust-analyzer diagnostics.

## Verification execution context

Sandbox-safe verification includes formatting, `rust-analyzer diagnostics .`, `cargo check`, `cargo build`, and pure/unit/codegen tests independent of native host-runtime semantics.

Host-context live verification includes native GUI startup, native OS package/runtime bootstrap, AppX/MSIX registration/package graph, interactive desktop/window/input, native window lifecycle, and platform services whose behavior can be altered by sandbox/process-token rules.

Required host-semantic acceptance must run outside the agent sandbox. Sandbox-only native runtime passes/failures are diagnostic evidence, not final host acceptance. Reproduce sandbox failures in host context before classifying a product defect. Use the normal non-elevated host user unless the scenario explicitly requires elevation.

If host execution is unavailable, report the live gate as blocked rather than weakening acceptance.

If a broad command such as `cargo test --workspace` includes a required host-context live test on the current platform, its final acceptance run must also execute in host context.

## Other Cargo workspace commands

Use when relevant to the task/acceptance criteria:

- `cargo build --workspace`
- `cargo check --workspace`
- `cargo test --workspace`
- `cargo run -p <example-name>`

When proc-macro, codegen, or rust-analyzer-shadow behavior changes, also run:

```text
RUSTFLAGS="--cfg rust_analyzer" cargo check --workspace
```

## Visual & UI Verification

- Run relevant examples when UI behavior requires live/visual evidence.
- For AppKit use `docs/agents/appkit.md`; native AppKit E2E routes to `docs/agents/appkit-e2e.md`.
- For WinUI 3 / Windows use `docs/agents/winui3.md`; native WinUI3 E2E (process/window control,
  UIA, real input, screenshots) routes to `docs/agents/winui3-e2e.md`, not ad-hoc scripts.

## Durable E2E case ownership

Durable product/application E2E test cases belong under `tests/e2e/`. Platform drivers remain
under `tools/*-ui-driver/`. Driver contract/self-tests (e.g.
`tools/windows-ui-driver/tests/driver-contract.ps1`, `tools/macos-ui-driver`'s own tests) are not
moved to `tests/e2e/` -- they verify the driver itself, not product behavior.

`docs/agents/appkit-e2e.md` and `docs/agents/winui3-e2e.md` route native execution through their
respective platform driver, while the durable case a tester executes is selected from `tests/e2e/`.
Case definition ownership (`tests/e2e/`) is not the same thing as platform driver ownership
(`tools/*-ui-driver/`) -- do not add permanent product E2E scenarios to a driver's own directory.
