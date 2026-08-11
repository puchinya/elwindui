//! `elwindui::ui::Slider` — a native continuous-value slider.

/// `elwindui::ui::Slider` — a native continuous-value slider (AppKit: `NSSlider`; WinUI3: `Slider`).
///
/// **Has no `text` property**, the same as `ToggleSwitch` — pair it with an adjacent `TextBlock`
/// if a label is needed.
///
/// `min`/`max` are runtime-mutable `#[prop]`s (not `#[param]`), so a `component` can change a
/// `Slider`'s range reactively rather than only at construction time. `value` is clamped to
/// `min..=max` by the native widget itself, not by elwindui.
#[elwindui_macros::class(trait_only, inherits = crate::ui::NativeControl, sealed)]
#[prop(two_way, value: f32)]
#[prop(min: f32)]
#[prop(max: f32)]
#[prop(enabled: Option<bool>)]
pub trait Slider {
    fn set_value(&self, value: f32);
    fn set_on_change(&self, callback: Box<dyn Fn(f32)>);
    fn set_min(&self, min: f32);
    fn set_max(&self, max: f32);
    fn set_enabled(&self, enabled: bool);
}
