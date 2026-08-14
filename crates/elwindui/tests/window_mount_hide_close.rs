//! CI-8 of #80 (docs/design/runtime/component_lifecycle_design.md §4g): `Window::show()` now
//! implicitly mounts an unmounted host-composition component on first call; `hide()`/`close()` are
//! new. AppKit keeps the usage shape type-checked but does not execute it because native window/view
//! construction requires the process main thread (`elwindui_backend_appkit::inner`'s
//! `mtm()`/`MainThreadMarker` calls panic from Rust's test-harness worker threads). WinUI3 executes
//! the same lifecycle in a real hosted application and verifies that hide/re-show does not rebuild.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]
#![cfg(any(feature = "backend-appkit", feature = "backend-winui3"))]

use std::cell::Cell;
use std::rc::Rc;

thread_local! {
    static BUILD_COUNT: Cell<u32> = const { Cell::new(0) };
}

#[elwindui::component(inherits Window)]
struct MountHideCloseWindow {
    #[prop]
    subtitle: String,

    body: view! {
        on_mount {
            BUILD_COUNT.with(|c| c.set(c.get() + 1));
        }
        title: subtitle
        content: VerticalLayout {
            TextBlock { text: "hello" }
        }
    },
}

#[elwindui::component]
impl MountHideCloseWindow {}

/// Type-checked, not executed (see module doc comment). Demonstrates the target usage shape from
/// spec §10/§11: `new()` performs no build; a property set between `new()` and `show()` is observed
/// by the initial build; `show()` mounts+builds exactly once; `show(); hide(); show();` does not
/// rebuild; `close()` runs cleanup.
#[allow(dead_code)]
#[cfg(feature = "backend-appkit")]
fn type_checked_new_show_hide_close_usage() {
    BUILD_COUNT.with(|c| c.set(0));

    let window: Rc<MountHideCloseWindow> = MountHideCloseWindow::new("initial".to_string());
    // `new()` alone must not have built the view yet (host-composition `on_constructed` no longer
    // auto-mounts — codegen.rs's `on_constructed_mount_call` is `None` for this case).
    debug_assert_eq!(BUILD_COUNT.with(|c| c.get()), 0);

    window.set_subtitle("changed before first show".to_string());

    window.show(); // first call: mounts + builds exactly once, then shows natively
    debug_assert_eq!(BUILD_COUNT.with(|c| c.get()), 1);

    window.hide(); // visibility only — does not unmount
    window.show(); // re-show: must not rebuild
    debug_assert_eq!(BUILD_COUNT.with(|c| c.get()), 1);

    window.close(); // ends the mount lifetime; releases the native window
}

#[cfg(feature = "backend-winui3")]
#[test]
fn winui3_show_hide_show_builds_once_and_close_exits() {
    elwindui::init().expect("initialize WinUI3");
    BUILD_COUNT.with(|count| count.set(0));

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
        window.show();
        assert_eq!(
            BUILD_COUNT.with(Cell::get),
            1,
            "hide() followed by show() must not rebuild"
        );

        window.close();
    });

    assert_eq!(BUILD_COUNT.with(Cell::get), 1);
}
