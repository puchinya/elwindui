//! `elwindui::ui::RadioButton` — a native mutually-exclusive selection button.

/// `elwindui::ui::RadioButton` — a native mutually-exclusive selection button (AppKit: `NSButton` with
/// `NSButtonType::Radio`; WinUI3: `RadioButton`).
///
/// **Grouping is managed by elwindui, not by either native toolkit.** AppKit's own automatic
/// radio grouping only applies to buttons that share both a superview *and* an action selector —
/// a condition this framework's per-instance click trampoline never satisfies, so nothing needs
/// suppressing there. Instead, every `RadioButton` sharing the same non-empty `group` string is
/// tracked together (backend-side; see `docs/status/control_status.md` for where), and
/// checking one un-checks every other member of its group. A `RadioButton` with no `group` (the
/// default) participates in no exclusivity at all — it behaves like a plain two-state toggle.
#[elwindui_macros::class(trait_only, inherits = crate::ui::NativeControl, sealed)]
#[prop(text: String)]
#[prop(two_way, checked: bool)]
#[prop(group: Option<String>)]
#[prop(enabled: Option<bool>)]
pub trait RadioButton {
    fn set_text(&self, text: &str);
    fn set_checked(&self, checked: bool);
    fn set_on_change(&self, callback: Box<dyn Fn(bool)>);
    fn set_group(&self, group: &str);
    fn set_enabled(&self, enabled: bool);
}
