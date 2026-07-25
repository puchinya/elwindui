//! WinUI 3 (Windows App SDK) implementation of the widget surface `elwindui-codegen` targets,
//! mirroring `elwindui-backend-appkit`'s shape (see that crate's doc comment for the overall
//! native-vs-virtual design this implements: `VerticalLayout`/`HorizontalLayout`/
//! `Rectangle`/`Ellipse`/`TextBlock` have no widget here at all, just `elwindui_core::ui::UIElement`
//! values `elwindui-codegen` builds directly (`TextBlock` is self-drawn, using the real XAML
//! `TextBlock` class only as a paint primitive inside `TreeHostPanel::relayout_static`, never as a
//! wrapped builtin widget — see `elwindui-backend-appkit`'s `CATextLayer` use for the same role);
//! only `Window`/`Button`/`TextArea`/`MenuBar`/`MenuBarItem`/`Menu`/`MenuItem`/`NativeTabView` are real
//! native widgets).
//!
//! Split into `inner` (private — raw WinRT/XAML plumbing, `Inner`-prefixed types) and `native_ui`
//! (public, re-exported here — implements every `elwindui_core::ui` builtin trait this backend
//! provides by composing the matching `inner` type). See each module's own doc comment — mirrors
//! `elwindui-backend-appkit`'s own split exactly.
//!
//! The WinUI projection is generated at build time from the Windows App SDK metadata. `build.rs`
//! resolves the metadata from `WINDOWS_APP_SDK_WINMD` or a normal NuGet package-cache install.

#![cfg(target_os = "windows")]
// `#[elwindui_macros::class]`'s `__elwindui_inherit_*!` chain mechanism needs a same-crate
// macro-to-macro reference (`$crate::the_macro!`) to also work cross-crate, which currently
// requires this lint disabled — see `crates/elwindui-macros/src/class.rs`'s own doc comment on
// `inherit_macro_self_ref_path` for the full explanation, and `docs/elwindui_macro_class_spec.md`.
// Every crate using `#[class]` with a same-crate `inherits` chain needs this same line —
// `elwindui-backend-appkit` carries the identical `#![allow(...)]` for the identical reason.
#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]




mod app;
mod bindings;
mod ffi;
mod host;
mod inner;
mod native_ui;
pub mod platform;
mod render;

pub use native_ui::*;

// `elwindui-codegen`'s generated code references `elwindui::backend::AnyView` directly (see
// `inner::AnyView`'s own doc comment), so it needs to stay reachable at this crate's own root even
// though the rest of `inner` is private.
pub use ffi::AnyView;

/// Initializes the Windows App SDK dynamic dependency for an unpackaged process.
///
/// Call this through [`elwindui::init`](https://docs.rs/elwindui) before constructing the first
/// WinUI 3 object. The operation is idempotent; the App SDK bootstrap remains active until process
/// exit, which is the lifetime required by WinUI 3 and Win2D objects.
pub fn init() -> windows::core::Result<()> {
    use std::sync::OnceLock;
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    use windows::core::{Error, HRESULT, PCWSTR, s, w};

    // COM apartments are thread-local, while the App SDK dynamic dependency is process-wide.
    // `init` deliberately keeps COM initialized for this UI thread until process exit; XAML must
    // subsequently be created on the same STA thread by `application::run`.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()? };

    static BOOTSTRAP: OnceLock<std::result::Result<(), HRESULT>> = OnceLock::new();
    let result = BOOTSTRAP.get_or_init(|| unsafe {
        let module = LoadLibraryW(w!("Microsoft.WindowsAppRuntime.Bootstrap.dll"))
            .map_err(|error| error.code())?;
        let proc = GetProcAddress(module, s!("MddBootstrapInitialize"))
            .ok_or_else(|| Error::from_thread().code())?;
        type BootstrapInitialize = unsafe extern "system" fn(u32, PCWSTR, u64) -> HRESULT;
        let initialize: BootstrapInitialize = std::mem::transmute(proc);
        // Windows App SDK 1.8, stable channel. A zero minimum version asks the bootstrapper for
        // the installed compatible package rather than pinning a patch release.
        let result = initialize((1 << 16) | 8, PCWSTR::null(), 0);
        result.ok().map_err(|error| error.code())
    });
    result.map_err(Error::from_hresult)
}

/// Re-exported so `elwindui`'s own facade can expose `application::run` uniformly across
/// backends. See `app`'s module doc.
pub mod application {
    pub use crate::app::run;
}
