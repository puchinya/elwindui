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
//! # UNVERIFIED — read before touching
//!
//! Written entirely without a Windows machine available in this environment to build or run it
//! against. The `elwindui_backend_appkit`-mirroring *structure* (which types exist, what methods
//! they expose, how `TreeHostPanel` reflects an `Rc<dyn UIElement>`) is deliberate and should be
//! sound;
//! the *exact* WinRT/`windows-rs` call shapes (event-handler registration syntax, exact property/
//! method names on `Microsoft.UI.Xaml` types, `build.rs`'s bindgen invocation) are written from
//! memory of the general `windows-rs` WinRT-projection pattern and are the most likely things to
//! need correction once this is actually compiled on Windows with the Windows App SDK installed.

#![cfg(target_os = "windows")]
// `#[elwindui_macros::class]`'s `__elwindui_inherit_*!` chain mechanism needs a same-crate
// macro-to-macro reference (`$crate::the_macro!`) to also work cross-crate, which currently
// requires this lint disabled — see `crates/elwindui-macros/src/class.rs`'s own doc comment on
// `inherit_macro_self_ref_path` for the full explanation, and `docs/elwindui_macro_class_spec.md`.
// Every crate using `#[class]` with a same-crate `inherits` chain needs this same line —
// `elwindui-backend-appkit` carries the identical `#![allow(...)]` for the identical reason.
#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

#[allow(non_snake_case, non_camel_case_types, dead_code, clippy::all)]
mod bindings {
    include!(env!("ELWINDUI_WINUI3_BINDINGS"));
}

mod inner;
mod native_ui;

pub use native_ui::*;

// `elwindui-codegen`'s generated code references `elwindui::backend::AnyView` directly (see
// `inner::AnyView`'s own doc comment), so it needs to stay reachable at this crate's own root even
// though the rest of `inner` is private.
pub use inner::AnyView;

/// See docs/elwindui_spec.md 付録T.2 — same async-shaped-but-synchronous-underneath API as
/// AppKit's `platform::file_dialog` (`IFileOpenDialog`/`IFileSaveDialog::Show` block the calling
/// thread until the user closes the dialog; there's no genuine suspend point). Uses the classic
/// Win32 common file dialog COM interfaces (`Win32_UI_Shell` — present in the mainstream `windows`
/// crate) rather than the WinRT `Windows.Storage.Pickers` pickers, since those need
/// `IInitializeWithWindow` interop to attach to a non-UWP top-level `HWND`, which is extra
/// complexity this skips in favor of a path more likely to actually compile as written.
pub mod platform {
    pub mod file_dialog {
        use std::path::PathBuf;
        use windows::core::Interface;
        use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
        use windows::Win32::UI::Shell::{FileOpenDialog, FileSaveDialog, IFileOpenDialog, IFileSaveDialog, SIGDN_FILESYSPATH};

        fn ensure_com_initialized() {
            unsafe {
                // Ignore the result: `RPC_E_CHANGED_MODE`/`S_FALSE` both mean COM is already
                // initialized on this thread (fine — this only ever runs on the UI thread), and
                // any other failure surfaces later as the dialog itself failing to create.
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            }
        }

        pub async fn open() -> Option<PathBuf> {
            ensure_com_initialized();
            unsafe {
                let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
                dialog.Show(None).ok()?;
                let item = dialog.GetResult().ok()?;
                let path = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
                Some(PathBuf::from(path.to_string().ok()?))
            }
        }

        pub async fn save() -> Option<PathBuf> {
            ensure_com_initialized();
            unsafe {
                let dialog: IFileSaveDialog = CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER).ok()?;
                dialog.Show(None).ok()?;
                let item = dialog.GetResult().ok()?;
                let path = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
                Some(PathBuf::from(path.to_string().ok()?))
            }
        }
    }
}

/// WinUI3's `Dispatcher` (docs/elwindui_spec.md 付録P.5): hops back to the UI thread via the
/// current thread's `DispatcherQueue` — the WinUI3/WinAppSDK analog of AppKit's
/// `dispatch2::DispatchQueue::main()`. `application::run()` (below) is what pumps this queue as
/// part of its own message loop, so a job enqueued from any thread is guaranteed to run promptly.
pub struct WinUI3Dispatcher {
    queue: bindings::Microsoft::UI::Dispatching::DispatcherQueue,
}

impl elwindui_core::task::Dispatcher for WinUI3Dispatcher {
    fn enqueue(&self, job: Box<dyn FnOnce() + Send + 'static>) {
        let job = std::cell::RefCell::new(Some(job));
        let _ = self.queue.TryEnqueue(&bindings::Microsoft::UI::Dispatching::DispatcherQueueHandler::new(move || {
            if let Some(job) = job.borrow_mut().take() {
                job();
            }
            Ok(())
        }));
    }
}

/// The single entry point that owns "enter the platform message loop" — kept separate from
/// `Window::show()` for the same reason as `elwindui-backend-appkit`'s `application::run()` (see
/// that module's doc comment): it's the one well-defined place to install the task executor before
/// any generated code runs.
pub mod application {
    use super::{bindings, WinUI3Dispatcher};
    use elwindui_core::task::LocalExecutor;
    use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, GetMessageW, TranslateMessage, MSG};

    /// Blocking: enters the classic Win32 message loop. A `DispatcherQueueController` is created
    /// first (needed so `#[command(async)]` bodies have somewhere to post continuations back to —
    /// see `WinUI3Dispatcher`), but for an unpackaged Win32 app hosting WinUI3 content (as opposed
    /// to a packaged UWP-style app whose `Application::Start` owns the whole loop), the actual
    /// "keep the app alive and pump input/paint messages" loop is still the plain
    /// `GetMessageW`/`DispatchMessageW` pattern every Win32 app uses — `GetMessageW` returns `0`
    /// (loop exit) once `PostQuitMessage` has been called, which every top-level `Window` here is
    /// expected to do when closed (not yet wired — see the module's UNVERIFIED note; a real
    /// implementation needs a `Window.Closed` handler calling `PostQuitMessage(0)` once the last
    /// window closes, mirroring AppKit's `applicationShouldTerminateAfterLastWindowClosed`).
    pub fn run() {
        let controller = bindings::Microsoft::UI::Dispatching::DispatcherQueueController::CreateOnCurrentThread()
            .expect("DispatcherQueueController::CreateOnCurrentThread");
        let queue = controller.DispatcherQueue().expect("DispatcherQueue");
        elwindui_core::task::set_current(LocalExecutor::new(WinUI3Dispatcher { queue }));

        let mut msg = MSG::default();
        unsafe {
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}
