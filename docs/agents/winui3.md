# WinUI3 Backend & Windows Agent Guidelines

Guidelines for AI agents modifying `elwindui-backend-winui3` or building/testing on Windows.

## Related documents

- Architecture: [`docs/design/backends/winui3_backend_design.md`](../design/backends/winui3_backend_design.md)
- Backend state: [`docs/status/backend_status.md`](../status/backend_status.md)
- Control state: [`docs/status/control_status.md`](../status/control_status.md)
- Native E2E driver and tester procedure: [`winui3-e2e.md`](winui3-e2e.md)

## Windows Build Environment

Before running commands requiring MSVC or Windows SDK on Windows, import the environment in PowerShell:

```powershell
. .\tools\setup-vs-env.ps1            # x64 (same as -Arch x64)
. .\tools\setup-vs-env.ps1 -Arch arm64  # native ARM64 host only
```

`-Arch` accepts only `x64` and `arm64` and requires a native host of that architecture (no x86, ARM64EC, cross-compilation or emulation).

## Native build modes (prebuilt by default)

Normal `cargo build`/`check`/`test` links the checked-in `crates/elwindui-backend-winui3/native/prebuilt/<TARGET>/` artifacts (`x86_64-pc-windows-msvc`, `aarch64-pc-windows-msvc`) and runs no MIDL, C++/WinRT, `cl.exe` or makepri. It still needs the restored NuGet packages (`tools/restore-winui3.ps1`, also run by `setup-vs-env.ps1`) and the normal Rust MSVC linker. A missing target bundle fails the build; never work around that by falling back to source mode silently. See `docs/design/backends/winui3_backend_design.md`, "Native artifact manufacturing".

Source-native mode (maintainers changing `cpp/`, the IDL or the native ABI; requires Visual Studio C++ and Windows SDK tools):

```powershell
. .\tools\setup-vs-env.ps1 -Arch x64
$env:ELWINDUI_WINUI3_BUILD_NATIVE = "1"
$env:CARGO_TARGET_DIR = ".build/winui3-source-verify"
cargo build -p custom-controls-demo --target x86_64-pc-windows-msvc
```

`ELWINDUI_WINUI3_BUILD_NATIVE` accepts only unset, `0` or `1`. Use a separate `CARGO_TARGET_DIR` per mode. A source build never modifies `native/prebuilt/`; `ELWINDUI_WINUI3_PREBUILT_EXPORT_DIR` (source mode only) exports its outputs to `<dir>/<TARGET>/`. A single-architecture diagnostic manufacture is `.\tools\build-winui3-prebuilt.ps1 -Arch x64 -StagingRoot .build\winui3-native-staging`.

Any change to a native generation input (`build.rs`, `build_support.rs`, `cpp/`, the prebuilt scripts, `setup-vs-env.ps1`, `restore-winui3.ps1`, `.github/workflows/winui3-prebuilt.yml`) makes the tracked bundle stale; `.\tools\verify-winui3-prebuilt.ps1` then fails and the bundle must be regenerated. An incompatible native ABI change also bumps `elwindui_winui3_native_abi_v<N>` (bump rules in the design document).

### Regenerating the prebuilt bundle

Only GitHub Actions manufactures the bundle (no local ARM64 machine is needed or used): `.github/workflows/winui3-prebuilt.yml` builds x64 on `windows-2025-vs2026` and ARM64 on `windows-11-vs2026-arm`, assembles and verifies both, final-links `custom-controls-demo` for both in prebuilt mode, then uploads `winui3-prebuilt-verified`. On the default branch:

```powershell
gh workflow run winui3-prebuilt.yml --ref master
gh run list --workflow winui3-prebuilt.yml --limit 5
gh run download <run-id> -n winui3-prebuilt-verified -D .build/winui3-prebuilt-candidate
.\tools\verify-winui3-prebuilt.ps1 -Root .build/winui3-prebuilt-candidate
.\tools\promote-winui3-prebuilt.ps1 -BundleRoot .build/winui3-prebuilt-candidate
```

For a PR that changes native inputs (when the workflow change is not yet on `master`), use that PR's `pull_request` run instead: review its source SHA, workflow and scripts, download its `winui3-prebuilt-verified` artifact, promote, commit on the Issue branch, and let the workflow run again on the new head. Promotion verifies the candidate (CI provenance, hashes, ABI, current source-input digest), replaces the tracked tree transactionally and never commits or pushes. Inspect `git diff --stat` and the manifest before committing. Check the logs for `cl`/SDK tool architecture, `dumpbin` machine lines and artifact hashes, not just green jobs. CI compile/link on the ARM64 runner is not WinUI3 runtime acceptance.

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
