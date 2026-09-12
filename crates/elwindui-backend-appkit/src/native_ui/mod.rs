//! The public façade: one class per builtin this backend provides, each implementing the
//! matching `elwindui_core::ui` `*Ext` trait by delegating to its `crate::inner` twin.
//!
//! Deliberately free of AppKit calls — every bit of genuinely toolkit-specific complexity
//! lives one layer down in `inner`. That boundary is why this layer is ~70% identical between
//! the two backends and is the natural candidate if it is ever shared outright.
//!
//! `VerticalLayout`/`HorizontalLayout`/`Rectangle`/`Ellipse`/`TextBlock` have no type here at
//! all: they are plain `elwindui_core::ui::UIElement` values that `elwindui-codegen` builds
//! directly, reflected into native views by `crate::host`.

// Deliberately *not* `use elwindui_core::base::AsAny;` here — see the doc comment on
// `MenuBarItem::set_submenu` (the one place that pattern is explained in full) for why importing
// `AsAny` directly, rather than relying on it as `MenuBarItemExt`/`MenuExt`/etc.'s own supertrait,
// silently breaks every `.as_any().downcast_ref::<T>()` call in this file.

// `#[class(inherits = crate::NativeControl)]` in the submodules below expands its supertrait
// bound to `crate::NativeControlExt`, so that trait has to be nameable at *this crate's*
// root. `NativeControl` is a `struct_only` class whose trait lives in elwindui-core, so it
// is re-exported explicitly here and lifted to the root by `lib.rs`'s `pub use native_ui::*`.
pub use elwindui_core::ui::NativeControlExt;

pub(crate) fn base_accessibility_semantics(
    role: elwindui_core::accessibility::AccessibilityRole,
    actions: &[elwindui_core::accessibility::AccessibilityActionKind],
) -> elwindui_core::accessibility::AccessibilitySemantics {
    let mut semantics = elwindui_core::accessibility::AccessibilitySemantics::new(role);
    semantics.actions = actions.to_vec();
    semantics
}

pub(crate) fn sync_intrinsic_enabled(node: &dyn elwindui_core::ui::UIElementExt, enabled: bool) {
    let Some(mut semantics) = node.intrinsic_accessibility_semantics() else {
        return;
    };
    semantics.state.disabled = !enabled;
    node.set_intrinsic_accessibility_semantics(semantics);
}

#[cfg(test)]
mod tests {
    use super::*;
    use elwindui_core::accessibility::{
        AccessibilityAction, AccessibilityActionKind, AccessibilityRole,
    };
    use elwindui_core::ui::{UIElementExt, VerticalLayout};
    use std::cell::Cell;
    use std::rc::Rc;

    #[test]
    fn disabled_intrinsic_semantics_reject_actions_until_reenabled() {
        let node = VerticalLayout::new();
        let mut semantics = base_accessibility_semantics(
            AccessibilityRole::Button,
            &[AccessibilityActionKind::Activate],
        );
        node.set_intrinsic_accessibility_semantics(semantics.clone());
        let calls = Rc::new(Cell::new(0));
        node.set_on_accessibility_action(Box::new({
            let calls = calls.clone();
            move |_| calls.set(calls.get() + 1)
        }));

        sync_intrinsic_enabled(node.as_ui_element(), false);
        semantics = node
            .intrinsic_accessibility_semantics()
            .expect("intrinsic semantics");
        assert!(semantics.state.disabled);
        let runtime = elwindui_core::accessibility::AccessibilityRuntime::new();
        let owner: Rc<dyn UIElementExt> = node.clone();
        let id = runtime.rebuild(&owner).roots[0].id;
        assert!(!runtime.dispatch_action(id, AccessibilityAction::Activate));
        assert_eq!(calls.get(), 0);

        sync_intrinsic_enabled(node.as_ui_element(), true);
        assert!(
            !node
                .intrinsic_accessibility_semantics()
                .expect("intrinsic semantics")
                .state
                .disabled
        );
        assert!(runtime.dispatch_action(id, AccessibilityAction::Activate));
        assert_eq!(calls.get(), 1);
    }
}

mod button;
mod check_box;
mod control;
mod dropdown;
mod dropdown_item;
mod menu;
mod radio_button;
mod scroll_view;
mod slider;
mod tab_view;
mod text;
mod toggle_switch;
mod window;

// Glob re-exports, not a named list: `#[class]` generates a companion `__elwindui_macros_of_*`
// path alias next to each class, which downstream `#[component(inherits ..)]` resolves as
// `elwindui::ui::__elwindui_macros_of_Window`. Naming only the types here would leave those
// aliases behind in the submodule and break every inheriting user component.
pub use button::*;
pub use check_box::*;
pub use control::*;
pub use dropdown::*;
pub use dropdown_item::*;
pub use menu::*;
pub use radio_button::*;
pub use scroll_view::*;
pub use slider::*;
pub use tab_view::*;
pub use text::*;
pub use toggle_switch::*;
pub use window::*;
