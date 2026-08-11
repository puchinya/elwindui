//! `elwindui::ui::TextArea` — multi-line text entry.

#[elwindui_macros::class(trait_only, inherits = crate::ui::NativeControl, sealed)]
#[prop(two_way, text: String)]
pub trait TextArea {
    fn set_text(&self, text: &str);
    fn set_on_change(&self, callback: Box<dyn Fn(String)>);
}
