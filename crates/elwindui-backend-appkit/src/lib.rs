//! AppKit implementation of the widget surface `elwindui-codegen` targets for the `notepad`
//! example. See docs/elwindui_spec.md 付録A, 付録C, docs/elwindui_gui_framework_design.md §3.
//!
//! Split into `inner` (private — raw AppKit plumbing, `Inner`-prefixed types) and `native_ui`
//! (public, re-exported here — implements every `elwindui_core::ui` builtin trait this backend
//! provides by composing the matching `inner` type). See each module's own doc comment.

#![cfg(target_os = "macos")]
// `#[elwindui_macros::class]`'s `__elwindui_inherit_*!` chain mechanism needs a same-crate
// macro-to-macro reference (`$crate::the_macro!`) to also work cross-crate, which currently
// requires this lint disabled — see `crates/elwindui-macros/src/class.rs`'s own doc comment on
// `inherit_macro_self_ref_path` for the full explanation, and `docs/elwindui_macro_class_spec.md`.
// Every crate using `#[class]` with a same-crate `inherits` chain needs this same line.
#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

/// Performs process-wide AppKit setup required before creating views.
///
/// AppKit performs this lazily when the application object is created, so this is intentionally
/// idempotent and currently has no eager work. It exists to keep the facade's `elwindui::init()`
/// contract uniform across native backends.
pub fn init() -> Result<(), std::convert::Infallible> {
    Ok(())
}

mod inner;
mod native_ui;
mod vector_renderer;

pub use native_ui::*;

// `elwindui-codegen`'s generated code references `elwindui::backend::AnyView` directly (see
// `inner::AnyView`'s own doc comment), so it needs to stay reachable at this crate's own root even
// though the rest of `inner` is private.
pub use inner::AnyView;

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSApplication, NSApplicationDelegate};
use objc2_foundation::NSObjectProtocol;
use std::cell::RefCell;

/// See docs/elwindui_spec.md 付録T.2. Modal file panels (`runModal`) are themselves synchronous
/// (they block until the user closes the panel), so these `async fn`s never actually suspend —
/// they resolve on the first poll. That's enough for `#[command(async)]` bodies that just need to
/// `.await` a dialog result; it is not a general-purpose async executor (nothing here can yield
/// across a real I/O wait), which is what `elwindui-core`'s planned `Dispatcher`/`spawn`
/// (docs/elwindui_gui_framework_design.md §7.3) is for.
pub mod platform {
    pub mod file_dialog {
        use crate::inner::mtm;
        use objc2_app_kit::{NSModalResponseOK, NSOpenPanel, NSSavePanel};
        use std::path::PathBuf;

        pub async fn open() -> Option<PathBuf> {
            let panel = NSOpenPanel::openPanel(mtm());
            if panel.runModal() != NSModalResponseOK {
                return None;
            }
            panel
                .URL()
                .and_then(|url| url.path())
                .map(|p| PathBuf::from(p.to_string()))
        }

        pub async fn save() -> Option<PathBuf> {
            let panel = NSSavePanel::savePanel(mtm());
            if panel.runModal() != NSModalResponseOK {
                return None;
            }
            panel
                .URL()
                .and_then(|url| url.path())
                .map(|p| PathBuf::from(p.to_string()))
        }
    }
}

/// AppKit's `Dispatcher` (docs/elwindui_spec.md 付録P.5): hops back to the main thread via GCD's
/// main queue, which `NSApplication.run()` (`application::run()` below) actively services as part
/// of its own event loop — so a job enqueued from any thread (a background `tokio` task
/// completing, say) is guaranteed to run promptly. See `elwindui_core::task` for how this lets a
/// suspended `#[command(async)]` body resume back on the UI thread, the same role C#'s
/// `SynchronizationContext.Post` plays.
pub struct AppKitDispatcher;

impl elwindui_core::task::Dispatcher for AppKitDispatcher {
    fn enqueue(&self, job: Box<dyn FnOnce() + Send + 'static>) {
        dispatch2::DispatchQueue::main().exec_async(job);
    }
}

thread_local! {
    /// `NSApplication.delegate` is an unretained (weak) reference, so this keeps it alive for the
    /// process's lifetime.
    static APP_DELEGATE: RefCell<Option<Retained<AppDelegate>>> = const { RefCell::new(None) };
}

define_class!(
    #[unsafe(super(objc2_foundation::NSObject))]
    #[thread_kind = objc2::MainThreadOnly]
    struct AppDelegate;

    unsafe impl NSObjectProtocol for AppDelegate {}

    unsafe impl NSApplicationDelegate for AppDelegate {
        /// Without this, AppKit's default behavior leaves the process running after the last
        /// (only, for `notepad`) window is closed via its close button.
        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn should_terminate_after_last_window_closed(&self, _sender: &NSApplication) -> bool {
            true
        }
    }
);

impl AppDelegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

/// The single entry point that owns "enter the platform event loop" — kept separate from
/// `Window::show()` so that there's one well-defined place to install the task executor (see
/// `elwindui_core::task::set_current`) and the app delegate before any generated code runs. Call
/// once, after showing the app's window(s).
pub mod application {
    use super::{APP_DELEGATE, AppDelegate, AppKitDispatcher};
    use crate::inner::mtm;
    use elwindui_core::task::LocalExecutor;
    use objc2_app_kit::NSApplication;

    /// Runs `startup` on AppKit's main thread, then enters the AppKit main event loop.
    pub fn run<F>(startup: F)
    where
        F: FnOnce() + 'static,
    {
        elwindui_core::task::set_current(LocalExecutor::new(AppKitDispatcher));

        let mtm = mtm();
        let app = NSApplication::sharedApplication(mtm);
        let delegate = AppDelegate::new(mtm);
        app.setDelegate(Some(objc2::runtime::ProtocolObject::from_ref(&*delegate)));
        APP_DELEGATE.with(|d| *d.borrow_mut() = Some(delegate));

        startup();
        app.run();
    }
}
