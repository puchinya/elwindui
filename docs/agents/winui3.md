# WinUI3 Backend & Windows Agent Guidelines

Guidelines for AI agents modifying `elwindui-backend-winui3` or building/testing on Windows.

## Related documents

- Architecture: [`docs/design/backends/winui3_backend_design.md`](../design/backends/winui3_backend_design.md)
- Backend state: [`docs/status/backend_status.md`](../status/backend_status.md)
- Control state: [`docs/status/control_status.md`](../status/control_status.md)

## Windows Build Environment

Before running commands requiring MSVC or Windows SDK on Windows, import the environment in PowerShell:

```powershell
. .\tools\setup-vs-env.ps1
```

## Sandbox boundary for WinUI3 live verification

The following commands must run outside the agent sandbox for final Windows acceptance
when they execute WinUI3 or Windows App SDK live paths:

```powershell
cargo test -p elwindui-backend-winui3
cargo test --workspace
cargo run -p <WinUI3 example>
```

This host-context requirement also applies to hosted XAML regression tests,
`elwindui::init()` / `MddBootstrapInitialize`, `Microsoft.UI.Xaml.Application`, Window
show/hide/close runtime tests, context-menu/popup/native-control runtime tests, the Issue
#178 pointer/coordinate runtime matrix, and other native GUI interaction or manual
verification.

Filtered pure WinUI3 unit tests may remain sandbox-safe when they are proven not to
initialize Windows App Runtime or WinUI, create native windows, use OS package services,
or depend on interactive desktop semantics.

These commands do not require host context merely because they target Windows:

```powershell
cargo check -p elwindui-backend-winui3
cargo build ...
rust-analyzer diagnostics .
```

Errors such as `MddBootstrapInitialize` `0x80070005`, AppX/DDLM access denied, missing
interactive desktop, or similar security-context failures observed inside an agent
sandbox must be rerun outside the sandbox before they are treated as WinUI3 product
defects. Final live evidence must record:

```text
execution context: host-context
user token: normal/non-elevated
```

unless the test specifically targets elevation.

## Command Execution on Windows

- Keep Windows commands short and execute one logical operation per command.
- Avoid long combined PowerShell pipelines.
- When searching generated bindings, use `rg -F -n -m 1 -A 13 "<pattern>" <file>` with ripgrep's `-m` flag.
- If a command stalls: cancel, retry once via direct method, and proceed without looping on hanging tools.

## C++/WinRT & NativeControl Constraints

- Maintain backend layering (`native_ui -> inner -> host -> render -> ffi`).
- WinUI3 C++/WinRT wrapper interactions must be isolated inside `inner` and `ffi.rs`.
