//! `builtin::TextBox` — single-line text entry.

use super::*;

/// Single-line text input — see `docs/status/nativecontrol_status.md` for the wider
/// NativeControl expansion this is part of. Deliberately narrower than the original spec's
/// `TextBox` sketch: `selection_start`/`selection_length` are not included (AppKit's `NSTextField`
/// selection lives on its *field editor*, which only exists while actively being edited — a shared
/// method here would either be AppKit-only-half-working or need a `NativeControl`-wide "is there an
/// active editor right now" concept this codebase doesn't have yet; revisit once a second consumer
/// needs it). `submit`-on-Enter is likewise not a dedicated trait method — `UIElement`'s existing
/// `on_key_down` (`#[routed]`) already covers it the same way any other element's own key handling
/// would (see `TextBox`'s `#[class]` declaration and `native_ui::TextBox::on_constructed`'s own doc
/// comment on why AppKit needs one narrow, TextBox-specific addition to make that work in practice).
#[elwindui_macros::class(trait_only, inherits = crate::ui::NativeControl, sealed)]
#[prop(two_way, text: String)]
#[prop(placeholder: Option<String>)]
#[prop(read_only: Option<bool>)]
#[prop(max_length: Option<u32>)]
#[prop(text_alignment: Option<crate::ui::TextAlignment>)]
pub trait TextBox {
    fn set_text(&self, text: &str);
    fn set_on_change(&self, callback: Box<dyn Fn(String)>);
    fn set_placeholder(&self, text: &str);
    fn set_read_only(&self, read_only: bool);
    fn set_max_length(&self, max_length: Option<u32>);
    fn set_text_alignment(&self, alignment: TextAlignment);
}
