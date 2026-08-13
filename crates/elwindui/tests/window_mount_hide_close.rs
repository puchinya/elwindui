//! CI-8 of #80 (docs/design/runtime/component_lifecycle_design.md §4g): `Window::show()` now
//! implicitly mounts an unmounted host-composition component on first call; `hide()`/`close()` are
//! new. Type-checked but **not executed** — same reasoning as `for_item_two_way.rs`: AppKit requires
//! native window/view construction on the process main thread (`elwindui_backend_appkit::inner`'s
//! `mtm()`/`MainThreadMarker` calls panic off it), while Rust's default test harness invokes `#[test]`
//! functions from worker threads. Runtime verification of `show`/`hide`/`close`'s actual mount-once/
//! visibility/cleanup behavior was done via `crates/elwindui-codegen`'s own generated-source-text
//! test (`component_frontend::tests::host_composition_gets_inherent_show_hide_close_and_no_auto_mount_on_constructed`)
//! and manual reasoning from the AppKit backend build succeeding — not by running this file.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]
#![cfg(feature = "backend-appkit")]

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
