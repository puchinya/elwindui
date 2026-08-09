//! Process/application lifecycle: the GCD-backed `Dispatcher`, the `NSApplicationDelegate`, and
//! the single entry point that enters AppKit's event loop.
//!
//! Kept out of `lib.rs` so the crate root is only wiring; `run` is the one well-defined place
//! that installs the task executor and the app delegate before any generated code runs.

use crate::ffi::mtm;
use elwindui_core::task::LocalExecutor;
use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSApplication, NSApplicationDelegate};
use objc2_foundation::NSObjectProtocol;
use std::cell::RefCell;

/// AppKit's `Dispatcher` (docs/design/gui_framework_design.md §7.3): hops back to the main thread via GCD's
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
    #[cfg(feature = "render-stats")]
    crate::diagnostics::schedule_env_report();
    app.run();
}
