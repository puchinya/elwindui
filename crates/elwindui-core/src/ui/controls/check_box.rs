//! `builtin::CheckBox` — a native tri-state checkbox.

/// A [`CheckBox`]'s check state.
///
/// Three states exist because `NSButton`/`Checkbox` genuinely support a mixed/indeterminate
/// display (e.g. "some but not all items in this group are selected"), but only two of them are
/// reachable by a user click — see [`CheckBox`]'s own doc comment for why `Indeterminate` is
/// program-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CheckState {
    #[default]
    Unchecked,
    Checked,
    /// A mixed/partial state. Never produced by a user click — only ever reached by setting
    /// `checked: CheckState::Indeterminate` from a `component`.
    Indeterminate,
}

/// `builtin::CheckBox` — a native tri-state checkbox (AppKit: `NSButton` with
/// `NSButtonType::Switch`; WinUI3: `CheckBox`).
///
/// **User interaction only ever toggles between `Unchecked` and `Checked`.** Both backends
/// disable native mixed-state cycling (AppKit `setAllowsMixedState(false)`) so a click can never
/// land on `Indeterminate` — that third state exists purely so a `component` can *display* "some
/// but not all" (e.g. a header checkbox reflecting a partially-selected list) without a user
/// being able to get stuck in it by clicking. Setting `checked: CheckState::Indeterminate` from
/// Rust still works; the next click always moves to `Checked` from any prior state, matching how
/// this two-state cycle already behaves without ever reading the mixed value.
#[elwindui_macros::class(trait_only, inherits = crate::ui::NativeControl, sealed)]
#[prop(text: String)]
#[prop(two_way, checked: crate::ui::CheckState)]
#[prop(enabled: Option<bool>)]
pub trait CheckBox {
    fn set_text(&self, text: &str);
    fn set_checked(&self, checked: crate::ui::CheckState);
    fn set_on_change(&self, callback: Box<dyn Fn(crate::ui::CheckState)>);
    fn set_enabled(&self, enabled: bool);
}
