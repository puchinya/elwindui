//! `builtin::ScrollView` — the `ScrollViewExt` implementation.

use super::NativeControl;
use crate::AnyView;
use crate::inner::InnerScrollView;
use elwindui_core::ui::UIElementExt;
use std::rc::Rc;

/// `content: std::rc::Rc<dyn UIElement>` (`ScrollView` in `builtins.elwind`, `#[content(content)]`)
/// resolves to `Rc<dyn UIElementExt>` here — the same type every other `visual_children()`/
/// `#[content(..)]` slot in this crate already uses.
#[elwindui_macros::class(struct_only = elwindui_core::ui::ScrollViewExt, inherits = crate::NativeControl)]
pub struct ScrollView {
    inner: InnerScrollView,
}

#[elwindui_macros::class]
impl ScrollView {
    #[inherent]
    pub fn into_any_view(&self) -> AnyView {
        self.inner.handle()
    }

    fn set_content(&self, content: Rc<dyn UIElementExt>) {
        self.inner.set_content(content);
    }
    fn set_horizontal_scroll_enabled(&self, enabled: bool) {
        self.inner.set_horizontal_scroll_enabled(enabled);
    }
    fn set_vertical_scroll_enabled(&self, enabled: bool) {
        self.inner.set_vertical_scroll_enabled(enabled);
    }

    fn construct() -> Self {
        let inner = InnerScrollView::new();
        let handle = inner.handle();
        Self {
            base: NativeControl::construct(handle),
            inner,
        }
    }
}
