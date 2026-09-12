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
use std::rc::Rc;

thread_local! {
    // The generated callback wrapper requires its closure to be `Send`, whereas startup is
    // intentionally UI-thread-local. Keeping it in TLS means the callback captures nothing
    // and startup never acquires an incorrect `Send` bound.
    static STARTUP: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
    static WINDOWS: RefCell<Vec<RetainedWindow>> = const { RefCell::new(Vec::new()) };
    static NEXT_WINDOW_ID: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    #[cfg(test)]
    static RELEASE_WINDOW_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(crate) struct RetainedWindow {
    id: u64,
    /// Issue #254: the final most-derived Rust `Window` owner (`Rc<dyn WindowExt>`, obtained via
    /// `__self_weak` at construction time) — the application layer is the strong lifetime
    /// authority for this until native close is observed. `InnerWindow` itself stores only the
    /// matching `Weak`. Retaining this (rather than only the native XAML `Window`, as before)
    /// keeps the whole Rust-side component tree — including `TreeHost`'s callback registry —
    /// alive for as long as the native window is shown.
    _owner: Rc<dyn elwindui_core::ui::WindowExt>,
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

/// Issue #254: becomes the application-layer strong lifetime authority for `owner` (the final
/// most-derived generated/backend `Window`) until native close is observed via `xaml`'s `Closed`
/// event. The `Closed` closure captures only the numeric `id`, never `owner` itself — so this
/// registry, not any native callback, is what keeps `owner` alive.
pub(crate) fn retain_window(
    owner: Rc<dyn elwindui_core::ui::WindowExt>,
    xaml: &bindings::Microsoft::UI::Xaml::Window,
) -> u64 {
    let id = NEXT_WINDOW_ID.with(|next| {
        let id = next.get();
        next.set(id.wrapping_add(1));
        id
    });
    let closed = windows::Foundation::TypedEventHandler::new(move |_, _| {
        release_window(id);
        Ok(())
    });
    xaml.Closed(&closed)
        .expect("Window::Closed event registration");
    WINDOWS.with(|windows| {
        windows
            .borrow_mut()
            .push(RetainedWindow { id, _owner: owner });
    });
    id
}

/// Issue #254 §2.4/§2.7: the removed `RetainedWindow` (and the `Rc<dyn WindowExt>` owner it
/// holds) is dropped only after `WINDOWS`'s `RefCell` borrow has ended, and registry emptiness is
/// re-checked with a fresh borrow afterward — owner destruction can synchronously re-enter Window
/// logic, which must never observe `WINDOWS` still mutably borrowed.
pub(crate) fn release_window(id: u64) {
    #[cfg(test)]
    RELEASE_WINDOW_CALLS.with(|calls| calls.set(calls.get() + 1));

    let removed = WINDOWS.with(|windows| {
        let mut windows = windows.borrow_mut();
        let index = windows.iter().position(|entry| entry.id == id);
        index.map(|index| windows.remove(index))
    });
    drop(removed);

    let is_empty = WINDOWS.with(|windows| windows.borrow().is_empty());
    if is_empty {
        bindings::Microsoft::UI::Xaml::Application::Current()
            .expect("Microsoft.UI.Xaml.Application::Current")
            .Exit()
            .expect("Microsoft.UI.Xaml.Application::Exit");
    }
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
    RELEASE_WINDOW_CALLS.with(std::cell::Cell::get)
}

/// Runs `startup` from the C++/WinRT shim's `App::OnLaunched` (via `startup_trampoline`), then
/// lets `Microsoft.UI.Xaml.Application::Start` (called from `cpp/app_host.cpp`) own the native
/// message loop.
pub fn run<F>(startup: F)
where
    F: FnOnce() + 'static,
{
    STARTUP.with(|slot| {
        assert!(
            slot.borrow().is_none(),
            "elwindui::application::run may only be called once"
        );
        *slot.borrow_mut() = Some(Box::new(startup));
    });
    // No ambient Environment entry needed (CI-6 of #80): every generated component's `mount()`
    // calls `elwindui_core::environment::application_environment()` directly, a plain deterministic
    // function call reachable from `startup()` and from any later event callback alike. See
    // `docs/design/runtime/theme_environment_design.md`'s "Application boundary" and
    // `elwindui-backend-appkit`'s `app::run` for the mirrored AppKit shape.
    unsafe { elwindui_winui3_run(startup_trampoline) };
}

#[cfg(test)]
mod window_lifecycle_tests {
    use super::*;
    use elwindui_core::ui::WindowExt;

    /// Issue #254, T1/T2/T3/T4/T12: proves the application-layer retention invariant using a bare
    /// `crate::native_ui::Window` (no `elwindui-codegen` generated component involved, and no
    /// dependency on the higher-level `elwindui` DSL crate, which cannot see this crate's
    /// `pub(crate)` test counters across the crate boundary). The mechanism under test —
    /// `__self_weak` captured at `Window::construct()`, threaded into `InnerWindow`, handed to
    /// `retain_window` on first `show()` — is identical for a bare `Window::new()` and for a
    /// generated `inherits Window` component's own base field (`__self_weak` there resolves to the
    /// outer generated `Rc`, coerced to `Weak<dyn WindowExt>` — see
    /// `docs/design/runtime/component_lifecycle_design.md` §4j), so this single-crate test proves
    /// the mechanism generically.
    ///
    /// Packed into one `#[test]`/one `crate::run()` call, mirroring
    /// `crates/elwindui/tests/window_mount_hide_close.rs`'s own convention:
    /// `Microsoft.UI.Xaml.Application` is a process/apartment-wide singleton, so only one
    /// `crate::run()` per test thread is safe; this crate's `thread_local!` `WINDOWS`/
    /// `NEXT_WINDOW_ID`/`RELEASE_WINDOW_CALLS` registry state is isolated per test thread
    /// regardless, so this does not need to be the crate's only test.
    #[test]
    fn window_registry_retention_lifecycle() {
        crate::init().expect("elwindui_backend_winui3::init");
        reset_window_lifecycle_test_state();

        run(|| {
            // T12: a never-shown Window creates no registry entry and can close/drop safely,
            // without releasing an unrelated id.
            let never_shown = crate::native_ui::Window::new();
            let never_shown_weak = std::rc::Rc::downgrade(&never_shown);
            assert_eq!(
                retained_window_count_for_test(),
                0,
                "T12: construct() alone must not retain"
            );
            never_shown.close();
            assert_eq!(
                release_window_call_count_for_test(),
                0,
                "T12: closing a never-shown window must not call release_window at all"
            );
            drop(never_shown);
            assert!(
                never_shown_weak.upgrade().is_none(),
                "T12: a never-shown, never-retained window must drop normally"
            );

            // T1: Window A survives the caller's own Rc dropping — the *only* remaining strong
            // reference from here on is the application registry's own, never a test-local one.
            // PR #257 review remediation (A1): every subsequent operation on A goes through a
            // freshly-upgraded, explicitly scoped temporary `Rc` that is dropped again before the
            // next assertion — an earlier revision of this test kept a `let window = weak.upgrade()
            // ...` binding alive across `hide()`/`show()`/`close()` and all the way to the final
            // `weak.upgrade().is_none()` assertion. That lingering local `Rc` made
            // `weak.upgrade()` keep succeeding regardless of whether `app::WINDOWS` had actually
            // released its own entry, so `weak.upgrade().is_none()` could never become true —
            // making the final assertion impossible to satisfy and therefore unable to prove that
            // application-registry release, specifically, is what this test claims to verify.
            let window_a = crate::native_ui::Window::new();
            let weak_a = std::rc::Rc::downgrade(&window_a);
            window_a.show();
            drop(window_a);
            assert!(
                weak_a.upgrade().is_some(),
                "T1: the final owner must still be alive after the caller's own Rc drops"
            );
            assert_eq!(
                retained_window_count_for_test(),
                1,
                "T1: exactly one retained window"
            );

            // T3: hide() then show() must retain exactly once, not twice. The temporary upgrade
            // is scoped and dropped again immediately after use.
            {
                let window_a = weak_a.upgrade().expect("A retained");
                window_a.hide();
                window_a.show();
            }
            assert_eq!(
                retained_window_count_for_test(),
                1,
                "T3: hide() then show() must not create a second retention entry"
            );
            assert!(
                weak_a.upgrade().is_some(),
                "T3: A must remain alive after the temporary upgrade used for hide()/show() drops"
            );

            // T4: a second, independent Window — its own caller-side Rc is dropped too, so both
            // Windows are proven alive by the application registry alone, not by any test-local
            // strong reference.
            let window_b = crate::native_ui::Window::new();
            let weak_b = std::rc::Rc::downgrade(&window_b);
            window_b.show();
            drop(window_b);
            assert_eq!(
                retained_window_count_for_test(),
                2,
                "T4: two shown windows must both be retained"
            );
            assert!(weak_a.upgrade().is_some(), "T4: A must still be alive");
            assert!(weak_b.upgrade().is_some(), "T4: B must still be alive");

            // T4/T2: closing A (through another scoped temporary upgrade) releases only A.
            {
                let window_a = weak_a.upgrade().expect("A retained");
                window_a.close();
            }
            assert_eq!(
                release_window_call_count_for_test(),
                1,
                "T2: release_window must be called exactly once for A"
            );
            assert_eq!(
                retained_window_count_for_test(),
                1,
                "T4: B must remain retained after A closes"
            );
            assert!(
                weak_a.upgrade().is_none(),
                "T2: A's owner must actually have dropped now that its temporary upgrade is gone too"
            );
            assert!(
                weak_b.upgrade().is_some(),
                "T4: B must remain alive while A is gone"
            );

            // T4/T2: closing B releases it too and empties the registry. WinUI3's
            // exit-on-empty-registry call (`release_window`) is not independently observable from
            // inside this same synchronous startup closure — `Application::Exit()` only takes
            // effect once this closure returns control to the native message loop — but this whole
            // `run()` call itself returning after this test function ends is the proof the loop
            // did in fact stop once the registry emptied.
            {
                let window_b = weak_b.upgrade().expect("B retained");
                window_b.close();
            }
            assert_eq!(
                release_window_call_count_for_test(),
                2,
                "T2: both windows must each release exactly once"
            );
            assert_eq!(
                retained_window_count_for_test(),
                0,
                "T4: registry must be empty"
            );
            assert!(
                weak_b.upgrade().is_none(),
                "T2: B's owner must have dropped now that its temporary upgrade is gone too"
            );
        });
    }
}
