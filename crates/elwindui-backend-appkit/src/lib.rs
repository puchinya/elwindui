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

/// Runs the executable AppKit main-thread regression for inactive-host accessibility semantics.
///
/// This is intentionally hidden from the ordinary backend API. It exists so the focused
/// `accessibility_active_host_regression` example can enter AppKit through `application::run`,
/// which is the supported way to make `TreeHostView`'s `MainThreadOnly` contract executable.
#[cfg(all(feature = "accessibility-regression", target_os = "macos"))]
#[doc(hidden)]
pub fn run_accessibility_active_host_regression() {
    use elwindui_core::accessibility::{AccessibilityAction, AccessibilityRole};
    use elwindui_core::ui::{ButtonExt, UIElementExt, VerticalLayout};
    use objc2::DefinedClass;
    use objc2_app_kit::NSApplication;
    use std::rc::Rc;

    application::run(|| {
        let actual_button = Button::new();
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

        let host = host::TreeHostView::new();
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
