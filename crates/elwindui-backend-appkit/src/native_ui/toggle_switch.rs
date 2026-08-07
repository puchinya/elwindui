//! `builtin::ToggleSwitch` — the `ToggleSwitchExt` implementation.

use super::NativeControl;
use crate::AnyView;
use crate::inner::InnerToggleSwitch;
use elwindui_core::ui::UIElementExt;

#[elwindui_macros::class(struct_only = elwindui_core::ui::ToggleSwitchExt, inherits = crate::NativeControl)]
pub struct ToggleSwitch {
    inner: InnerToggleSwitch,
}

#[elwindui_macros::class]
impl ToggleSwitch {
    #[inherent]
    pub fn into_any_view(&self) -> AnyView {
        self.inner.handle()
    }

    fn set_is_on(&self, is_on: bool) {
        self.inner.set_is_on(is_on);
    }
    fn set_on_change(&self, callback: Box<dyn Fn(bool)>) {
        self.inner.set_on_change(callback);
    }
    fn set_enabled(&self, enabled: bool) {
        self.inner.set_enabled(enabled);
    }

    fn construct() -> Self {
        let inner = InnerToggleSwitch::new();
        let handle = inner.handle();
        Self {
            base: NativeControl::construct(handle),
            inner,
        }
    }

    fn on_constructed(&self) {
        self.set_tab_stop(true);
    }

    /// `#[two_way] is_on` — the change-back half of the binding; `elwindui_core::ui::ToggleSwitch::
    /// set_is_on` is the model→widget half. Mirrors `TextBox::set_on_text_change`'s own naming.
    #[inherent]
    pub fn set_on_is_on_change(&self, callback: Box<dyn Fn(bool)>) {
        self.inner.set_on_change(callback);
    }
}
