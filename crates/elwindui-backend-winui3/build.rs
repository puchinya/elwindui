//! Generates `src/bindings.rs` from the Windows App SDK's `Microsoft.UI.Xaml.winmd` metadata.
//!
//! UNVERIFIED / best-effort: written without a Windows machine to run this against, so treat the
//! exact `windows-bindgen` invocation below as a starting point, not a known-working setup —
//! `windows-bindgen`'s CLI-style argument list has changed shape across versions, and hasn't been
//! run here even once.
//!
//! The Windows App SDK isn't a plain Cargo dependency — its `.winmd` metadata ships inside the
//! `Microsoft.WindowsAppSDK` NuGet package, which needs a `nuget restore`/`dotnet restore` (e.g.
//! against a throwaway `.csproj`/`packages.config` referencing that package) before this can find
//! it. Point `WINDOWS_APP_SDK_WINMD` at the restored
//! `Microsoft.UI.Xaml.winmd` (typically under
//! `<nuget-packages>/microsoft.windowsappsdk/<version>/lib/win10-x64/Microsoft.UI.Xaml.winmd` —
//! the exact relative path has moved between Windows App SDK releases, so check the actual
//! restored package contents rather than trusting this literally) if the default guess below
//! doesn't resolve.
fn main() {
    // `windows-bindgen` is only a build-dependency on Windows (see Cargo.toml's target-specific
    // `[build-dependencies]`) — `src/lib.rs` itself is `#![cfg(target_os = "windows")]`, so its
    // `include!(env!("ELWINDUI_WINUI3_BINDINGS"))` line is stripped entirely on any other target
    // and never needs a real (or even placeholder) bindings file to exist.
    #[cfg(target_os = "windows")]
    generate_bindings();
}

#[cfg(target_os = "windows")]
fn generate_bindings() {
    println!("cargo:rerun-if-env-changed=WINDOWS_APP_SDK_WINMD");

    let winmd = std::env::var("WINDOWS_APP_SDK_WINMD").unwrap_or_else(|_| {
        let profile = std::env::var("USERPROFILE").expect("USERPROFILE must be set on Windows");
        format!(
            "{profile}\\.nuget\\packages\\microsoft.windowsappsdk\\1.6.240923002\\lib\\win10-x64\\Microsoft.UI.Xaml.winmd"
        )
    });

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR is set by cargo");
    let out_path = format!("{out_dir}/bindings.rs");

    // `--in default` pulls in the core Windows SDK metadata (Win32 types the WinUI3 APIs
    // reference, e.g. `HWND`/`HSTRING`'s underlying representation); `--in <winmd path>` adds the
    // Windows App SDK's own metadata on top. `--filter` limits generation to the namespaces this
    // backend actually uses — the full WinAppSDK surface is far larger than needed here.
    windows_bindgen::bindgen([
        "--in",
        "default",
        &winmd,
        "--out",
        &out_path,
        "--filter",
        "Microsoft.UI.Xaml",
        "Microsoft.UI.Dispatching",
        "Windows.Storage.Pickers",
        "Windows.Storage",
    ]);

    println!("cargo:rustc-env=ELWINDUI_WINUI3_BINDINGS={out_path}");
}
