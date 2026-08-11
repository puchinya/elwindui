//! `elwindui::ui::PasswordBox` — masked single-line entry.

/// Secure single-line text input (masked entry). Deliberately narrower than `TextBox`: no
/// `read_only`/`text_alignment` (neither platform's password widget meaningfully supports them the
/// way a text field does, and adding dead setters would misrepresent what's actually usable), and
/// no `selection_start`/`selection_length` (same rationale as `TextBox`, doubly so here — selection
/// semantics on obscured text are rarely product-relevant). The field/method is named `password`,
/// not `text`, everywhere (trait, `#[class]` declaration, backend structs) — a deliberate naming
/// divergence from `TextBox` so nothing can accidentally get routed through a code path that
/// assumes plaintext display is fine. See `docs/status/control_status.md` for the
/// `reveal_enabled` AppKit/WinUI3 asymmetry this control has (WinUI3's `PasswordRevealMode` is
/// native; AppKit's `NSSecureTextField` has no equivalent, so `true` is a documented no-op there).
#[elwindui_macros::class(trait_only, inherits = crate::ui::NativeControl, sealed)]
#[prop(two_way, password: String)]
#[prop(placeholder: Option<String>)]
#[prop(max_length: Option<u32>)]
#[prop(reveal_enabled: Option<bool>)]
pub trait PasswordBox {
    fn set_password(&self, password: &str);
    fn set_on_change(&self, callback: Box<dyn Fn(String)>);
    fn set_placeholder(&self, text: &str);
    fn set_max_length(&self, max_length: Option<u32>);
    fn set_reveal_enabled(&self, enabled: bool);
}
