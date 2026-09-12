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
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// AppKit's `Dispatcher` (docs/design/runtime/state_management_design.md): hops back to the main thread via GCD's
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
    static WINDOWS: RefCell<Vec<RetainedWindow>> = const { RefCell::new(Vec::new()) };
    static NEXT_WINDOW_ID: Cell<u64> = const { Cell::new(0) };
    #[cfg(test)]
    static RELEASE_WINDOW_CALLS: Cell<usize> = const { Cell::new(0) };
}

/// Issue #254: the application-layer registry that becomes the strong lifetime authority for the
/// final most-derived Rust `Window` owner (`Rc<dyn WindowExt>`, obtained via `__self_weak` at
/// construction time) once it has been shown for the first time — mirrors
/// `elwindui-backend-winui3::app::RetainedWindow`. `InnerWindow` itself stores only a matching
/// `Weak`; the native close hook (`ElwinduiWindow::windowWillClose:`) is what calls
/// `release_window` — never a strong native-callback capture.
struct RetainedWindow {
    id: u64,
    _owner: Rc<dyn elwindui_core::ui::WindowExt>,
}

/// Allocates a retention id, registers `owner` as strongly retained until `release_window(id)` is
/// called, and returns the id for the caller (`InnerWindow::show`) to store on
/// `ElwinduiWindowIvars::retention_id`.
pub(crate) fn retain_window(owner: Rc<dyn elwindui_core::ui::WindowExt>) -> u64 {
    let id = NEXT_WINDOW_ID.with(|next| {
        let id = next.get();
        next.set(id.wrapping_add(1));
        id
    });
    WINDOWS.with(|windows| {
        windows
            .borrow_mut()
            .push(RetainedWindow { id, _owner: owner });
    });
    id
}

/// Issue #254 §2.4: removes the entry under a short `RefCell` borrow, then drops the removed
/// owner only after that borrow has ended — owner destruction can synchronously re-enter Window
/// logic (e.g. via `Drop` on content still mounted), which must never observe `WINDOWS` still
/// mutably borrowed. Unlike WinUI3, application termination stays entirely policy-driven through
/// `AppDelegate::should_terminate_after_last_window_closed` (§2.12) — no exit call belongs here.
pub(crate) fn release_window(id: u64) {
    #[cfg(test)]
    RELEASE_WINDOW_CALLS.with(|calls| calls.set(calls.get() + 1));

    let removed = WINDOWS.with(|windows| {
        let mut windows = windows.borrow_mut();
        let index = windows.iter().position(|entry| entry.id == id);
        index.map(|index| windows.remove(index))
    });
    drop(removed);
}

#[cfg(test)]
pub(crate) fn reset_window_lifecycle_test_state() {
    WINDOWS.with(|windows| {
        assert!(
            windows.borrow().is_empty(),
            "window lifecycle test must start without retained windows"
        );
    });
    RELEASE_WINDOW_CALLS.with(|calls| calls.set(0));
}

#[cfg(test)]
pub(crate) fn retained_window_count_for_test() -> usize {
    WINDOWS.with(|windows| windows.borrow().len())
}

#[cfg(test)]
pub(crate) fn release_window_call_count_for_test() -> usize {
    RELEASE_WINDOW_CALLS.with(Cell::get)
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

    // No ambient Environment entry needed (CI-6 of #80): every generated component's `mount()`
    // calls `elwindui_core::environment::application_environment()` directly, a plain deterministic
    // function call reachable from `startup()` and from any later event callback alike. See
    // `docs/design/runtime/theme_environment_design.md`'s "Application boundary".
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
