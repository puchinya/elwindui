//! CI-8 of #80 (docs/design/runtime/component_lifecycle_design.md §4g): `Window::show()` now
//! implicitly mounts an unmounted host-composition component on first call; `hide()`/`close()` are
//! new. Issue #126: `Window::close()` recursively cascades teardown to its child subtree in
//! child-first order and cancels subscriptions. AppKit keeps the usage shape type-checked but does
//! not execute it because native window/view construction requires the process main thread
//! (`elwindui_backend_appkit::inner`'s `mtm()`/`MainThreadMarker` calls panic from Rust's
//! test-harness worker threads). WinUI3 executes the same lifecycle in a real hosted application
//! and verifies that hide/re-show does not rebuild and close performs child-first recursive unmount.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]
#![cfg(any(feature = "backend-appkit", feature = "backend-winui3"))]

use elwindui::core::ui::WindowExt;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

thread_local! {
    static BUILD_COUNT: Cell<u32> = const { Cell::new(0) };
    static UNMOUNT_EVENTS: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
}

fn record_unmount(name: &'static str) {
    UNMOUNT_EVENTS.with(|events| events.borrow_mut().push(name));
}

fn get_unmount_events() -> Vec<&'static str> {
    UNMOUNT_EVENTS.with(|events| events.borrow().clone())
}

#[elwindui::component(inherits ContentControl)]
struct WindowChildComponent {
    template: template_view! {
        on_unmount {
            record_unmount("WindowChild");
        }
        TextBlock { text: "window child" }
    },
}

#[elwindui::component]
impl WindowChildComponent {}

#[elwindui::component(inherits VerticalLayout)]
struct WindowParentComponent {
    body: view! {
        on_unmount {
            record_unmount("WindowParent");
        }
        WindowChildComponent { }
    },
}

#[elwindui::component]
impl WindowParentComponent {}

#[elwindui::component(inherits Window)]
struct MountHideCloseWindow {
    #[prop]
    subtitle: String,

    body: view! {
        on_mount {
            BUILD_COUNT.with(|c| c.set(c.get() + 1));
        }
        on_unmount {
            record_unmount("WindowRoot");
        }
        title: subtitle
        content: VerticalLayout {
            WindowParentComponent { }
        }
    },
}

#[elwindui::component]
impl MountHideCloseWindow {}

/// PR #165 rereview remediation round 2, A6/T25: a Window whose content declares a
/// `context_popup: view! { .. }` directly (not through a nested Component) — used only to prove
/// the *shape* compiles; see `type_checked_window_with_declarative_popup_content_compiles`, below,
/// for why this is compile-only (real execution needs a native Window, main-thread-only, same
/// constraint as every other fixture in this file) and where the actual T25 ordering proof lives
/// instead.
#[elwindui::component(inherits Window)]
struct MountHideCloseWindowWithPopup {
    body: view! {
        on_unmount {
            record_unmount("PopupWindowRoot");
        }
        title: "popup window"
        content: VerticalLayout {
            TextBlock {
                text: "target",
                context_popup: view! {
                    on_unmount {
                        record_unmount("PopupContent");
                    }
                    TextBlock { text: "popup" }
                },
            }
        }
    },
}

#[elwindui::component]
impl MountHideCloseWindowWithPopup {}

/// Type-checked, not executed (see module doc comment). Demonstrates the target usage shape from
/// spec §10/§11 and Issue #126: `new()` performs no build; a property set between `new()` and `show()`
/// is observed by the initial build; `show()` mounts+builds exactly once; `show(); hide(); show();`
/// does not rebuild or unmount; `close()` runs child-first recursive unmount; `close()` before `show()`
/// does not run `on_unmount`.
#[allow(dead_code)]
#[cfg(feature = "backend-appkit")]
fn type_checked_new_show_hide_close_usage() {
    BUILD_COUNT.with(|c| c.set(0));
    UNMOUNT_EVENTS.with(|events| events.borrow_mut().clear());

    // 1. Close before show: Created -> Unmounted (does not run on_unmount)
    let window0: Rc<MountHideCloseWindow> = MountHideCloseWindow::new("w0".to_string());
    debug_assert_eq!(BUILD_COUNT.with(|c| c.get()), 0);
    debug_assert_eq!(get_unmount_events().len(), 0);

    window0.close();
    debug_assert_eq!(BUILD_COUNT.with(|c| c.get()), 0);
    debug_assert_eq!(get_unmount_events().len(), 0);

    window0.show();
    debug_assert_eq!(BUILD_COUNT.with(|c| c.get()), 0);
    debug_assert_eq!(get_unmount_events().len(), 0);

    // 2. Show -> Hide -> Show -> Close
    let window: Rc<MountHideCloseWindow> = MountHideCloseWindow::new("initial".to_string());
    // `new()` alone must not have built the view yet (host-composition `on_constructed` no longer
    // auto-mounts — codegen.rs's `on_constructed_mount_call` is `None` for this case).
    debug_assert_eq!(BUILD_COUNT.with(|c| c.get()), 0);

    window.set_subtitle("changed before first show".to_string());

    window.show(); // first call: mounts + builds exactly once, then shows natively
    debug_assert_eq!(BUILD_COUNT.with(|c| c.get()), 1);

    window.hide(); // visibility only — does not unmount
    debug_assert_eq!(get_unmount_events().len(), 0);

    window.show(); // re-show: must not rebuild
    debug_assert_eq!(BUILD_COUNT.with(|c| c.get()), 1);
    debug_assert_eq!(get_unmount_events().len(), 0);

    window.close(); // ends the mount lifetime; cascades child-first recursive unmount
    debug_assert_eq!(
        get_unmount_events(),
        vec!["WindowChild", "WindowParent", "WindowRoot"]
    );

    // Double close is safe and idempotent
    window.close();
    debug_assert_eq!(
        get_unmount_events(),
        vec!["WindowChild", "WindowParent", "WindowRoot"]
    );

    // show() after close is a no-op (does not remount or rebuild or reopen)
    window.show();
    debug_assert_eq!(BUILD_COUNT.with(|c| c.get()), 1);
}

/// PR #165 rereview remediation round 2, A6/T25: compile-only usage fixture — a `debug_assert_eq!`
/// inside a function that is never executed (this crate's own tests never call it; see the module
/// doc comment for why AppKit `Window` construction can't run in this `#[test]` harness at all)
/// proves *nothing* about runtime ordering, so this function's own body no longer makes that
/// claim. Its only remaining job is to keep `Window` + a declarative `context_popup: view! { .. }`
/// on the same content tree compiling. The actual T25 (popup-before-owner-content teardown
/// ordering) proof lives at two lower layers that genuinely do execute (or are structurally
/// inspected) without needing a real native `Window`:
/// - `elwindui-codegen::codegen::tests::t25_generated_unmount_override_runs_before_owner_content_unmount_subtree`
///   (and `t19_generated_close_and_unmount_order_teardown_before_native_close`, which asserts the
///   same ordering as part of a broader proof) — inspects the real generated `unmount()` source,
///   proving `unmount_override()` (which closes any active popup) runs before the owner's own
///   content `unmount_subtree`.
/// - `elwindui_backend_appkit::host::close_active_popup_slot_tests` (executed on this environment)
///   and `elwindui_backend_winui3::host::tests` (code-reviewed only, this crate does not compile
///   here) — prove the backend's own active-popup slot is taken and closed reentrancy-safely.
#[allow(dead_code)]
#[cfg(feature = "backend-appkit")]
fn type_checked_window_with_declarative_popup_content_compiles() {
    let window: Rc<MountHideCloseWindowWithPopup> = MountHideCloseWindowWithPopup::new();
    window.show();
    window.close();
}

#[cfg(all(feature = "backend-winui3", target_os = "windows"))]
#[test]
fn winui3_show_hide_show_builds_once_and_close_cascades_unmount() {
    elwindui::init().expect("initialize WinUI3");
    BUILD_COUNT.with(|count| count.set(0));
    UNMOUNT_EVENTS.with(|events| events.borrow_mut().clear());

    elwindui::application::run(|| {
        // Part 1: close before show
        let window1: Rc<MountHideCloseWindow> = MountHideCloseWindow::new("w1".to_string());
        assert_eq!(BUILD_COUNT.with(Cell::get), 0);
        assert_eq!(get_unmount_events().len(), 0);

        window1.close();
        assert_eq!(BUILD_COUNT.with(Cell::get), 0);
        assert_eq!(
            get_unmount_events().len(),
            0,
            "close() on an unshown window must not execute on_unmount"
        );

        window1.close();
        assert_eq!(get_unmount_events().len(), 0);

        window1.show();
        assert_eq!(BUILD_COUNT.with(Cell::get), 0);
        assert_eq!(get_unmount_events().len(), 0);

        // Part 2: show -> hide -> show -> close
        let window: Rc<MountHideCloseWindow> = MountHideCloseWindow::new("initial".to_string());
        assert_eq!(
            BUILD_COUNT.with(Cell::get),
            0,
            "new() must not build a Window-rooted component"
        );

        window.set_subtitle("changed before first show".to_string());
        window.show();
        assert_eq!(
            BUILD_COUNT.with(Cell::get),
            1,
            "first show() must mount and build exactly once"
        );

        window.hide();
        assert_eq!(
            get_unmount_events().len(),
            0,
            "hide() must not trigger unmount"
        );

        window.show();
        assert_eq!(
            BUILD_COUNT.with(Cell::get),
            1,
            "hide() followed by show() must not rebuild"
        );
        assert_eq!(
            get_unmount_events().len(),
            0,
            "re-show must not trigger unmount"
        );

        window.close();
        assert_eq!(
            get_unmount_events(),
            vec!["WindowChild", "WindowParent", "WindowRoot"],
            "close() must execute child-first recursive unmount"
        );

        // Double close idempotency
        window.close();
        assert_eq!(
            get_unmount_events(),
            vec!["WindowChild", "WindowParent", "WindowRoot"],
            "second close() must be a no-op"
        );

        // show() after close is a no-op
        window.show();
        assert_eq!(
            BUILD_COUNT.with(Cell::get),
            1,
            "show() after close must be a no-op"
        );
    });

    assert_eq!(BUILD_COUNT.with(Cell::get), 1);
}
