//! The XAML `Slider` and its `ValueChanged` event.

use crate::bindings::Microsoft::UI::Xaml::Controls::Slider as XamlSlider;
use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::RangeBaseValueChangedEventHandler;
use crate::ffi::AnyView;

/// Raw `XamlSlider` — composed by `native_ui::Slider`. `ValueChanged`'s own args carry the new
/// value directly (`args.NewValue()`), so there is no separate `on_change` storage/registration
/// dance the way `InnerToggleSwitch`/`InnerDropdown` need (no re-entrant callback-id indirection
/// required — the handler closure can call straight into whatever `set_on_change` was given, once
/// stored).
pub(crate) struct InnerSlider {
    handle: AnyView,
    xaml: XamlSlider,
}

impl InnerSlider {
    pub(crate) fn new() -> Self {
        let xaml = XamlSlider::new().expect("Slider::new");
        let handle = AnyView::from(xaml.clone());
        Self { handle, xaml }
    }

    pub(crate) fn handle(&self) -> AnyView {
        self.handle.clone()
    }

    pub(crate) fn set_enabled(&self, enabled: bool) {
        let _ = self.xaml.SetIsEnabled(enabled);
    }

    pub(crate) fn set_value(&self, value: f32) {
        let _ = self.xaml.SetValue(value as f64);
    }

    pub(crate) fn set_min(&self, min: f32) {
        let _ = self.xaml.SetMinimum(min as f64);
    }

    pub(crate) fn set_max(&self, max: f32) {
        let _ = self.xaml.SetMaximum(max as f64);
    }

    /// `NSSlider`'s AppKit counterpart fires continuously while dragging by default
    /// (`isContinuous`); `Slider.ValueChanged` already behaves the same way, so no extra setup is
    /// needed here to match.
    pub(crate) fn set_on_change(&self, callback: Box<dyn Fn(f32)>) {
        // `args` arrives as `&Option<RangeBaseValueChangedEventArgs>` — same shape
        // `inner/tab_view.rs`'s own `TabCloseRequested` handler already unwraps via `.cloned()`.
        let _ = self.xaml.ValueChanged(&RangeBaseValueChangedEventHandler::new(
            move |_sender, args| {
                if let Some(args) = args.cloned() {
                    if let Ok(new_value) = args.NewValue() {
                        callback(new_value as f32);
                    }
                }
                Ok(())
            },
        ));
    }
}
