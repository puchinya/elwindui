# WinUI3 Backend & Windows Agent Guidelines

Guidelines for AI agents modifying `elwindui-backend-winui3` or building/testing on Windows.

## Related Status Documents

- [`docs/status/winui3_backend_status.md`](../status/winui3_backend_status.md) — WinUI3 build environment, C++/WinRT shim status, and pitfalls.
- [`docs/status/nativecontrol_status.md`](../status/nativecontrol_status.md) — NativeControl implementation status checklist across backends.

## Windows Build Environment

Before running commands requiring MSVC or Windows SDK on Windows, import the environment in PowerShell:

```powershell
. .\tools\setup-vs-env.ps1
```

## Command Execution on Windows

- Keep Windows commands short and execute one logical operation per command.
- Avoid long combined PowerShell pipelines.
- When searching generated bindings, use `rg -F -n -m 1 -A 13 "<pattern>" <file>` with ripgrep's `-m` flag.
- If a command stalls: cancel, retry once via direct method, and proceed without looping on hanging tools.

## C++/WinRT & NativeControl Constraints

- Maintain backend layering (`native_ui -> inner -> host -> render -> ffi`).
- WinUI3 C++/WinRT wrapper interactions must be isolated inside `inner` and `ffi.rs`.
