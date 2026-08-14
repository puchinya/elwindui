//! WinUI 3 backend — the concrete widget surface `elwindui-codegen` targets on Windows.
//! See docs/design/backends/winui3_backend_design.md.
//!
//! Layering — dependencies run one way only, `native_ui -> inner -> host -> render -> ffi`:
//!
//! | module      | owns |
//! |-------------|------|
//! | `native_ui` | the public façade: one `#[class]` per builtin, implementing the matching
//! |             | `elwindui_core::ui` `*Ext` trait by delegating to its `inner` twin |
//! | `inner`     | raw per-control plumbing, `Inner`-prefixed |
//! | `host`      | the tree host view: layout/render driving, native event -> core input |
//! | `render`    | drawing only — knows nothing about `UIElement`, focus or any control |
//! | `ffi`       | the toolkit seam: the erased native handle (`AnyView`) |
//! | `app`       | dispatcher, app delegate, event-loop entry |
//! | `platform`  | OS services that are not UI elements (file dialogs) |
//! | `bindings`  | the generated WinRT projection (`windows-bindgen` output) |
//!
//! `elwindui-backend-appkit` mirrors this file-for-file; keep the two in step. This crate is
//! `#![cfg(target_os = "windows")]`; current build and runtime verification is recorded in
//! docs/status/backend_status.md.

#![cfg(target_os = "windows")]
// `#[elwindui_macros::class]`'s `__elwindui_inherit_*!` chain mechanism needs a same-crate
// macro-to-macro reference (`$crate::the_macro!`) to also work cross-crate, which currently
// requires this lint disabled — see `crates/elwindui-macros/src/class.rs`'s own doc comment on
// `inherit_macro_self_ref_path` for the full explanation, and `docs/specs/macro_class_spec.md`.
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

// `windows-bindgen` emits references to XAML interop types through `crate::Windows`.
// Keep that compatibility namespace at the crate root even though the generated
// projection itself lives in the private `bindings` module.
#[allow(unused_imports)]
pub(crate) use bindings::Windows;

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

    // Registers this crate's `elwindui_core::graphics::TextBackend` — see
    // `elwindui-backend-appkit::lib.rs::init()`'s identical registration and
    // `docs/design/runtime/text_design.md` for why this needs to happen before any `TextBlock`
    // measurement or `NativeControl::sync_text_style` call.
    elwindui_core::graphics::set_text_backend(std::rc::Rc::new(render::WinUi3TextBackend));

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
