//! `elwindui::ui::Dropdown` — a native, non-editable selection control (AppKit: `NSPopUpButton`;
//! WinUI3: `ComboBox`).

use super::*;

/// `Dropdown`'s class trait (docs/design/gui_framework_design.md §5.1). Its content is a live,
/// ordered collection of `DropdownItem`s — mirrors `TabView`'s own `children` shape exactly.
/// `selected_index` is the single source of truth for which item is selected (no per-item
/// `selected` flag — see `docs/specs/ui_spec.md#dropdown`).
#[elwindui_macros::class(trait_only, inherits = crate::ui::NativeControl, sealed)]
#[content(items)]
#[prop(items: Vec<std::rc::Rc<dyn crate::ui::DropdownItemExt>>)]
#[prop(two_way, selected_index: usize)]
#[prop(enabled: Option<bool>)]
pub trait Dropdown {
    fn items(&self) -> &dyn ListExt<dyn DropdownItemExt>;
    fn set_selected_index(&self, selected_index: usize);
    fn set_on_change(&self, callback: Box<dyn Fn(usize)>);
    fn set_enabled(&self, enabled: bool);
}
