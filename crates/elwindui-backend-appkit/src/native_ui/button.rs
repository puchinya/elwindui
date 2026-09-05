//! `elwindui::ui::Button` — the `ButtonExt` implementation.

use super::NativeControl;
use crate::AnyView;
use crate::inner::InnerButton;
use elwindui_core::ui::UIElementExt;
use std::rc::Rc;

#[elwindui_macros::class(struct_only = elwindui_core::ui::ButtonExt, inherits = crate::NativeControl)]
pub struct Button {
    inner: InnerButton,
}

#[elwindui_macros::class]
impl Button {
    #[overrides]
    fn measure_override(&self, available: elwindui_core::base::Size) -> elwindui_core::base::Size {
        self.base.measure_with_text_style_refresh(available)
    }

    /// `#[routed] on_click` (`Button`'s `#[class]` declaration) is registered directly onto this
    /// widget's own `base` — real since construction (see `new`), and already wired (also in `new`)
    /// to fire `dispatch_routed` starting at this same node.
    #[inherent]
    pub fn register_routed_handler<T: 'static>(
        &self,
        name: &'static str,
        handler: Box<dyn Fn(&T, &elwindui_core::input::RoutedEventArgs)>,
    ) {
        self.base
            .as_ui_element()
            .register_routed_handler(name, handler);
    }

    #[inherent]
    pub fn into_any_view(&self) -> AnyView {
        self.inner.handle()
    }

    fn set_enabled(&self, enabled: bool) {
        self.inner.set_enabled(enabled);
    }
    fn set_on_click(&self, callback: Box<dyn Fn()>) {
        self.inner.set_on_click(callback);
    }
    fn set_text(&self, text: &str) {
        self.inner.set_text(text);
        self.base.reapply_text_style();
        // `NSButton.setTitle` changes the fitting size. Resync can set the same authored title
        // again after a sibling's layout was replaced, so invalidate unconditionally to ensure
        // the button is measured and painted back into the containing layout group.
        self.base.invalidate_measure();
    }
    fn set_role(&self, role: elwindui_core::ui::ButtonRole) {
        self.inner.set_role(role);
    }
    fn set_is_default(&self, is_default: bool) {
        self.inner.set_is_default(is_default);
    }

    fn construct() -> Self {
        let inner = InnerButton::new();
        let handle = inner.handle();
        Self {
            base: NativeControl::construct(handle),
            inner,
        }
    }

    fn on_constructed(&self) {
        // WinUI3's `Button` is a tab stop by default — see
        // docs/design/runtime/input_focus_design.md.
        self.set_tab_stop(true);
        // Wires the real `NSButton` click directly to `dispatch_routed`, once, right here, rather
        // than re-detecting/re-wiring it on every relayout. Unconditional — `dispatch_routed`
        // already no-ops gracefully when nothing is registered for `"on_click"` at this node or any
        // ancestor (`elwindui-codegen`'s `emit_wiring` registers the actual `#[routed] on_click`
        // handler here, via `register_routed_handler` above, right after this constructor returns).
        // `owner_rc()` is guaranteed `Some` here — `on_constructed` only ever runs once the
        // enclosing `Rc` is fully built.
        let node: Rc<dyn UIElementExt> = self
            .as_ui_element()
            .visual_collection
            .owner_rc()
            .expect("Button::on_constructed: object must already be Rc-constructed");
        self.inner.set_on_click(Box::new(move || {
            let args = elwindui_core::input::RoutedEventArgs::default();
            elwindui_core::ui::dispatch_routed(&node, "on_click", &(), &args);
        }));
    }
}
