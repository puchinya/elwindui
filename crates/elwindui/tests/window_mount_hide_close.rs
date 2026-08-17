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
    body: view! {
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

/// Type-checked, not executed (see module doc comment). Demonstrates the target usage shape from
/// spec §10/§11 and Issue #126: `new()` performs no build; a property set between `new()` and `show()`
/// is observed by the initial build; `show()` mounts+builds exactly once; `show(); hide(); show();`
/// does not rebuild or unmount; `close()` runs child-first recursive unmount.
#[allow(dead_code)]
#[cfg(feature = "backend-appkit")]
fn type_checked_new_show_hide_close_usage() {
    BUILD_COUNT.with(|c| c.set(0));
    UNMOUNT_EVENTS.with(|events| events.borrow_mut().clear());

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
}

#[cfg(all(feature = "backend-winui3", target_os = "windows"))]
#[test]
fn winui3_show_hide_show_builds_once_and_close_cascades_unmount() {
    elwindui::init().expect("initialize WinUI3");
    BUILD_COUNT.with(|count| count.set(0));
    UNMOUNT_EVENTS.with(|events| events.borrow_mut().clear());

    elwindui::application::run(|| {
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
    });

    assert_eq!(BUILD_COUNT.with(Cell::get), 1);
}
