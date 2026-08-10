# Testing & Verification Guidelines

Guidelines for AI agents verifying code changes in `elwindui`.

## Cargo Workspace Commands

- `cargo build --workspace` — Build all workspace crates and examples.
- `cargo test --workspace` — Run tests across all workspace crates.
- `cargo run -p <example-name>` — Run a specific example app from `examples/`.

## IDE Verification with rust-analyzer

`cargo build` passing is not sufficient for proc-macro workspace verification. After code changes:

1. Run `rust-analyzer diagnostics .` from the repository root.
2. Fix actionable errors and lints (`unused_variables`, unnecessary code, etc.).
3. Ignore `"inactive-code"` diagnostics on `#[cfg(test)]` blocks (normal rust-analyzer behavior outside test analysis mode).

## Visual & UI Verification

- Run relevant example apps under `examples/` to visually verify UI behavior.
- For AppKit UI verification on macOS, follow [`docs/agents/appkit.md`](appkit.md).
- For WinUI3 / Windows build & verification environment, follow [`docs/agents/winui3.md`](winui3.md).
