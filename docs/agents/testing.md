# Testing & Verification Guidelines

Guidelines for AI agents verifying code changes in `elwindui`.

## Canonical Rust verification gate

This file is the canonical command authority for Rust verification. The gate applies to every
task that changes Rust source (`*.rs`), Cargo or build configuration that affects Rust
compilation, proc-macro or codegen behavior, or generated Rust API/output semantics.

Run the complete gate from the repository root before creating or updating a Pull Request, and
repeat it after any Rust-affecting review remediation:

1. Apply the repository formatter and keep its result in the working tree:

   ```text
   cargo fmt --all
   ```

   The mutating formatter result is repository source state and must be included in the
   implementation. A changed-file-only `rustfmt --check` is insufficient.

2. Verify the workspace formatter is clean and idempotent:

   ```text
   cargo fmt --all -- --check
   ```

3. Run actual rust-analyzer diagnostics, from the repository root:

   ```text
   rust-analyzer diagnostics .
   ```

   `cargo check` or `RUSTFLAGS="--cfg rust_analyzer" cargo check --workspace` is not a
   replacement for this command. The latter is an additive deterministic companion check when
   proc-macro, codegen, or rust-analyzer-shadow behavior changes.

4. Fix every rust-analyzer `Error`, `Warning`, and `WeakWarning` except for the one explicitly
   permitted diagnostic class below.

   The only permitted `WeakWarning` diagnostic is `Ra("inactive-code", WeakWarning)` when the
   code is inactive solely because of intentional repository `#[cfg(...)]` conditional
   compilation, including test, target, feature, and debug/release configuration branches.
   Any other `WeakWarning` is actionable and must be fixed. Do not generalize this exception to
   all `WeakWarning`.

   The completion condition is zero `Error`, zero `Warning`, and zero non-exempt `WeakWarning`
   diagnostics. Allowed `inactive-code` `WeakWarning` records must still be counted and
   reported.

5. The task is not verification-complete if either mandatory formatter command or actual
   rust-analyzer diagnostics is skipped or fails. If a required tool cannot run, report the task
   as unverified/blocked rather than treating the check as optional.

6. Record the exact command and result in the Pull Request verification report. Review-time
   Rust edits require rerunning this same complete gate; an earlier pass before remediation is
   not sufficient.

Do not manufacture a pass by disabling rust-analyzer diagnostics, hiding them in editor or
workspace settings, adding a repository-wide ignore list, adding blanket `#[allow(...)]`
attributes, downgrading diagnostic severity, or replacing actual rust-analyzer verification with
Cargo compilation.

## Verification execution context

Verification is either sandbox-safe or host-context live verification:

- Sandbox-safe verification includes formatting, `rust-analyzer diagnostics .`,
  `cargo check`, `cargo build`, and pure/unit/codegen tests that do not depend on native
  host-runtime semantics. These may run inside an agent sandbox.
- Host-context live verification includes native GUI startup, native OS
  package/runtime bootstrap, AppX/MSIX package registration or package-graph behavior,
  interactive desktop/window/input, native window lifecycle, and platform services whose
  behavior can be altered by process-token or sandboxing rules. These must run outside the
  agent sandbox for final acceptance.

For every required live command, record whether it ran in `host-context` or `sandbox`.
Sandbox-only live passes and failures are diagnostic evidence, not authoritative
platform-runtime evidence. Reproduce a sandbox failure outside the sandbox before
classifying it as a product defect. The final host-context run uses a normal,
non-elevated user by default. If host execution is impossible, classify the required live
verification as blocked. Do not weaken code or tests to make sandbox execution pass.

If a broad command such as `cargo test --workspace` includes any required host-context
live test on the current platform, its final acceptance run must also execute in
host-context.

## Other Cargo workspace commands

- `cargo build --workspace` 窶・Build all workspace crates and examples.
- `cargo check --workspace` 窶・Check all workspace crates and examples.
- `cargo test --workspace` 窶・Run tests across all workspace crates.
- `cargo run -p <example-name>` 窶・Run a specific example app from `examples/`.

When proc-macro, codegen, or rust-analyzer-shadow behavior changes, also run the additive
companion check:

```text
RUSTFLAGS="--cfg rust_analyzer" cargo check --workspace
```

## Visual & UI Verification

- Run relevant example apps under `examples/` to visually verify UI behavior.
- For AppKit UI verification on macOS, follow [`docs/agents/appkit.md`](appkit.md).
- For WinUI3 / Windows build & verification environment, follow [`docs/agents/winui3.md`](winui3.md).
