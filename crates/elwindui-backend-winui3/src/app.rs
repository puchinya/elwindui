//! Process/application lifecycle: the `DispatcherQueue`-backed `Dispatcher`, the C ABI
//! trampoline into the C++/WinRT app host (`cpp/app_host.cpp`), and the entry point that starts
//! the XAML application.
//!
//! Kept out of `lib.rs` so the crate root is only wiring; `run` is the one well-defined place
//! that installs the task executor before any generated code runs.

/// WinUI3's `Dispatcher` (docs/design/runtime/state_management_design.md): hops back to the UI thread via the
/// current thread's `DispatcherQueue` — the WinUI3/WinAppSDK analog of AppKit's
/// `dispatch2::DispatchQueue::main()`. `application::run()` (below) is what pumps this queue as
/// part of its own message loop, so a job enqueued from any thread is guaranteed to run promptly.
pub struct WinUI3Dispatcher {
    queue: bindings::Microsoft::UI::Dispatching::DispatcherQueue,
}

impl elwindui_core::task::Dispatcher for WinUI3Dispatcher {
    fn enqueue(&self, job: Box<dyn FnOnce() + Send + 'static>) {
        let job = std::cell::RefCell::new(Some(job));
        let _ = self.queue.TryEnqueue(
            &bindings::Microsoft::UI::Dispatching::DispatcherQueueHandler::new(move || {
                if let Some(job) = job.borrow_mut().take() {
                    job();
                }
                Ok(())
            }),
        );
    }
}

/// The single entry point that owns "enter the platform message loop" — kept separate from
/// `Window::show()` for the same reason as `elwindui-backend-appkit`'s `application::run()` (see
/// that module's doc comment): it's the one well-defined place to install the task executor before
/// any generated code runs.
use crate::bindings;
use elwindui_core::task::LocalExecutor;
use std::cell::RefCell;

thread_local! {
    // The generated callback wrapper requires its closure to be `Send`, whereas startup is
    // intentionally UI-thread-local. Keeping it in TLS means the callback captures nothing
    // and startup never acquires an incorrect `Send` bound.
    static STARTUP: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
    static WINDOWS: RefCell<Vec<RetainedWindow>> = const { RefCell::new(Vec::new()) };
    static NEXT_WINDOW_ID: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

pub(crate) struct RetainedWindow {
    id: u64,
    _window: bindings::Microsoft::UI::Xaml::Window,
}

// Hosting `Application` itself (composing it, registering `XamlControlsResources` into
// `Application.Resources`, receiving `OnLaunched`) lives in `cpp/app_host.cpp`, a small
// C++/WinRT shim built by `build.rs` via `cc` — not here. `windows-rs` has no support for
// WinRT "composable class" aggregation (subclassing a WinRT runtime class like `Application`);
// two from-scratch Rust attempts were tried (a `#[windows_core::implement]`-based one, and a
// from-scratch manual COM aggregation with correct outer->inner `QueryInterface` forwarding —
// see `git log` for `composed_application.rs`, since removed) and both left
// `Application.Resources` reproducibly broken (`Error 0x80004002 in
// ifactory->QueryInterface(Microsoft.UI.Xaml.Media.AcrylicBrush)`), ruling out COM identity as
// the cause. `ApplicationT<App>` — cppwinrt's own, real, widely-used composable-class support —
// does not hit this. Everything past `Application` construction/resources (window creation,
// controls, layout, rendering, event routing) stays in Rust; `cpp/app_host.cpp` calls back into
// it through nothing but the one C ABI function below. See microsoft/windows-rs#3404 and
// `cpp/app_host.cpp`'s own doc comment for the full investigation.
unsafe extern "C" {
    fn elwindui_winui3_run(startup: extern "C" fn());
}

/// The C ABI entry point `cpp/app_host.cpp`'s `App::OnLaunched` calls, once, after
/// `Application.Resources` already has `XamlControlsResources` merged in and before any
/// `Window`/control is constructed. Installs the task executor (needs a live `DispatcherQueue`,
/// which only exists once `Microsoft.UI.Xaml.Application::Start` has actually started running —
/// same requirement the old pure-Rust callback had), then runs the user's `startup`.
extern "C" fn startup_trampoline() {
    let queue = bindings::Microsoft::UI::Dispatching::DispatcherQueue::GetForCurrentThread()
        .expect("Microsoft.UI.Dispatching.DispatcherQueue::GetForCurrentThread");
    elwindui_core::task::set_current(LocalExecutor::new(WinUI3Dispatcher { queue }));

    STARTUP.with(|slot| {
        if let Some(startup) = slot.borrow_mut().take() {
            startup();
        }
    });
}

pub(crate) fn retain_window(window: &bindings::Microsoft::UI::Xaml::Window) {
    let id = NEXT_WINDOW_ID.with(|next| {
        let id = next.get();
        next.set(id.wrapping_add(1));
        id
    });
    let closed = windows::Foundation::TypedEventHandler::new(move |_, _| {
        release_window(id);
        Ok(())
    });
    window
        .Closed(&closed)
        .expect("Window::Closed event registration");
    WINDOWS.with(|windows| {
        windows.borrow_mut().push(RetainedWindow { id, _window: window.clone() });
    });
}

pub(crate) fn release_window(id: u64) {
    let has_windows = WINDOWS.with(|windows| {
        let mut windows = windows.borrow_mut();
        windows.retain(|entry| entry.id != id);
        !windows.is_empty()
    });
    if !has_windows {
        bindings::Microsoft::UI::Xaml::Application::Current()
            .expect("Microsoft.UI.Xaml.Application::Current")
            .Exit()
            .expect("Microsoft.UI.Xaml.Application::Exit");
    }
}

/// Runs `startup` from the C++/WinRT shim's `App::OnLaunched` (via `startup_trampoline`), then
/// lets `Microsoft.UI.Xaml.Application::Start` (called from `cpp/app_host.cpp`) own the native
/// message loop.
pub fn run<F>(startup: F)
where
    F: FnOnce() + 'static,
{
    STARTUP.with(|slot| {
        assert!(slot.borrow().is_none(), "elwindui::application::run may only be called once");
        *slot.borrow_mut() = Some(Box::new(startup));
    });
    // No ambient Environment entry needed (CI-6 of #80): every generated component's `mount()`
    // calls `elwindui_core::environment::application_environment()` directly, a plain deterministic
    // function call reachable from `startup()` and from any later event callback alike. See
    // `docs/design/runtime/theme_environment_design.md`'s "Application boundary" and
    // `elwindui-backend-appkit`'s `app::run` for the mirrored AppKit shape.
    unsafe { elwindui_winui3_run(startup_trampoline) };
}
