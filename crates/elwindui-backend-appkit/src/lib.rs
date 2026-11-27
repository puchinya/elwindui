//! AppKit backend — the concrete widget surface `elwindui-codegen` targets on macOS.
//! See docs/design/backends/appkit_backend_design.md.
//!
//! Layering — dependencies run one way only, `native_ui -> inner -> host -> render -> ffi`:
//!
//! | module      | owns |
//! |-------------|------|
//! | `native_ui` | the public façade: one `#[class]` per builtin, implementing the matching
//! |             | `elwindui_core::ui` `*Ext` trait by delegating to its `inner` twin |
//! | `inner`     | raw per-control plumbing, `Inner`-prefixed |
//! | `host`      | the tree host view: layout/render driving, native event -> core input |
//! | `render`    | drawing only — knows nothing about `UIElement`, focus or any control |
//! | `ffi`       | the toolkit seam: the erased native handle (`AnyView`) |
//! | `app`       | dispatcher, app delegate, event-loop entry |
//! | `platform`  | OS services that are not UI elements (file dialogs) |
//!
//! `elwindui-backend-winui3` mirrors this file-for-file; keep the two in step.

#![cfg(target_os = "macos")]
// `#[elwindui_macros::class]`'s `__elwindui_inherit_*!` chain mechanism needs a same-crate
// macro-to-macro reference (`$crate::the_macro!`) to also work cross-crate, which currently
// requires this lint disabled — see `crates/elwindui-macros/src/class.rs`'s own doc comment on
// `inherit_macro_self_ref_path` for the full explanation, and `docs/specs/macro_class_spec.md`.
// Every crate using `#[class]` with a same-crate `inherits` chain needs this same line.
#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

/// Performs process-wide AppKit setup required before creating views.
///
/// AppKit performs this lazily when the application object is created, so this is intentionally
/// idempotent and has one piece of eager work: registering this crate's
/// `elwindui_core::graphics::TextBackend` so `TextBlock::measure_override` (and every
/// `NativeControl::sync_text_style` call) gets real font metrics instead of the core-only
/// deterministic fallback (`DummyTextBackend`). Runs on the main thread — this same guarantee is
/// what every subsequent `measure_text`/`default_text_style` call (always during a main-thread
/// layout pass) relies on to call `NSFont`/`NSFontDescriptor` APIs without its own `mtm()` check.
pub fn init() -> Result<(), std::convert::Infallible> {
    elwindui_core::graphics::set_text_backend(std::rc::Rc::new(render::AppKitTextBackend));
    // Native controls are Core-owned Components whose peers this backend creates
    // (`docs/design/runtime/native_peer_design.md`); register the factory for this UI thread.
    elwindui_core::ui::native::register_native_peer_factory(std::rc::Rc::new(
        native_ui::AppKitPeerFactory,
    ));
    // Self-drawn accent chrome falls back to the system control accent when the app Theme leaves
    // `Primary` unset (`docs/specs/docking_spec.md`). Queried on demand so theme refreshes see
    // accent changes.
    elwindui_core::theme::set_platform_accent_provider(Some(std::rc::Rc::new(|| {
        use objc2_app_kit::{NSColor, NSColorSpace};
        #[allow(unused_unsafe)]
        let rgba = unsafe {
            let accent = NSColor::controlAccentColor();
            let srgb = accent.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
            (
                srgb.redComponent(),
                srgb.greenComponent(),
                srgb.blueComponent(),
                srgb.alphaComponent(),
            )
        };
        let channel = |value: f64| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
        Some(elwindui_core::graphics::Color::rgba(
            channel(rgba.0 as f64),
            channel(rgba.1 as f64),
            channel(rgba.2 as f64),
            channel(rgba.3 as f64),
        ))
    })));
    Ok(())
}

mod app;
#[cfg(feature = "render-stats")]
pub mod diagnostics;
mod ffi;
mod host;
mod inner;
mod native_ui;
pub mod platform;
mod render;

#[cfg(test)]
mod testsupport;

pub use inner::{AppKitPopupHandle, AppKitPopupHost};
pub use native_ui::*;

// `elwindui-codegen`'s generated code references `elwindui::backend::AnyView` directly (see
// `inner::AnyView`'s own doc comment), so it needs to stay reachable at this crate's own root even
// though the rest of `inner` is private.
pub use ffi::AnyView;

/// Re-exported so `elwindui`'s own facade can expose `application::run` uniformly across
/// backends. See `app`'s module doc.
pub mod application {
    pub use crate::app::run;
}

/// Runs the executable AppKit main-thread regression for the native menu-item ownership boundary.
///
/// A Core `MenuItem` added to a Core `Menu` and then dropped by the caller must keep its native
/// `NSMenuItem` and target alive, keep its explicit enabled state, and fire its callback exactly
/// once through the native `NSMenu::performActionForItemAtIndex` path. Hidden from the ordinary
/// backend API; only the focused `menu_lifetime_runtime` example calls it.
#[cfg(all(feature = "menu-lifetime-regression", target_os = "macos"))]
#[doc(hidden)]
pub fn run_menu_lifetime_regression() {
    use elwindui_core::ui::{Menu, MenuExt, MenuItem, MenuItemExt};
    use std::cell::Cell;
    use std::rc::Rc;

    let _ = init();
    application::run(|| {
        let menu = Menu::new();
        let callback_count = Rc::new(Cell::new(0_u32));

        let item = MenuItem::new();
        item.set_text("Lifetime regression");
        item.set_enabled(true);
        let callback_count_for_item = Rc::clone(&callback_count);
        item.set_on_select(Box::new(move || {
            callback_count_for_item.set(callback_count_for_item.get() + 1);
        }));
        menu.items().add(Rc::clone(&item) as Rc<dyn MenuItemExt>);
        drop(item);

        let menu_peer = elwindui_core::ui::__menu_peer_of(&*menu);
        let native_menu = menu_peer
            .as_any()
            .downcast_ref::<native_ui::menu::AppKitMenuPeer>()
            .expect("AppKit menu peer")
            .ns();
        let native_item = native_menu
            .itemAtIndex(0)
            .expect("menu must retain its native item");
        assert!(
            native_item.target().is_some(),
            "native target must remain alive"
        );
        assert!(
            native_item.isEnabled(),
            "explicit enabled state must remain authoritative"
        );

        native_menu.performActionForItemAtIndex(0);
        assert_eq!(
            callback_count.get(),
            1,
            "native menu selection must fire exactly once"
        );

        eprintln!(
            "menu_lifetime_runtime: PASS (native_item_retained=true, callback_count={})",
            callback_count.get()
        );
        drop(menu);

        run_menu_single_parent_regression();

        dispatch2::DispatchQueue::main().exec_async(|| {
            objc2_app_kit::NSApplication::sharedApplication(crate::ffi::mtm()).terminate(None);
        });
    });
}

/// Issue #323 on AppKit: a `MenuItem` that belongs to native `NSMenu` A is rejected by Core
/// before `NSMenu addItem:` could raise its second-parent exception for B, both native menus stay
/// unchanged, and after an explicit `remove` the same `NSMenuItem` moves to B and still fires.
#[cfg(all(feature = "menu-lifetime-regression", target_os = "macos"))]
fn run_menu_single_parent_regression() {
    use elwindui_core::ui::{__menu_peer_of, ListExt, Menu, MenuItem, MenuItemExt};
    use objc2::rc::Retained;
    use objc2_app_kit::{NSMenu, NSMenuItem};
    use std::cell::Cell;
    use std::rc::Rc;

    fn native(menu: &Menu) -> Retained<NSMenu> {
        __menu_peer_of(menu)
            .as_any()
            .downcast_ref::<native_ui::menu::AppKitMenuPeer>()
            .expect("AppKit menu peer")
            .ns()
    }
    fn entries(menu: &NSMenu) -> Vec<Retained<NSMenuItem>> {
        (0..menu.numberOfItems())
            .map(|index| menu.itemAtIndex(index).expect("native entry"))
            .collect()
    }
    fn rejection(action: impl FnOnce()) -> String {
        let error = std::panic::catch_unwind(std::panic::AssertUnwindSafe(action))
            .expect_err("adding an item owned by another live Menu must be rejected");
        error
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| error.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default()
    }

    let menu_a = Menu::new();
    let menu_b = Menu::new();
    let item = MenuItem::new();
    item.set_text("Shared");
    let selected = Rc::new(Cell::new(0_u32));
    let selected_for_callback = Rc::clone(&selected);
    item.set_on_select(Box::new(move || {
        selected_for_callback.set(selected_for_callback.get() + 1)
    }));
    menu_a.add(item.clone());

    let (native_a, native_b) = (native(&menu_a), native(&menu_b));
    let native_item = entries(&native_a).pop().expect("A holds the native entry");
    assert_eq!(entries(&native_a).len(), 1);
    assert!(entries(&native_b).is_empty());

    for message in [
        rejection(|| menu_b.add(item.clone())),
        rejection(|| ListExt::insert(&*menu_b, 0, item.clone())),
    ] {
        assert!(
            message.contains("this `MenuItem` already belongs to another `Menu`"),
            "Core diagnostic: {message}"
        );
    }
    assert_eq!((menu_a.len(), menu_b.len()), (1, 0));
    let a_after = entries(&native_a);
    assert!(
        a_after.len() == 1 && Retained::as_ptr(&a_after[0]) == Retained::as_ptr(&native_item),
        "A's native entries are unchanged"
    );
    assert!(
        entries(&native_b).is_empty(),
        "nothing was added to B natively"
    );
    native_a.performActionForItemAtIndex(0);
    assert_eq!(selected.get(), 1, "A still activates the item once");

    let item_dyn: Rc<dyn MenuItemExt> = item.clone();
    assert!(ListExt::remove(&*menu_a, &item_dyn));
    menu_b.add(item.clone());
    assert!(entries(&native_a).is_empty(), "A detached the native entry");
    let b_after = entries(&native_b);
    assert!(
        b_after.len() == 1 && Retained::as_ptr(&b_after[0]) == Retained::as_ptr(&native_item),
        "B holds the same NSMenuItem"
    );
    native_b.performActionForItemAtIndex(0);
    assert_eq!(selected.get(), 2, "B activates the moved item once");

    eprintln!(
        "menu_single_parent_runtime: PASS (rejected_add_insert=2, native_unchanged=true, moved_same_nsmenuitem=true, select_count={})",
        selected.get()
    );
}

/// Runs the executable AppKit main-thread regression for inactive-host accessibility semantics.
///
/// This is intentionally hidden from the ordinary backend API. It exists so the focused
/// `accessibility_active_host_regression` example can enter AppKit through `application::run`,
/// which is the supported way to make `TreeHost`'s `MainThreadOnly` contract executable.
#[cfg(all(feature = "accessibility-regression", target_os = "macos"))]
#[doc(hidden)]
pub fn run_accessibility_active_host_regression() {
    use elwindui_core::accessibility::{AccessibilityAction, AccessibilityRole};
    use elwindui_core::ui::{ButtonExt, UIElementExt, VerticalLayout};
    use objc2::DefinedClass;
    use objc2_app_kit::NSApplication;
    use std::rc::Rc;

    application::run(|| {
        let actual_button = elwindui_core::ui::Button::new();
        actual_button.set_text("disabled-setter-test");
        actual_button.set_enabled(false);
        let actual_button_id = actual_button.as_ui_element().accessibility_id();
        let actual_button_node: Rc<dyn UIElementExt> = actual_button.clone();
        let actual_button_runtime = elwindui_core::accessibility::AccessibilityRuntime::new();
        let disabled_snapshot = actual_button_runtime.rebuild(&actual_button_node);
        assert!(disabled_snapshot.roots[0].semantics.state.disabled);
        assert!(
            !actual_button_runtime.dispatch_action(actual_button_id, AccessibilityAction::Activate)
        );
        actual_button.set_enabled(true);
        let enabled_snapshot = actual_button_runtime.rebuild(&actual_button_node);
        assert!(!enabled_snapshot.roots[0].semantics.state.disabled);

        let host = host::TreeHost::new();
        let root = VerticalLayout::new();
        root.set_accessibility_role(AccessibilityRole::Button);
        root.set_accessibility_label("inactive-host-test");
        let root_id = root.accessibility_id();
        let root_node: Rc<dyn UIElementExt> = root.clone();
        host.set_tree(root_node);

        let active_snapshot = host.ivars().accessibility_runtime.snapshot();
        assert_eq!(active_snapshot.roots.len(), 1, "active host semantic root");
        assert_eq!(active_snapshot.roots[0].id, root_id);
        assert_eq!(
            host.accessibility_children_count_for_regression(),
            1,
            "active host AX children"
        );

        let stale_element = host.accessibility_element_for(root_id);
        host.set_active(false);
        assert!(
            host.ivars()
                .accessibility_runtime
                .snapshot()
                .roots
                .is_empty(),
            "inactive host snapshot must be empty"
        );
        assert_eq!(
            host.accessibility_children_count_for_regression(),
            0,
            "inactive host AX children must be empty"
        );
        assert!(
            !host
                .ivars()
                .accessibility_runtime
                .dispatch_action(root_id, AccessibilityAction::Activate),
            "stale runtime owner must reject actions"
        );
        let stale_press: bool =
            unsafe { objc2::msg_send![&*stale_element, accessibilityPerformPress] };
        assert!(
            !stale_press,
            "stale synthetic AX element must reject actions"
        );

        root.set_accessibility_label("mutated-while-inactive");
        host.rebuild_accessibility();
        assert!(
            host.ivars()
                .accessibility_runtime
                .snapshot()
                .roots
                .is_empty(),
            "inactive invalidation must not repopulate semantics"
        );

        host.set_active(true);
        let reactivated_snapshot = host.ivars().accessibility_runtime.snapshot();
        assert_eq!(
            reactivated_snapshot.roots.len(),
            1,
            "reactivated semantic root"
        );
        assert_eq!(reactivated_snapshot.roots[0].id, root_id);
        assert_eq!(
            reactivated_snapshot.roots[0].semantics.label.as_deref(),
            Some("mutated-while-inactive")
        );

        eprintln!(
            "accessibility-active-host-regression PASS active=1 inactive=0 stale_action=0 reactivated=1 enabled_setter=PASS id={}",
            root_id.raw()
        );
        dispatch2::DispatchQueue::main().exec_async(|| {
            NSApplication::sharedApplication(crate::ffi::mtm()).terminate(None);
        });
    });
}
