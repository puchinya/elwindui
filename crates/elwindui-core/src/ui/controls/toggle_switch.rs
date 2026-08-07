//! `builtin::ToggleSwitch` — a native on/off switch.

/// `builtin::ToggleSwitch` — a native on/off switch (AppKit: `NSSwitch`, macOS 10.15+; WinUI3:
/// `ToggleSwitch`).
///
/// **Has no `text` property.** Neither `NSSwitch` nor a bare Fluent `ToggleSwitch` carries a
/// label the way `Button`/`CheckBox`/`RadioButton` do — pair it with an adjacent `TextBlock`
/// (`HorizontalLayout { ToggleSwitch { .. } TextBlock { text: ".." } }`) the same way a `Slider`
/// or any other unlabeled control would be.
#[elwindui_macros::class(trait_only, inherits = crate::ui::NativeControl, sealed)]
#[prop(two_way, is_on: bool)]
#[prop(enabled: Option<bool>)]
pub trait ToggleSwitch {
    fn set_is_on(&self, is_on: bool);
    fn set_on_change(&self, callback: Box<dyn Fn(bool)>);
    fn set_enabled(&self, enabled: bool);
}
