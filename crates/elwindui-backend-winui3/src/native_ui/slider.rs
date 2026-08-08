//! `builtin::Slider` — the `SliderExt` implementation. Mirrors
//! `elwindui_backend_appkit::native_ui::slider`'s own structure exactly (unverified — no Windows
//! environment).

use super::NativeControl;
use crate::AnyView;
use crate::inner::InnerSlider;
use elwindui_core::ui::UIElementExt;

#[elwindui_macros::class(struct_only = elwindui_core::ui::SliderExt, inherits = crate::NativeControl)]
pub struct Slider {
    inner: InnerSlider,
}

#[elwindui_macros::class]
impl Slider {
    #[inherent]
    pub fn into_any_view(&self) -> AnyView {
        self.inner.handle()
    }

    fn set_value(&self, value: f32) {
        self.inner.set_value(value);
    }
    fn set_on_change(&self, callback: Box<dyn Fn(f32)>) {
        self.inner.set_on_change(callback);
    }
    fn set_min(&self, min: f32) {
        self.inner.set_min(min);
    }
    fn set_max(&self, max: f32) {
        self.inner.set_max(max);
    }
    fn set_enabled(&self, enabled: bool) {
        self.inner.set_enabled(enabled);
    }

    fn construct() -> Self {
        let inner = InnerSlider::new();
        let handle = inner.handle();
        Self {
            base: NativeControl::construct(handle),
            inner,
        }
    }

    fn on_constructed(&self) {
        self.set_tab_stop(true);
    }

    /// `codegen` calls exactly `set_on_{field}_change` for a `#[two_way]` prop.
    #[inherent]
    pub fn set_on_value_change(&self, callback: Box<dyn Fn(f32)>) {
        self.inner.set_on_change(callback);
    }
}
