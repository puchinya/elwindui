//! `builtin::Button` — the native push button.

/// `builtin::Button`. The `#[dsl(..)]`/`#[dsl_prop(..)]` lines below are this builtin's DSL-visible
/// surface, declared here on the interface itself rather than duplicated in a separate compiler-side
/// shape table — `#[class]` turns them into `__elwindui_shape_Button!`, which the generated view
/// code invokes (see `build_shape_macro` for why the shape has to reach consumers as a macro).
#[elwindui_macros::class(trait_only, inherits = crate::ui::NativeControl, sealed)]
#[prop(text: String)]
#[prop(enabled: Option<bool>)]
#[prop(routed, on_click: fn())]
pub trait Button {
    fn set_enabled(&self, enabled: bool);
    fn set_on_click(&self, callback: Box<dyn Fn()>);
    fn set_text(&self, text: &str);
}
