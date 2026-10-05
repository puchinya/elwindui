use crate::core::environment::application_environment;
use crate::core::graphics::Brush;
use crate::core::theme::{BrushStyle, ResolvedValue};
use crate::core::ui::UIElementExt;
use std::rc::{Rc, Weak};

/// Recovers the concrete owner handle from the weak owner stored by UIElementCollection. This is
/// the same weak-only boundary used by custom-controls and keeps Docking callbacks acyclic.
pub(crate) fn weak_self_from_visual_owner<T: UIElementExt + 'static>(value: &T) -> Weak<T> {
    let owner: Option<Rc<dyn UIElementExt>> = value.as_ui_element().visual_collection.owner_rc();
    let Some(owner) = owner else {
        return Weak::<T>::new();
    };
    assert!(
        owner.as_any().is::<T>(),
        "unexpected Docking visual owner type"
    );
    let raw = Rc::into_raw(owner) as *const () as *const T;
    let owner = unsafe { Rc::from_raw(raw) };
    let weak = Rc::downgrade(&owner);
    drop(owner);
    weak
}

/// Resolves a runtime-owned brush through the application's current Theme. Runtime chrome is
/// created programmatically, so it uses the same semantic roles as declarative surfaces instead
/// of embedding a second palette in the docking crate.
pub(crate) fn themed_brush(style: BrushStyle) -> Option<Brush> {
    match style.resolve(&application_environment()) {
        ResolvedValue::Value(brush) => Some(brush),
        ResolvedValue::PlatformDefault => None,
    }
}

/// Active/accent chrome brush: the Theme's `Primary` when it resolves to a value, otherwise the
/// platform accent (`docs/specs/docking_spec.md`). Self-drawn accent chrome must not disappear
/// when the application leaves `Primary` to the platform default.
pub(crate) fn accent_brush() -> Brush {
    themed_brush(BrushStyle::Primary)
        .or_else(|| crate::core::theme::platform_accent_color().map(Brush::Solid))
        .unwrap_or_else(|| Brush::Solid(crate::core::graphics::Color::rgb(0, 120, 215)))
}
