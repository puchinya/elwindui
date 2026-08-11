//! `elwindui::ui::TabViewItem` — one tab of a `TabView`.

/// `TabViewItem`'s own class trait. No `inherits`: like `Window`,
/// `TabViewItem` is never itself embedded as a real `Rc<dyn UIElement>` node (see its own
/// `#[class]` doc comment), so it has no meaningful `NativeControl`/`UIElement` ancestor.
#[elwindui_macros::class(trait_only, sealed)]
#[prop(header: String)]
#[content(content)]
#[prop(content: std::rc::Rc<dyn crate::ui::UIElementExt>)]
#[prop(closable: Option<bool>)]
#[prop(on_close: fn())]
pub trait TabViewItem {}
