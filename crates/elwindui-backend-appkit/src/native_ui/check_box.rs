//! `elwindui::ui::CheckBox` — the `CheckBoxExt` implementation.

use super::NativeControl;
use crate::AnyView;
use crate::inner::InnerCheckBox;
use elwindui_core::ui::{CheckState, UIElementExt};

#[elwindui_macros::class(struct_only = elwindui_core::ui::CheckBoxExt, inherits = crate::NativeControl)]
pub struct CheckBox {
    inner: InnerCheckBox,
}

#[elwindui_macros::class]
impl CheckBox {
    #[inherent]
    pub fn into_any_view(&self) -> AnyView {
        self.inner.handle()
    }

    fn set_text(&self, text: &str) {
        self.inner.set_text(text);
        self.base.reapply_text_style();
    }
    fn set_checked(&self, checked: CheckState) {
        self.inner.set_checked(checked);
    }
    fn set_on_change(&self, callback: Box<dyn Fn(CheckState)>) {
        self.inner.set_on_change(callback);
    }
    fn set_enabled(&self, enabled: bool) {
        self.inner.set_enabled(enabled);
    }

    fn construct() -> Self {
        let inner = InnerCheckBox::new();
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
    pub fn set_on_checked_change(&self, callback: Box<dyn Fn(CheckState)>) {
        self.inner.set_on_change(callback);
    }
}
