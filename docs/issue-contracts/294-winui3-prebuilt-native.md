# ElwindUI WinUI3 Prebuilt Native Build — Implementation Contract

**Repository:** `puchinya/elwindui`  
**Planning baseline:** default branch HEAD `fa5201b55a018320480f0eb6ac7397c79a6bf7e0` (rechecked 2026-10-10)  
**Manufacturing decision:** GitHub Actions runs native Windows x64 and native Windows ARM64 jobs; no local ARM64 Windows machine is required for compilation/link validation.  
**Owning Issue:** [#294](https://github.com/puchinya/elwindui/issues/294) (`phase:requirements` at handoff). Before implementation, the agent must follow `AGENTS.md`, advance requirements/design as required, and save this contract with `scripts/agent/save-implementation-contract.*`.

> **Execution Rule**  
> This document is an Implementation Contract. Do not redesign the architecture during implementation. If a requirement conflicts with repository reality or requires a material design change, do not silently choose an alternative. Report the exact conflict and continue only with independent valid work.

## 1. Repository Baseline

### 1.1 Current build behavior

`crates/elwindui-backend-winui3/build.rs` currently performs all of the following on Windows on every relevant build:

1. Resolves Windows App SDK / Win2D metadata from the restored NuGet packages.
2. Runs `windows-bindgen` to produce the Rust WinRT projection consumed by `src/bindings.rs`.
3. Copies the Win2D runtime and Windows App Runtime bootstrap DLL beside built executables/tests.
4. Runs `makepri.exe` through `generate_resources_pri` to create `resources.pri`.
5. Runs `midl.exe` for `cpp/accessibility_semantic_peer.idl`.
6. Runs `cppwinrt.exe` to generate the C++/WinRT projection and accessibility component source.
7. Runs `cc::Build` to compile:
   - `cpp/app_host.cpp`
   - `cpp/accessibility_host.cpp`
   - generated `module.g.cpp`
   into `elwindui_winui3_app_host.lib`.
8. Links `WindowsApp`.

The required native toolchain is therefore broader than Rust itself: `cl.exe` through `cc`, `midl.exe`, `cppwinrt.exe`, and `makepri.exe` are all part of the current normal build path.

### 1.2 Relevant repository authority

The implementation must preserve the following existing facts and invariants.

| Reference | Relevant fact / invariant |
|---|---|
| `AGENTS.md` | Repository workflow is `specs -> design -> code -> status`; repository-changing tasks require Issue ownership, phase routing, reviewer checklist, branch helper, final PR, and review transition. |
| `docs/design/backends/winui3_backend_design.md`, **Projection and startup** | C++/WinRT `ApplicationT<App, IXamlMetadataProvider>` hosting is load-bearing. Replacing the C++ shim architecture is out of scope. |
| `docs/design/backends/winui3_backend_design.md`, **Accessibility projection** | Rust/Core remains the semantic authority. C++ owns only composable XAML/AutomationPeer mechanics and the narrow copied-value C ABI. |
| `crates/elwindui-backend-winui3/cpp/app_host.cpp` | The C++ boundary intentionally hosts WinUI `Application`, installs `XamlControlsResources`, and exposes narrow C ABI helpers. Window/control/layout/rendering remain Rust-owned. |
| `crates/elwindui-backend-winui3/cpp/accessibility_host.h/.cpp` | Accessibility crosses a C ABI using copied records/callback tables; C++ must not acquire Core ownership. |
| `crates/elwindui-backend-winui3/src/app.rs` | Rust calls `elwindui_winui3_run` through C ABI. Application lifetime/dispatcher behavior must not change. |
| `crates/elwindui-backend-winui3/src/host/accessibility.rs` | Rust owns the callback table and semantic state passed through the C ABI. Struct layout/signature changes are native ABI changes. |
| `crates/elwindui-backend-winui3/src/render/text.rs` | Rust calls C ABI ClearValue helpers implemented by the native shim. |
| `crates/elwindui-backend-winui3/src/bindings.rs` | Rust WinRT bindings are currently generated into `$OUT_DIR` by `windows-bindgen` and included with `include!`; they are not checked in. |
| `tools/setup-vs-env.ps1` | Imports Visual Studio C++ / Windows SDK and restores WinUI3 NuGet packages; it currently requires x64 tools and fixes `-arch=x64 -host_arch=x64`. Target-aware component discovery and ARM64 host setup are required. |
| `tools/restore-winui3.ps1` | Pins `Microsoft.WindowsAppSDK 1.8.260209005`, `Microsoft.WindowsAppSDK.WinUI 1.8.260204000`, and `Microsoft.Graphics.Win2D 1.4.0`. |
| `docs/agents/winui3.md` | Host-context execution is required for final native WinUI3 runtime acceptance; `cargo build`/`cargo check` alone are not runtime acceptance. |
| `docs/agents/testing.md` | Rust-affecting changes require the canonical formatter + rust-analyzer gate, with relevant workspace build/check/test commands in addition. |
| `docs/status/backend_status.md` | Current WinUI3 runtime/build state is substantial and already verified on real Windows hosts; this task must not regress established startup/accessibility/native-control behavior. |
| `examples/custom-controls-demo/Cargo.toml` | Existing `custom-controls-demo` package enables the WinUI3 facade; it is the required real executable link target for per-architecture CI. |
| GitHub-hosted Windows runner images | Official labels `windows-2025-vs2026` (x64) and `windows-11-vs2026-arm` (native ARM64) are available; the ARM64 VS2026 image includes ARM64 C++/Windows SDK workloads. Links in §2.14. |

### 1.3 Current specification impact

No public ElwindUI API or user-visible UI semantics need to change. `docs/specs/` therefore does **not** require a normative change unless implementation discovers that the build/distribution behavior is already part of a public specification. This task is primarily:

- internal build architecture;
- distribution/build reproducibility;
- developer workflow;
- verification/status documentation.

Do not modify a public specification solely to document an internal prebuilt/source-build switch.

### 1.4 Existing Issue/PR state relevant to planning

No dedicated prebuilt-native Issue/PR was found. Existing Windows Issues/PRs such as #222/#223 concern host-context verification rather than native artifact distribution. Do not attach this work to those Issues merely because they mention Windows SDK or build environment.

---

## 2. Architecture Decisions

### 2.1 Default mode is prebuilt native

A normal Windows build must use checked-in, target-specific prebuilt native artifacts.

```text
cargo build
cargo check
cargo test
```

must **not** invoke the source-native production pipeline (`midl.exe`, `cppwinrt.exe`, `makepri.exe`, or `cc::Build`/`cl.exe`) unless the developer explicitly selects source-native mode.

This applies both to downstream users and to developers building the crate from the repository. Merely having `cpp/` source files present must not switch behavior.

### 2.2 Source-native mode is explicit and environment-controlled

Use exactly this environment variable:

```text
ELWINDUI_WINUI3_BUILD_NATIVE
```

Semantics:

- unset: prebuilt mode;
- `0`: prebuilt mode;
- `1`: source-native mode;
- any other value: fail immediately with a clear build error naming the accepted values.

Do **not** use a Cargo feature for this mode selection. Cargo features are additive/unified across the dependency graph and are inappropriate for a local manufacturing decision that must not be accidentally enabled by a downstream dependency.

`build.rs` must emit:

```text
cargo:rerun-if-env-changed=ELWINDUI_WINUI3_BUILD_NATIVE
```

### 2.3 No automatic fallback

If a required prebuilt artifact is missing, the normal build must fail with an actionable message.

It must **not** silently fall back to source-native compilation. Silent fallback would reintroduce the exact user-facing VC++/SDK-tool requirement this task is intended to remove.

The error must identify:

- the exact `TARGET`;
- the missing file;
- that supported checked-in prebuilt artifacts are expected;
- the explicit source-build escape hatch: `ELWINDUI_WINUI3_BUILD_NATIVE=1` for ElwindUI maintainers.

### 2.4 Supported prebuilt target set

Provide prebuilt artifacts for exactly these MSVC Windows architectures:

```text
x86_64-pc-windows-msvc
aarch64-pc-windows-msvc
```

`i686-pc-windows-msvc` / x86 is explicitly unsupported and out of scope for this task.

Select the artifact directory from Cargo's exact `TARGET` string, not from host architecture and not from ad-hoc `x64`/`arm64` aliases.

Cross-building Windows from a non-Windows host is not added by this task. Build artifacts on native Windows GitHub-hosted runners, one runner architecture per target. The build script's Windows-host assumption remains unchanged.

### 2.5 Prebuilt artifact layout

Use this repository layout:

```text
crates/elwindui-backend-winui3/
  native/
    prebuilt/
      manifest.json
      x86_64-pc-windows-msvc/
        elwindui_winui3_app_host.lib
        Elwindui.WinUI3.Accessibility.winmd
        resources.pri
      aarch64-pc-windows-msvc/
        elwindui_winui3_app_host.lib
        Elwindui.WinUI3.Accessibility.winmd
        resources.pri
```

Keep the existing native library logical name `elwindui_winui3_app_host`; do not introduce an additional wrapper DLL or rename the C ABI as part of this work.

Per-target copies of `.winmd` and `resources.pri` are intentional even if outputs currently happen to be byte-identical. Do not create a new assumption that MIDL/makepri products are architecture-independent unless that is separately designed and approved.

### 2.6 Normal prebuilt build still owns Rust projection/runtime deployment

This task does **not** check in the generated Rust `windows-bindgen` projection.

In prebuilt mode, preserve these existing build steps:

- locate the pinned/restored Windows App SDK / Win2D metadata;
- run `windows-bindgen` and generate `$OUT_DIR/bindings.rs` and `$OUT_DIR/xaml_interop.rs` exactly as today;
- keep the existing post-processing of generated Rust bindings;
- run `copy_win2d_runtime` so the Win2D DLL and Windows App Runtime bootstrap DLL deployment behavior remains unchanged;
- set `ELWINDUI_WINUI3_BINDINGS` exactly as today.

The source-native/prebuilt split applies to the C++/MIDL/PRI production path, not to the Rust projection path or Microsoft runtime deployment in this task.

This is deliberate scope control: normal builds stop requiring ElwindUI's C++/MIDL/PRI tool invocation, while NuGet metadata/runtime handling remains the existing mechanism.

### 2.7 Prebuilt mode deployment behavior

In prebuilt mode:

1. select `native/prebuilt/<TARGET>/`;
2. emit a native link search path for that directory;
3. emit `cargo:rustc-link-lib=static=elwindui_winui3_app_host`;
4. emit `cargo:rustc-link-lib=WindowsApp` exactly as required by the existing native shim;
5. copy the prebuilt `resources.pri` to both existing destinations:
   - `target/<profile>/resources.pri`
   - `target/<profile>/deps/resources.pri`
6. copy the prebuilt `Elwindui.WinUI3.Accessibility.winmd` to both existing destinations:
   - `target/<profile>/Elwindui.WinUI3.Accessibility.winmd`
   - `target/<profile>/deps/Elwindui.WinUI3.Accessibility.winmd`
7. preserve existing Win2D/bootstrap DLL deployment.

No runtime filename or executable-adjacent lookup contract changes.

### 2.8 Source-native mode preserves the current native pipeline

With `ELWINDUI_WINUI3_BUILD_NATIVE=1`, preserve the current source pipeline:

```text
IDL
  -> midl.exe
  -> component .winmd
  -> cppwinrt.exe
  -> generated C++ component/projection
  -> cc::Build / MSVC C++
  -> elwindui_winui3_app_host.lib

makepri.exe
  -> resources.pri
```

Do not rewrite the WinUI application shim, accessibility architecture, or C++/WinRT component model merely to support prebuilt artifacts.

### 2.9 Common linking rule

`WindowsApp` is required by both source-native and prebuilt modes. Move/structure `build.rs` so `cargo:rustc-link-lib=WindowsApp` is emitted in a common Windows path rather than accidentally remaining reachable only from `build_cpp_app_host`.

Avoid duplicate or mode-dependent link ordering that changes current runtime behavior.

### 2.10 Native ABI generation anchor

Add a link-time ABI generation anchor.

C++ must export exactly:

```cpp
extern "C" __declspec(dllexport) void elwindui_winui3_native_abi_v1() noexcept {}
```

Rust must declare and reference that symbol from the normal WinUI application path so every linked application requires it to resolve. Place the Rust declaration beside the existing `elwindui_winui3_run` native boundary in `src/app.rs`; call the anchor before entering `elwindui_winui3_run`.

Purpose: a stale prebuilt library from an older native ABI generation must fail at link time rather than being silently accepted.

`v1` must be incremented whenever any of the following changes incompatibly:

- exported C function name/signature/calling convention;
- `ElwinduiAccessibilityNodeRecord` layout/meaning;
- `ElwinduiAccessibilityCallbacks` layout/signatures;
- native ABI assumptions used by Rust call sites.

Changing only internal Rust/C++ implementation without changing the boundary does not require an ABI generation bump.

Do not add a runtime version negotiation protocol or dynamic loading layer.

### 2.11 Prebuilt provenance manifest

`native/prebuilt/manifest.json` is manufacturing provenance, not runtime configuration. `build.rs` must not add a JSON parser solely for it.

Schema version 1 records one source revision/digest and **per-target** actual SDK/toolchain values. The two hosted images may use different installed SDK/compiler versions; do not falsely force equal toolchain versions. The pinned NuGet dependency versions must match for both targets.

```json
{
  "schema_version": 1,
  "native_abi": 1,
  "source_commit": "<full source checkout commit SHA>",
  "source_inputs_sha256": "<canonical source input hash>",
  "windows_app_sdk": "1.8.260209005",
  "windows_app_sdk_winui": "1.8.260204000",
  "win2d": "1.4.0",
  "targets": {
    "x86_64-pc-windows-msvc": {
      "runner_label": "windows-2025-vs2026",
      "windows_sdk_version": "<actual>",
      "msvc_version": "<actual>",
      "rustc_version": "<actual>",
      "lib_sha256": "...",
      "winmd_sha256": "...",
      "pri_sha256": "..."
    },
    "aarch64-pc-windows-msvc": {
      "runner_label": "windows-11-vs2026-arm",
      "windows_sdk_version": "<actual>",
      "msvc_version": "<actual>",
      "rustc_version": "<actual>",
      "lib_sha256": "...",
      "winmd_sha256": "...",
      "pri_sha256": "..."
    }
  }
}
```

Compute `source_inputs_sha256` deterministically across platforms using the lexicographically sorted repository-relative UTF-8 path and raw file bytes (SHA-256 per file), then SHA-256 over lines `<path> <hex>\n` encoded as UTF-8 (LF only). Include at minimum:

- `crates/elwindui-backend-winui3/build.rs`
- `crates/elwindui-backend-winui3/cpp/app_host.cpp`
- `crates/elwindui-backend-winui3/cpp/accessibility_host.cpp`
- `crates/elwindui-backend-winui3/cpp/accessibility_host.h`
- `crates/elwindui-backend-winui3/cpp/accessibility_semantic_peer.idl`
- `tools/restore-winui3.ps1`
- `tools/setup-vs-env.ps1`
- `tools/build-winui3-prebuilt.ps1`
- `tools/assemble-winui3-prebuilt.ps1`
- `.github/workflows/winui3-prebuilt.yml`

The hash must be identical in both CI jobs for the same checkout. Treat a change in the build script as stale until regeneration, even if only some build-script content is unrelated to native output. `source_commit` identifies the manufacturing checkout; later committing the binary bundle can change HEAD without making the recorded source-input hash invalid. Validate that hash and pinned dependencies, not HEAD string equality. Never mix target artifacts generated from different source-input hashes or source commits in one bundle.

### 2.12 Source-build export is explicit

Normal source-native builds must **not** modify checked-in `native/prebuilt/` files.

Add a second environment variable, honored only when source-native mode is active:

```text
ELWINDUI_WINUI3_PREBUILT_EXPORT_DIR
```

When set, `build.rs` copies the three produced native artifacts for the current `TARGET` into:

```text
<ELWINDUI_WINUI3_PREBUILT_EXPORT_DIR>/<TARGET>/
```

When unset, source-native output remains only in normal Cargo build output locations.

Using `ELWINDUI_WINUI3_PREBUILT_EXPORT_DIR` while `ELWINDUI_WINUI3_BUILD_NATIVE` is not `1` must fail with a clear error. This prevents treating already-checked-in prebuilt files as newly manufactured outputs.

### 2.13 Target-local manufacturing, CI assembly, and promotion

The supported production path is a **GitHub Actions two-target matrix followed by one aggregator and two consumer-link jobs**. Do not require an ARM64 Windows machine owned by the developer, and do not replace the ARM64 runner with x64 cross-compilation.

Add the following scripts with fixed responsibilities:

- `tools/build-winui3-prebuilt.ps1 -Arch x64|arm64 -StagingRoot <path>`: build **only that architecture** from source, export to `<path>/<TARGET>/`, emit one per-target provenance fragment containing target, source commit/input hash, dependency pins, actual SDK/MSVC/Rust versions and artifact SHA-256. It must never update tracked `native/prebuilt/`. It must verify native host architecture and the selected Rust triple before compiling.
- `tools/assemble-winui3-prebuilt.ps1 -InputRoot <path> -OutputRoot <path>`: require exactly one artifact+fragment for each target; reject duplicates, missing files, source-commit/input-hash mismatch, dependency-version mismatch, architecture mismatch, or corrupt hash; assemble the §2.5 directory layout and the single §2.11 manifest; invoke full verification. Do not copy or mutate repository-tracked artifacts.
- `tools/promote-winui3-prebuilt.ps1 -BundleRoot <path>`: accept **only** a fully verified bundle whose source-input digest matches the current checkout; stage in the tracked tree's parent filesystem, safeguard the old tree, and replace/restore it transactionally. On any error, restore the previous tree; never leave a successful half-updated bundle. Do not push, commit, or create a PR automatically.

A local maintainer may use these scripts to manufacture one architecture for diagnosis, but **a complete release bundle requires both targets from the same CI source commit and the aggregator**. No single-host both-target build is required. Inspect/check the resulting diff and generate the owning implementation PR through the repository workflow.

### 2.14 Architecture-aware Visual Studio environment and hosted runners

Extend `tools/setup-vs-env.ps1` with an optional `-Arch` parameter. Dot sourcing and existing no-argument x64 behavior remain compatible:

```powershell
. .\tools\setup-vs-env.ps1
. .\tools\setup-vs-env.ps1 -Arch x64
. .\tools\setup-vs-env.ps1 -Arch arm64
```

| Target | `runs-on` label | Required native environment |
|---|---|---|
| `x86_64-pc-windows-msvc` | `windows-2025-vs2026` | `-arch=x64 -host_arch=x64`; component `Microsoft.VisualStudio.Component.VC.Tools.x86.x64` |
| `aarch64-pc-windows-msvc` | `windows-11-vs2026-arm` | `-arch=arm64 -host_arch=arm64`; component `Microsoft.VisualStudio.Component.VC.Tools.ARM64` |

Select `vswhere -requires` using the corresponding target component, instead of the existing unconditional `Microsoft.VisualStudio.Component.VC.Tools.x86.x64`. On native ARM64 runner require an ARM64 OS/process/compiler and ARM64-hosted SDK tools; do not claim success from x64 emulation or switch silently to x64 cross-compilation. Reject `x86`, ARM64EC and any other `-Arch` value.

The setup script must retain its `VsDevCmd.bat` environment import, `WindowsSdkDir`/`WindowsSDKVersion` reporting, and `tools/restore-winui3.ps1` invocation. The source build must fail with the exact missing tool/component when ARM64 runner does not provide a required `midl.exe`, `cppwinrt.exe`, or `makepri.exe`; do not silently use tools or NuGet assets from a different architecture.

Runner labels are deliberate, not `windows-latest`/`windows-11-arm`, because the VS2026 environment is a manufacturing input. Image software versions may change despite the label; record actual versions and verify on each run. GitHub announced Windows 11 ARM64 + VS2026 GA on 2026-08-20: <https://github.blog/changelog/2026-08-20-windows-11-arm64-vs2026-image-generally-available/>. Official runner labels: <https://docs.github.com/en/actions/reference/runners/github-hosted-runners>.

### 2.14a GitHub Actions workflow contract

Create `.github/workflows/winui3-prebuilt.yml` (one workflow) with triggers `workflow_dispatch` (canonical regeneration after the workflow is present on the default branch) and `pull_request` scoped to native input, generation/verification scripts, `Cargo.toml`/lockfile as applicable, prebuilt artifacts, and this workflow. Use `permissions: { contents: read }`; no repository write tokens/secrets, automatic commit, package publication, or deployment. Do not promote artifacts produced by untrusted PR code without verifying a trusted source checkout and hashes.

Required jobs in order:

1. **`manufacture` (matrix, fail-fast false)**: on a PR explicitly checkout `github.event.pull_request.head.sha` rather than the synthetic merge commit; on dispatch checkout `github.sha`; propagate this exact immutable source SHA to every job; use x64/ARM64 runner/target mapping above. Install/verify the matching Rust MSVC target, dot-source `setup-vs-env.ps1 -Arch ...`, assert `rustc -vV` host/target and `cl` target architecture and SDK tools, run target-local script in source mode with isolated `CARGO_TARGET_DIR`, perform final source-mode link of `custom-controls-demo`, verify produced `.lib` COFF machine using `dumpbin /headers` or equivalent, then upload uniquely named `winui3-native-x64` / `winui3-native-arm64` artifacts (do not let two jobs upload the same name). Include provenance fragments. CLI success without emitted files/COFF architecture verification is not PASS.
2. **`assemble` (`needs: manufacture`)**: run on x64 Windows, check out the **same source SHA**, download both separately named artifacts without flattened path collisions, validate fragment source SHA/digest and artifact hashes, assemble exactly two target directories, generate complete manifest, run `verify-winui3-prebuilt.ps1 -Root <candidate-root>`, and upload `winui3-prebuilt-candidate`. Any failure prevents bundle acceptance.
3. **`consumer-link` (matrix, `needs: assemble`, fail-fast false)**: each Windows runner checks out the **same source SHA**, downloads the candidate bundle, places it into `crates/elwindui-backend-winui3/native/prebuilt/` only in the disposable checkout, unsets both manufacturing environment variables, verifies the candidate, and runs `cargo build -p custom-controls-demo --target <TARGET>` with isolated `CARGO_TARGET_DIR`. Assert no `midl.exe`, `cppwinrt.exe`, `makepri.exe`, or `cl.exe` invocation by ElwindUI `build.rs` (linker/SDK import-library usage may remain); check the linked binary architecture and required executable-adjacent `.pri/.winmd`/runtime DLL deployment. `cargo check` alone cannot fulfill this job.
4. **`publish-verified-bundle` (`needs: [assemble, consumer-link]`)**: only after both consumer-link jobs pass, explicitly download the previously checked candidate into this new job, verify its full manifest/content again, and upload the immutable final Actions artifact `winui3-prebuilt-verified`. Its provenance includes source SHA, run URL/ID, per-target hashes and successful job identity. This is an Actions download artifact, **not** a crates.io release or automatic git commit.

Make `manufacture`/`assemble`/`consumer-link` independent of an interactive Windows desktop. CI may run non-GUI unit tests, but do not equate a hosted-runner process, non-interactive `cargo test`, or a successful link with WinUI3 interactive runtime acceptance. **Initial bootstrap exception:** `workflow_dispatch` cannot be relied upon for a brand-new workflow file until it exists on the default branch. For the first implementation PR, generate via that PR's `pull_request` event, review the exact PR-head source SHA/workflow/scripts and the generated artifacts, then manually promote only the `winui3-prebuilt-verified` artifact from the reviewed run onto the same Issue branch. Re-run CI after the artifact commit; verify source-input digest and two-target link again. This reviewed, explicit first-PR promotion is permitted, but no automatic promotion of PR artifacts or blind acceptance of arbitrary fork artifacts is allowed. After merge, canonical regeneration uses `workflow_dispatch` on the approved default branch.

After downloading the approved verified bundle, promote into the repository on the Issue-scoped branch, inspect binaries/licenses/package size, commit and open/update the PR per `AGENTS.md`. Never add a workflow capable of using PR-supplied code with repository write permissions or secrets.

### 2.15 Prebuilt verification script

Add:

```text
tools/verify-winui3-prebuilt.ps1
```

It must accept an optional root directory; default is the tracked `crates/elwindui-backend-winui3/native/prebuilt` tree.

It must fail if any of the following is true:

- one of the two supported target directories is missing;
- any expected `.lib`, `.winmd`, or `.pri` file is missing or zero-length;
- manifest schema/native ABI does not match the repository's expected generation;
- artifact SHA-256 differs from the manifest;
- `source_inputs_sha256` differs from current native generation inputs when checking the tracked tree;
- a target is missing from the manifest;
- an unexpected target directory exists that the script would otherwise publish silently.

In manufacturing and consumer-link CI jobs, require PE/COFF machine inspection (`dumpbin /headers` or equivalent) that proves x64 library/executable is AMD64 and ARM64 library/executable is ARM64. Manufacturing must also inspect the `elwindui_winui3_native_abi_v1` symbol in the `.lib`. No `dumpbin`/equivalent is required for an ordinary downstream prebuilt `cargo build`; this inspection is a manufacturing/review obligation.

The verifier must accept an explicit candidate root and must not assume `native/prebuilt` files have already been promoted to the working tree. Its source-digest check must use the workflow's checked-out source files and reject mixed CI runs.

### 2.16 Toolchain requirement boundary

This task removes the following from the **ordinary ElwindUI native-generation path**:

- C++ source compilation for the WinUI shim;
- C++/WinRT projection generation;
- MIDL generation;
- PRI generation.

It does **not** claim that the standard Rust `*-pc-windows-msvc` target can build a final Windows executable without the normal MSVC linker / Windows import-library environment required by Rust/MSVC itself.

Do not bundle Microsoft Windows SDK import libraries, Visual Studio tool binaries, or Microsoft redistributable files into the crate as a workaround.

### 2.17 Crate package-size boundary

Checked-in prebuilt libraries materially affect distribution size. The current Cargo Book documents a 10 MB crates.io `.crate` size limit.

If this backend is packaged for crates.io during this task, measure the actual `.crate` archive after both supported architectures are included. If the package exceeds the registry limit, **stop and return to design**. Do not silently invent target-specific companion crates, remote downloads, Git LFS runtime fetching, or binary compression/extraction during `build.rs`.

If current workspace publication is already blocked for unrelated reasons (for example unpublished path-dependency metadata), record that existing blocker and do not broaden this task into a workspace-wide publishing redesign.

### 2.18 Rejected alternatives

The following are explicitly rejected for this task:

- Cargo feature such as `build-native` for selecting source/native manufacturing;
- auto-detection based on whether Visual Studio or SDK tools happen to be installed;
- fallback from missing prebuilt artifact to native source compilation;
- automatically rebuilding native artifacts merely because the repository contains C++ sources;
- checking generated prebuilt files into the repository during every ordinary `cargo build`;
- replacing the C++/WinRT `ApplicationT` architecture with pure Rust;
- moving accessibility semantic ownership into C++;
- converting the native shim to a DLL/dynamic-loader architecture;
- shipping Microsoft SDK `.lib` files or SDK executables inside the crate;
- checking in the `windows-bindgen` Rust projection as part of this task;
- redesigning Windows App SDK / Win2D runtime deployment;
- adding non-MSVC Windows support;
- splitting the prebuilt payload into new target-specific crates without a new design decision.

---

## 3. Exact Change Set

### 3.1 `crates/elwindui-backend-winui3/build.rs`

Refactor the Windows `main` path into explicit common/prebuilt/source phases without changing generated Rust binding semantics.

Required conceptual order:

```text
resolve build mode
validate mode-specific env
resolve app-sdk metadata
run windows-bindgen + existing Rust binding post-processing
copy Win2D / Windows App Runtime bootstrap runtime
emit common WindowsApp link
if prebuilt:
    select native/prebuilt/<TARGET>
    link prebuilt static library
    deploy prebuilt resources.pri
    deploy prebuilt accessibility winmd
else source-native:
    generate resources.pri
    build C++/WinRT native host
    deploy generated accessibility winmd
    optionally export generated artifacts to staging
emit ELWINDUI_WINUI3_BINDINGS
```

Required function-level separation (names may vary only cosmetically; responsibilities may not be merged back into one opaque block):

- pure mode parser;
- pure supported-target/prebuilt-path resolver;
- prebuilt linker/deployer;
- source-native builder (existing `build_cpp_app_host` logic retained);
- common runtime artifact deployment helper(s);
- optional source-build export helper.

`find_sdk_tool`, `find_sdk_union_metadata_dir`, `find_sdk_reference_winmd`, `find_makepri`, `generate_resources_pri`, and C++ compilation must only be reachable from source-native mode.

`copy_win2d_runtime` remains common.

Refactor `generate_resources_pri` and `build_cpp_app_host` as needed so the generated PRI, WinMD, and native `.lib` paths are available to the explicit export step. Do not discover outputs by recursively scanning arbitrary `target/` directories.

### 3.2 `crates/elwindui-backend-winui3/build_support.rs` — new

Add a small dependency-free module containing only pure build-decision logic that can be unit-tested outside Cargo build-script execution, including:

- `NativeBuildMode { Prebuilt, Source }`;
- parse function for `ELWINDUI_WINUI3_BUILD_NATIVE` values;
- supported target validation/path mapping;
- constants for native ABI generation and supported target triples if this avoids duplication.

`build.rs` imports this module directly.

Do not put process launching, file copying, or environment mutation in this module.

### 3.3 `crates/elwindui-backend-winui3/tests/build_support.rs` — new

Reuse the pure build-support module by path and test the exact mode/target contract.

Required tests are listed in §7.

### 3.4 `crates/elwindui-backend-winui3/cpp/app_host.cpp`

Add the ABI v1 anchor only. Do not change startup/resource semantics.

### 3.5 `crates/elwindui-backend-winui3/src/app.rs`

Declare `elwindui_winui3_native_abi_v1` beside `elwindui_winui3_run` and reference it before calling the native run entry point.

No other application lifecycle change.

### 3.6 `crates/elwindui-backend-winui3/native/prebuilt/**` — new

Add exactly the layout in §2.5 plus the provenance manifest.

These are tracked binary release artifacts. Do not add Microsoft SDK libraries to this tree.

### 3.7 `tools/setup-vs-env.ps1`

Implement target+host architecture selection and architecture-specific Visual Studio component discovery per §2.14, while preserving the x64 no-argument path.

### 3.8 `tools/build-winui3-prebuilt.ps1` — new

Implement one-target source-native generation/staged export and provenance fragment per §2.13. A CI matrix job invokes this script **once for its own target**; no local ARM64 machine is assumed.

### 3.9 Additional manufacturing/validation tooling — new

- `tools/assemble-winui3-prebuilt.ps1`: merge the two immutable target artifacts into one candidate bundle, verifying digest/source identity and files before writing the manifest.
- `tools/promote-winui3-prebuilt.ps1`: transactionally promote only a complete verified bundle into the tracked `native/prebuilt/` tree; no implicit git commit/push.
- `tools/verify-winui3-prebuilt.ps1`: validate completeness, hashes, ABI/provenance, target COFF architecture in CI, and source-input hash.
- `.github/workflows/winui3-prebuilt.yml`: two-target manufacture matrix, one assembly job, two-target prebuilt consumer-link matrix, final verified artifact upload; exact runners and permissions in §2.14a.

### 3.10 Documentation

Update only responsibility-bearing documents:

- `docs/design/backends/winui3_backend_design.md`
  - document default prebuilt/native-source manufacturing split;
  - preserve the C++/WinRT architectural boundary;
  - document ABI-generation anchor and artifact ownership.
- `docs/agents/winui3.md`
  - normal build uses prebuilt native artifacts;
  - exact source-native command and toolchain precondition;
  - exact GitHub Actions workflow dispatch/download/promotion commands, runner labels and no-local-ARM64 requirement;
  - exact source-native commands and verification commands.
- `docs/status/backend_status.md`
  - concise verified CI build/link state per target and separately identified host-runtime gaps;
  - do not add raw logs/history.

`docs/agents/windows.md` changes are not required unless an existing sentence becomes factually false. Its current statement is conditional (“commands that require MSVC or Windows SDK”), so do not duplicate WinUI3-specific mode instructions there without need.

### 3.11 `Cargo.toml`

Keep the existing Windows build-dependencies (`windows-bindgen`, `cc`) unless implementation proves a technical need to change them. `cc` remains necessary for the explicit source-native mode even though it is not invoked in normal prebuilt mode.

Do not add a build dependency merely to parse `manifest.json`.

---

## 4. Implementation Sequence

Perform implementation in this dependency order.

1. **Workflow bootstrap**
   - locate/create owning Issue per `AGENTS.md`;
   - complete requirements/design approval as repository workflow requires;
   - save this contract using `save-implementation-contract.*`;
   - prepare the effective reviewer checklist before editing.

2. **Extract pure build-mode logic**
   - add `build_support.rs`;
   - add unit tests for mode parsing and target mapping;
   - keep existing build behavior active until tests pass.

3. **Add ABI generation anchor**
   - add `elwindui_winui3_native_abi_v1` in C++;
   - reference it from Rust application startup;
   - verify source-native mode still links/runs before introducing prebuilt selection.

4. **Separate source-native and prebuilt paths in `build.rs`**
   - make current source pipeline explicitly reachable only for mode `Source`;
   - add prebuilt link/deploy path;
   - keep `windows-bindgen` and runtime copy common;
   - ensure `WindowsApp` linking is common.

5. **Add architecture-aware setup + export support**
   - extend `setup-vs-env.ps1` with native x64/ARM64 host and component discovery;
   - add per-target source-native export staging environment handling;
   - ensure ordinary source build does not modify tracked artifacts.

6. **Add CI manufacturing and verification tooling**
   - implement one-target producer with a provenance fragment and no tracked changes;
   - implement aggregator that validates distinct target artifacts and consistent source identity;
   - implement candidate verifier and transactional local promotion tool.

7. **Add the GitHub Actions workflow**
   - configure `windows-2025-vs2026` for x64 and `windows-11-vs2026-arm` for ARM64;
   - matrix-source-build both, then aggregate, consumer-link both, upload final verified bundle;
   - explicitly prevent secrets, auto-commit, untrusted-PR promotion, and GUI runtime claims.

8. **Manufacture and promote the initial bundle**
   - for the first implementation PR (workflow not yet on default), obtain a verified bundle through `pull_request`, manually inspect and approve its exact source/workflow SHA, then promote it into that PR branch;
   - after the workflow lands on default, use `workflow_dispatch` for subsequent canonical regeneration;
   - download the corresponding final verified artifact and ensure its checked-out source-input digest matches;
   - promote into tracked `native/prebuilt` via script, then commit the reviewed output on the Issue branch;
   - record actual per-target Windows SDK/MSVC/Rust versions and artifact hashes.

9. **Validate both consumer and source-native modes**
   - use both GitHub Actions native architectures for source builds and clean prebuilt final links;
   - run the existing x64 WinUI3 hosted regression in both modes on a normal Windows host;
   - do **not** claim ARM64 interactive GUI testing from Actions compile/link alone;
   - verify negative cases and package payload size.

10. **Update design/agent/status documentation** only after behavior is verified.

11. **Run complete repository verification and self-review**, commit, push, create PR, and advance Issue to review.

---

## 5. Required Runtime and Build Semantics

### 5.1 Normal path

For a supported Windows MSVC target with prebuilt artifacts present:

- normal Cargo build selects prebuilt mode without requiring an opt-in flag;
- source C++/IDL/PRI generation does not execute;
- Rust WinRT binding generation remains current behavior;
- runtime DLL deployment remains current behavior;
- prebuilt `.winmd` and `.pri` are deployed to the same executable/test locations as generated outputs today;
- the static native library resolves all existing C ABI symbols plus the ABI anchor;
- application startup, XamlControlsResources installation, accessibility provider creation, text-style clearing, Window lifetime, and native event behavior are unchanged.

### 5.2 Explicit source-native path

When `ELWINDUI_WINUI3_BUILD_NATIVE=1`:

- current SDK/native tools are required;
- missing `cppwinrt.exe`, `midl.exe`, `makepri.exe`, compiler, metadata, or required SDK inputs fail as source-build failures;
- prebuilt `.lib/.winmd/.pri` are not linked or copied;
- generated native outputs are used for that build;
- no tracked prebuilt artifact is modified unless `ELWINDUI_WINUI3_PREBUILT_EXPORT_DIR` is explicitly supplied.

### 5.3 Repeated builds

- Changing Rust source unrelated to the native ABI must not regenerate native C++ artifacts in prebuilt mode.
- Re-running normal Cargo builds may re-run `windows-bindgen` according to existing Cargo/build-script invalidation, but must not execute native manufacturing.
- Source-native mode follows existing `rerun-if-changed` inputs and rebuilds as necessary.

### 5.4 Missing/stale prebuilt artifacts

- Missing target/artifact: deterministic build failure; no fallback.
- Stale source hash: `verify-winui3-prebuilt.ps1` failure and release/self-review blocker.
- Wrong ABI generation: verification failure and/or unresolved ABI anchor at link time.
- Artifact hash mismatch: verification failure; do not regenerate only one file by hand.

### 5.5 Failure and cleanup semantics

This task must not change native runtime failure behavior:

- `XamlControlsResources` setup failure remains fatal as today;
- C++ HRESULT-to-Rust/native helper behavior remains unchanged;
- accessibility late-query/teardown semantics remain unchanged;
- application/window lifecycle and `Application::Exit` behavior remain unchanged.

The manufacturing script must clean or discard staging on failed generation. It must never leave a partially refreshed tracked bundle that appears valid.

### 5.6 Reentrancy and ownership

No change to:

- `ApplicationT` ownership;
- Rust startup callback ownership;
- accessibility callback context lifetime;
- `WINDOWS` registry lifetime authority;
- Core semantic state ownership;
- native control ownership or render/host layering.

### 5.7 Debug vs release consumer builds

Prebuilt native artifacts are manufactured with a release native build and are used by both Rust debug and release consumer builds. The native ABI contains no C++ object/STL ownership transfer across the Rust boundary; only C-compatible primitive/pointer/callback records cross it.

Do not add separate debug prebuilt artifacts unless a demonstrated ABI/runtime problem requires a return to design.

---

## 6. Non-goals / Forbidden Changes

Do not perform any of the following in this task:

- change public ElwindUI Rust API or DSL;
- change WinUI visual/input/layout behavior;
- replace Windows App SDK or Win2D versions except when separately approved;
- redesign C++/WinRT `ApplicationT` hosting;
- move WinUI application hosting back to pure Rust;
- redesign accessibility semantics, pattern behavior, or C ABI ownership;
- check in generated Rust `windows-bindgen` output;
- remove the `cc` build dependency solely because default mode does not call it;
- create target-specific crates to avoid package size without returning to design;
- download native binaries from the network during `build.rs`;
- invoke PowerShell/download scripts automatically from downstream consumer builds;
- silently install Visual Studio, Windows SDK, Rust targets, or NuGet packages;
- bundle Microsoft SDK `.lib`, compiler tools, or SDK tools into `native/prebuilt`;
- change MSVC/GNU target support policy;
- add non-Windows cross-build support;
- weaken host-context runtime acceptance requirements;
- change unrelated CI, packaging, or workspace publication metadata; only the scoped `.github/workflows/winui3-prebuilt.yml` is authorized.

---

## 7. Concrete Tests

### T1 — mode defaults to prebuilt

**Setup:** call the pure parser with no environment value.  
**Action:** resolve build mode.  
**Expected:** `Prebuilt`.

### T2 — explicit zero is prebuilt

**Setup:** parser input `"0"`.  
**Expected:** `Prebuilt`.

### T3 — explicit one is source-native

**Setup:** parser input `"1"`.  
**Expected:** `Source`.

### T4 — invalid mode is rejected

**Setup:** parser inputs such as `"true"`, `"source"`, `"2"`, and empty-but-present if represented distinctly.  
**Expected:** deterministic error naming `ELWINDUI_WINUI3_BUILD_NATIVE` and accepted values.

### T5 — supported target mapping

For each:

```text
x86_64-pc-windows-msvc
aarch64-pc-windows-msvc
```

**Expected:** exact `native/prebuilt/<TARGET>/` resolution with no host-architecture substitution.

### T6 — unsupported target mapping

**Setup:** an unsupported triple such as `x86_64-pc-windows-gnu`.  
**Expected:** deterministic unsupported-prebuilt-target result; no fallback directory and no silent source build.

### T7 — prebuilt verifier completeness

**Setup:** tracked valid bundle.  
**Action:** `tools/verify-winui3-prebuilt.ps1`.  
**Expected:** PASS for both supported targets, all files non-empty, manifest complete, hashes match, source-input hash current.

### T8 — prebuilt verifier detects tampering

**Setup:** copy bundle to a temporary directory and modify one byte/delete one artifact.  
**Action:** verifier against temporary copy.  
**Expected:** FAIL identifying the target/artifact/hash mismatch. The tracked bundle remains untouched.

### T9 — source-input staleness is detected

**Setup:** use temporary copies or a test fixture so one native input differs from the manifest source-input hash.  
**Expected:** verifier rejects the stale bundle rather than treating artifact hashes alone as sufficient.

### T10 — default prebuilt compile/link

**Windows setup:** isolated `CARGO_TARGET_DIR`; `ELWINDUI_WINUI3_BUILD_NATIVE` unset.  
**Action:** build a real WinUI3 executable target (prefer an existing repository demo) and the backend crate.  
**Expected:** successful final link through checked-in `.lib`; prebuilt `.winmd` and `.pri` exist beside executable where current runtime expects them.

The test must not be satisfied by `cargo check` alone because that may not exercise the final native link.

### T11 — source-native compile/link

**Windows setup:** import VS environment; isolated source target directory; set `ELWINDUI_WINUI3_BUILD_NATIVE=1`.  
**Action:** build the same executable target.  
**Expected:** current `midl/cppwinrt/makepri/cc` pipeline succeeds and the executable links without consuming tracked prebuilt files.

### T12 — export is source-only

**Case A:** set `ELWINDUI_WINUI3_PREBUILT_EXPORT_DIR` without source mode.  
**Expected:** clear failure.

**Case B:** source mode + export directory.  
**Expected:** exactly one target subdirectory is populated with `.lib`, `.winmd`, `.pri`; repository tracked bundle is unchanged.

### T13 — two-runner consistency, promotion transactionality

**Setup:** manufacture x64 and ARM64 from the same source commit using the required GitHub Actions runners; separately inject one missing/corrupted/mismatched architecture fragment into a temporary aggregation attempt.  
**Expected:** aggregator rejects mismatch and never writes to tracked `native/prebuilt/`. Different source commits or source-input digests cannot be merged; valid fragments may record different actual SDK/compiler versions.

**Promotion fault test:** simulate failure during the local promotion stage.  
**Expected:** previously tracked bundle is restored byte-for-byte, and no partially updated bundle is considered successful. A successful promotion changes both targets and one manifest as one reviewed unit.

### T14 — ABI anchor stale-library failure

**Setup:** use a temporary prebuilt directory containing an old/test library without `elwindui_winui3_native_abi_v1`, while the Rust side requires it.  
**Action:** final link.  
**Expected:** unresolved native ABI anchor/link failure; never runtime execution with an unversioned stale library.

### T15 — x64 host-runtime equivalence

Run the established WinUI3 hosted regression in **host context** against the prebuilt build and source-native build separately, using isolated target directories. Prefer the existing hosted test named in status documentation:

```text
hosted_button_text_and_window_lifecycle_regressions_work
```

**Expected:** both modes PASS with equivalent startup, native control, and window lifetime behavior.

Do not claim ARM64 runtime verification unless actually executed on a suitable ARM64 host. Its required acceptance for this task is successful manufacturing/link validation, with runtime state reported honestly.

### T16 — resources/accessibility deployment

For both prebuilt and source-native x64 builds:

**Expected beside the actual executable and relevant `deps/` test location:**

```text
resources.pri
Elwindui.WinUI3.Accessibility.winmd
Microsoft.Graphics.Canvas.dll
Microsoft.WindowsAppRuntime.Bootstrap.dll
```

The first two must originate from the selected native mode; the latter two preserve the existing NuGet/runtime deployment path.

### T17 — package payload inspection

If backend crate packaging is currently supported:

```text
cargo package -p elwindui-backend-winui3 --list
cargo package -p elwindui-backend-winui3 --no-verify
```

Verify all required prebuilt artifacts are included and measure the actual `.crate` size against the registry limit. If workspace publication metadata/path dependencies already prevent packaging, record the existing blocker and do not widen this change into a workspace-wide redesign.

### T18 — native ARM64 GitHub Actions manufacture and link

**Setup:** `manufacture` job on exact `windows-11-vs2026-arm` (not x64 cross-compile), targeting `aarch64-pc-windows-msvc`.  
**Action:** run source-native generation, COFF inspection, assembly, then default prebuilt `cargo build -p custom-controls-demo --target aarch64-pc-windows-msvc` in `consumer-link`.  
**Expected:** ARM64 native `cl`, required SDK tools, produced `.lib` and executable are ARM64; artifact has correct hashes; source and consumer paths both final-link successfully. Do not classify absence of GUI/interactive desktop as a successful ARM64 runtime test.

### T19 — CI artifact mixing is rejected

**Setup:** provide x64 artifact from one workflow run and ARM64 artifact from a different source commit, or duplicate one target fragment.  
**Action:** run aggregator.  
**Expected:** explicit rejection before manifest creation; tracked binary tree unchanged.

### T20 — trusted approval and promotion

**Setup:** dispatch a canonical workflow on the approved default branch when already installed; for initial bootstrap, run the new workflow on its implementation PR. Download the approved `winui3-prebuilt-verified` artifact; separately test an unreviewed PR artifact and a manipulated candidate.  
**Expected:** canonical artifact or explicitly reviewed first-PR verified artifact with matching source-input digest can be promoted by a maintainer through the local script. Untrusted PR artifacts cannot automatically publish/commit/replace tracked assets; manufacturing receives no repository write permission or secrets. The first-PR path includes a CI rerun after binary promotion.

### T21 — x64 runner compatibility

**Setup:** matrix x64 job on `windows-2025-vs2026`, target `x86_64-pc-windows-msvc`.  
**Expected:** existing `setup-vs-env.ps1` default x64 flow and explicit `-Arch x64` both locate x64 tools and can compile/link the existing demo. `-Arch arm64` on ARM64 runner selects native ARM64 host tools; `-Arch x86` must fail.

## 8. Verification

Follow `docs/agents/testing.md` as the command authority.

### 8.1 Focused implementation checks

At minimum while iterating:

```text
cargo test -p elwindui-backend-winui3 --test build_support
```

On Windows, run both build modes in isolated target directories. PowerShell examples:

```powershell
Remove-Item Env:ELWINDUI_WINUI3_BUILD_NATIVE -ErrorAction SilentlyContinue
$env:CARGO_TARGET_DIR = ".build/winui3-prebuilt-verify"
cargo build -p <existing-winui3-demo> --target x86_64-pc-windows-msvc
```

Source-native:

```powershell
. .\tools\setup-vs-env.ps1 -Arch x64
$env:ELWINDUI_WINUI3_BUILD_NATIVE = "1"
$env:CARGO_TARGET_DIR = ".build/winui3-source-verify"
cargo build -p <same-existing-winui3-demo> --target x86_64-pc-windows-msvc
```

Use the actual existing demo package selected during implementation; do not create a throwaway application if an existing WinUI3 example already provides a real final link.

### 8.2 Prebuilt tooling and negative verification

On Windows (or via the new Actions workflow):

```powershell
.\tools\verify-winui3-prebuilt.ps1
cargo test -p elwindui-backend-winui3 --test build_support
```

Run the T8/T9/T13/T19/T20 negative cases against temporary artifacts; ensure repository-tracked content remains unchanged on failure.

### 8.3 GitHub Actions manufacturing and consumer-link verification

For subsequent regeneration after the workflow is installed on the approved default branch, dispatch it through Actions UI or (where authenticated host-context `gh` is available):

```powershell
gh workflow run winui3-prebuilt.yml --ref <approved-default-branch>
gh run list --workflow winui3-prebuilt.yml --limit 5
# Once an exact successful run ID is known:
gh run download <run-id> -n winui3-prebuilt-verified -D .build/winui3-prebuilt-candidate
.\tools\verify-winui3-prebuilt.ps1 -Root .build/winui3-prebuilt-candidate
.\tools\promote-winui3-prebuilt.ps1 -BundleRoot .build/winui3-prebuilt-candidate
```

For the initial bootstrap PR, use its automatic `pull_request` run instead of the default-branch `workflow_dispatch`; download and manually review its final verified artifact, then rerun the PR checks after committing the binary bundle. The exact default branch must be discovered from repository state; `<...>` is not a value to paste literally. Actions must report individually:

- x64 source-native build/link on `windows-2025-vs2026`;
- ARM64 source-native build/link on `windows-11-vs2026-arm`;
- both provenance fragments and artifact integrity merged on a single checked-out SHA;
- x64 default prebuilt final link and deploy;
- ARM64 default prebuilt final link and deploy;
- final artifact generated only after both consumer-link jobs PASS.

Inspect `cl` host/target, SDK tool presence, `dumpbin` machine fields and per-target artifact hashes in logs, not merely green job statuses. Use `actions/upload-artifact@v4` and `actions/download-artifact@v4` (unique per-target artifact names). Do not use a single upload artifact name concurrently from both matrix jobs. Source/binary bundle promotion remains an explicit human-reviewed repository change, not a CI write-back.

### 8.4 Canonical Rust final gate

Run from repository root after the Rust/build changes are stable:

```text
cargo fmt --all
cargo fmt --all -- --check
rust-analyzer diagnostics .
```

Also run the relevant workspace commands required by this task:

```text
cargo check --workspace
cargo build --workspace
cargo test --workspace
```

If proc-macro/codegen/rust-analyzer-shadow behavior is not changed, the extra `RUSTFLAGS="--cfg rust_analyzer" cargo check --workspace` command is not introduced solely by this task; follow `docs/agents/testing.md` if another changed area triggers it.

On Windows, `cargo test --workspace` or hosted WinUI tests that execute Windows App Runtime / XAML are final acceptance evidence only when run in host context as required by `docs/agents/winui3.md`.

### 8.5 Runtime acceptance

At least x64 must have real host-context native runtime evidence after switching to prebuilt by default. The accepted evidence must exercise a real WinUI3 `Application` and native control/window path, not only a pure unit test.

ARM64 must be reported as:

- native ARM64 GitHub Actions source compile + link PASS/FAIL, separately;
- native ARM64 GitHub Actions prebuilt consumer final link + deploy PASS/FAIL, separately;
- interactive WinUI3 runtime **UNVERIFIED** unless actually executed in a suitable interactive ARM64 host context.

Even when GitHub-hosted ARM64 is a real ARM64 Windows VM, non-interactive CI compile/link does not satisfy the repository's GUI host-context E2E requirements.

Do not infer runtime PASS from x64.

---

## 9. Reviewer Checklist

The implementation agent must self-review every item as `PASS`, `FAIL`, or `N/A` with concrete evidence before PR delivery.

- [ ] **R1 — Default build is prebuilt.** Unset/`0` mode never invokes `midl`, `cppwinrt`, `makepri`, or C++ compilation logic.
- [ ] **R2 — Source build is explicit.** Only exact `ELWINDUI_WINUI3_BUILD_NATIVE=1` selects the current native source pipeline; invalid values fail clearly.
- [ ] **R3 — No silent fallback.** Missing/unsupported prebuilt artifacts fail without source compilation.
- [ ] **R4 — Two MSVC targets are complete.** x64 and ARM64 each contain `.lib`, accessibility `.winmd`, and `resources.pri`, and manifest hashes match.
- [ ] **R5 — Runtime deployment paths are preserved.** PRI/WinMD are copied to the same executable/test locations as before; Win2D/bootstrap behavior is unchanged.
- [ ] **R6 — Rust projection behavior is preserved.** `windows-bindgen`, generated binding post-processing, and `ELWINDUI_WINUI3_BINDINGS` remain semantically unchanged.
- [ ] **R7 — Native architecture is preserved.** `ApplicationT`, accessibility ownership, host/render/backend layering, callbacks, and runtime failure behavior are not redesigned.
- [ ] **R8 — ABI stale-artifact protection exists.** `elwindui_winui3_native_abi_v1` is exported, referenced by Rust, verified in manufacturing, and documented with bump rules.
- [ ] **R9 — Source build never mutates tracked prebuilt output implicitly.** Export requires explicit staging variable/source mode.
- [ ] **R10 — CI manufacturing/aggregation and local promotion are transactional.** Failed target/aggregation cannot change tracked binaries; failed promotion restores the former bundle.
- [ ] **R11 — Provenance is complete.** Manifest records source commit/input hash, dependency versions, per-target actual SDK/MSVC/Rust versions, targets/hashes and ABI/schema versions.
- [ ] **R12 — Licensing/distribution boundary is preserved.** No Microsoft SDK import libraries/tools are checked into the prebuilt tree.
- [ ] **R13 — Target selection uses `TARGET`.** No host-architecture substitution or accidental cross-target artifact reuse.
- [ ] **R14 — x64 prebuilt and source-native runtime equivalence is verified in host context.** Both execute the established native regression successfully.
- [ ] **R15 — ARM64 native CI verification is real and claims are honest.** `windows-11-vs2026-arm` manufacture and prebuilt consumer-link pass with correct machine type; interactive GUI remains unverified absent host E2E.
- [ ] **R16 — Package-size/distribution risk is checked when package assembly is available.** Any >10 MB crates.io payload returns to design rather than inventing a workaround.
- [ ] **R17 — Documentation responsibilities are synchronized.** WinUI3 design, agent instructions, and status reflect the implemented behavior; public specs remain unchanged unless a real public contract change was discovered.
- [ ] **R18 — Canonical repository verification passes.** Formatter, rust-analyzer, relevant workspace build/check/test, and required host-context tests are all recorded against final committed HEAD.
- [ ] **R19 — No unrelated refactor/dependency change.** `cc` remains available for source mode; no new JSON/build helper dependency is added just for manifest parsing.
- [ ] **R20 — Delivery gate is complete.** Changes committed and pushed, self-review validator passes, PR exists with `Closes #<issue>`, Issue is `phase:review`, review workflow entered.
- [ ] **R21 — GitHub Actions uses native architecture and exact runners.** x64 = `windows-2025-vs2026`, ARM64 = `windows-11-vs2026-arm`; both matrix source builds and both prebuilt consumer links PASS, with unique per-target artifacts.
- [ ] **R22 — CI supply-chain separation.** Jobs have read-only contents permission, no secrets/auto-push/publishing, verified fragment consistency; initial bootstrap uses explicitly reviewed PR artifact plus post-promotion rerun, subsequent regeneration uses trusted default-branch dispatch.
- [ ] **R23 — Windows environment setup works on both hosts.** VS component discovery, `VsDevCmd` host/target selection, and SDK tool checks are architecture-correct; default x64 remains compatible.

Paste-safe canonical checklist block:

```text
ELWINDUI_REVIEWER_CHECKLIST_V1_BEGIN
REVIEW_ITEM: R1 — Default build is prebuilt; unset/0 mode does not execute native source-generation tools.
REVIEW_ITEM: R2 — Only ELWINDUI_WINUI3_BUILD_NATIVE=1 selects source-native mode; invalid values fail.
REVIEW_ITEM: R3 — Missing/unsupported prebuilt artifacts fail without source-build fallback.
REVIEW_ITEM: R4 — x86_64/aarch64 MSVC prebuilt bundles are complete and manifest hashes match; i686/x86 is unsupported.
REVIEW_ITEM: R5 — PRI/WinMD executable+deps deployment and Win2D/bootstrap deployment remain correct.
REVIEW_ITEM: R6 — windows-bindgen Rust projection behavior remains unchanged.
REVIEW_ITEM: R7 — ApplicationT/accessibility/backend ownership and lifecycle architecture remain unchanged.
REVIEW_ITEM: R8 — Native ABI v1 link anchor exists, is referenced, verified, and has documented bump rules.
REVIEW_ITEM: R9 — Source builds do not modify tracked prebuilt artifacts without explicit export staging.
REVIEW_ITEM: R10 — CI aggregation and local promotion are transactional; any failure leaves or restores tracked bundle unchanged.
REVIEW_ITEM: R11 — Manifest records source commit/input hash/pinned dependencies and per-target SDK/MSVC/Rust versions/hashes/ABI.
REVIEW_ITEM: R12 — No Microsoft SDK libraries or SDK/compiler tools are bundled as project prebuilt artifacts.
REVIEW_ITEM: R13 — Prebuilt selection uses Cargo TARGET exactly and cannot reuse host-architecture artifacts.
REVIEW_ITEM: R14 — x64 prebuilt and source-native established WinUI3 regression both pass in host context.
REVIEW_ITEM: R15 — Native ARM64 VS2026 runner source and consumer final-link verification pass; interactive runtime is not inferred.
REVIEW_ITEM: R16 — Package payload size is checked when package assembly is available; oversize returns to design.
REVIEW_ITEM: R17 — WinUI3 design/agent/status docs are synchronized without unnecessary public spec changes.
REVIEW_ITEM: R18 — Canonical Rust/workspace/host-context final verification passes on final committed HEAD.
REVIEW_ITEM: R19 — No unrelated refactor/dependency change; cc remains for source mode and no manifest parser dep added.
REVIEW_ITEM: R20 — Commit/push/self-review/PR Closes/phase:review/review-entry delivery gate is complete.
REVIEW_ITEM: R21 — Exact x64 and ARM64 GitHub Actions native runners execute source manufacture and prebuilt consumer links with separate uploaded artifacts.
REVIEW_ITEM: R22 — CI read-only with no auto-publish/secrets; initial PR artifact requires explicit source-reviewed promotion and post-promotion CI rerun.
REVIEW_ITEM: R23 — setup-vs-env selects native x64/ARM64 VS components, host arch and SDK tools; default x64 remains compatible.
ELWINDUI_REVIEWER_CHECKLIST_V1_END
```

---

## 10. Completion Report

The implementation is not complete until the repository delivery gate is complete.

The final report must contain:

### Files changed

List every changed source/build/tool/workflow/document path and all added binary artifact paths. Group generated/prebuilt binary files by target rather than omitting them.

### Build-mode results

Record separately:

```text
Prebuilt default mode: PASS/FAIL
Source-native x64 mode: PASS/FAIL
x86_64 prebuilt generation: PASS/FAIL
ARM64 prebuilt generation on windows-11-vs2026-arm: PASS/FAIL
ARM64 prebuilt consumer link on windows-11-vs2026-arm: PASS/FAIL
x64 prebuilt consumer link on windows-2025-vs2026: PASS/FAIL
GitHub Actions artifact assembly/integrity: PASS/FAIL
Prebuilt verifier: PASS/FAIL
```

### Runtime results

Record actual host-context execution separately by architecture and by prebuilt/source mode. Include GitHub Actions run URL, runner labels, source checkout SHA, and ARM64 compiled/link-only qualification. Do not combine compile and runtime evidence into one PASS.

### Reviewer Checklist

Include PASS/FAIL/N/A counts and the validated checklist SHA/source required by repository workflow. Any FAIL or unjustified N/A blocks delivery.

### Verification commands

List exact commands executed and outcomes, including:

- `cargo fmt --all`
- `cargo fmt --all -- --check`
- `rust-analyzer diagnostics .`
- relevant workspace `cargo check/build/test`
- `tools/verify-winui3-prebuilt.ps1`
- `tools/build-winui3-prebuilt.ps1 -Arch <x64|arm64> -StagingRoot <path>`
- `tools/assemble-winui3-prebuilt.ps1` and `tools/promote-winui3-prebuilt.ps1`
- GitHub Actions workflow run URL and per-target job conclusions
- host-context WinUI3 regression commands
- package payload inspection if available.

### Unverified platforms/configurations

Explicitly state any supported architecture not runtime-tested. Do not infer ARM64 runtime behavior from x64.

### Contract deviations/conflicts

State `None` or list each deviation, why it was necessary, and the approved decision that replaced this contract. A silent deviation is not allowed.

### Remaining work

State `None` or concrete follow-up Issues. Do not hide package-size/publication blockers.

### Pull Request URL

The report must end with the actual Pull Request URL. A commit or pushed branch without a PR is **blocked**, not completed.

The PR body must contain:

```text
Closes #<issue-number>
```

and the owning Issue must be advanced to `phase:review` according to repository workflow before implementation completion is reported.