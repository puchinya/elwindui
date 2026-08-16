//! `elwindui::ui::MenuItem` — one entry in a `Menu`.

#[elwindui_macros::class(trait_only)]
#[prop(text: String)]
#[prop(shortcut: Option<String>)]
#[prop(enabled: Option<bool>)]
#[prop(on_select: fn())]
pub trait MenuItem {
    fn text(&self) -> String;
    fn set_text(&self, text: &str);
    fn enabled(&self) -> bool;
    fn set_enabled(&self, enabled: bool);
    fn shortcut(&self) -> Option<String>;
    fn set_shortcut(&self, key_equivalent: &str);
    fn set_on_select(&self, callback: Box<dyn Fn()>);
    fn select(&self);
}
