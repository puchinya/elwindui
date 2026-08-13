//! `elwindui::ui::Button` — the native push button.

/// How much emphasis a [`Button`] carries, and what it implies about the action behind it.
///
/// Mapped to each toolkit's *own* emphasis affordance rather than to elwindui-drawn styling, so a
/// `Primary` button looks like the platform's accent button and follows the user's system accent
/// colour instead of a colour this framework picked (AppKit `NSButton.bezelColor` /
/// `hasDestructiveAction`, WinUI3 `AccentButtonStyle`). There is deliberately no per-role override
/// beyond this: a role-specific background would fight the system accent colour users expect to
/// control themselves. An explicit `background:` still wins, because `NativeControl`'s background
/// is applied independently of — and after — the role's bezel.
///
/// Orthogonal to [`Button`]'s `is_default`: a role says what kind of action this is, `is_default`
/// says whether Return activates it. A destructive button can be the default one, and often should
/// not be.
///
/// # Example
///
/// ```ignore
/// Button {
///     text: "Delete"
///     role: elwindui::core::ui::ButtonRole::Destructive
///     on_click: vm.delete
/// }
/// ```
///
/// The fully qualified path is required: the DSL resolves an enum-typed property's value as an
/// ordinary Rust expression, so a bare `Destructive` does not resolve. Shorthand would need
/// enum-aware value resolution in `elwindui-codegen`, which does not exist today.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ButtonRole {
    /// An ordinary, unemphasized action — the platform's plain push button. The default.
    #[default]
    Normal,
    /// The action this view is primarily for, drawn with the platform's accent fill.
    Primary,
    /// An action that destroys or discards something, drawn with the platform's destructive
    /// treatment (red text on AppKit). Marks intent — it does not add a confirmation step.
    Destructive,
}

/// `elwindui::ui::Button`. The `#[dsl(..)]`/`#[dsl_prop(..)]` lines below are this element's DSL-visible
/// surface, declared here on the interface itself rather than duplicated in a separate compiler-side
/// shape table — `#[class]` turns them into `__elwindui_shape_Button!`, which the generated view
/// code invokes (see `build_shape_macro` for why the shape has to reach consumers as a macro).
///
/// This *is* the native button — there is no separate `NativeButton` type. `role`/`is_default`
/// were added here rather than to a parallel control (the open question
/// `docs/status/control_status.md` used to record) because everything they need is a
/// property of the same `NSButton`/`Button` widget this already wraps.
///
/// `tooltip` is not declared here: it comes from `NativeControl`, so every native leaf has it.
#[elwindui_macros::class(trait_only, inherits = crate::ui::NativeControl, sealed)]
#[prop(text: String)]
#[prop(enabled: Option<bool>)]
#[prop(role: Option<crate::ui::ButtonRole>)]
#[prop(is_default: Option<bool>)]
#[prop(routed, on_click: fn())]
pub trait Button {
    fn set_enabled(&self, enabled: bool);
    fn set_on_click(&self, callback: Box<dyn Fn()>);
    fn set_text(&self, text: &str);

    /// Applies a [`ButtonRole`]'s native emphasis treatment, replacing whatever the previous role
    /// applied. Implementations must fully reset the other roles' effects, so setting `Normal`
    /// returns the button to the plain platform appearance.
    fn set_role(&self, role: ButtonRole);

    /// Makes this the window's default button — the one Return activates, drawn by the platform
    /// with whatever "this is the default" cue it uses. Setting it on more than one button in the
    /// same window is a caller mistake neither toolkit resolves for you.
    fn set_is_default(&self, is_default: bool);
}
