use crate::core::ui::UIElementExt;
use std::rc::{Rc, Weak};

/// Recovers the concrete owner handle from the weak owner stored by UIElementCollection. This is
/// the same weak-only boundary used by custom-controls and keeps Docking callbacks acyclic.
pub(crate) fn weak_self_from_visual_owner<T: UIElementExt + 'static>(value: &T) -> Weak<T> {
    let Some(owner) = value.as_ui_element().visual_collection.owner_rc() else {
        return Weak::new();
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
