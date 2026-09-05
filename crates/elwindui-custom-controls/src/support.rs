use super::core::ui::UIElementExt;
use std::rc::{Rc, Weak};

/// Recovers the concrete self handle from the Visual collection's owner weak reference.
///
/// Generated component storage is intentionally private to the #[component] expansion. The
/// collection owner is the same most-derived Rc used to initialize that storage, so this keeps
/// callback wiring weak without depending on generated implementation fields that are not part
/// of the rust-analyzer shadow surface.
pub(crate) fn weak_self_from_visual_owner<T: UIElementExt + 'static>(value: &T) -> Weak<T> {
    let owner: Option<Rc<dyn UIElementExt>> = value.as_ui_element().visual_collection.owner_rc();
    let Some(owner) = owner else {
        return Weak::<T>::new();
    };
    assert!(
        owner.as_any().is::<T>(),
        "component Visual collection owner has an unexpected concrete type"
    );
    let raw = Rc::into_raw(owner) as *const () as *const T;
    // SAFETY: the owner was checked against the exact T type above. Reconstructing this
    // temporary Rc<T> preserves the original strong count; it is dropped normally after the
    // weak callback handle is derived.
    let owner = unsafe { Rc::from_raw(raw) };
    let weak = Rc::downgrade(&owner);
    drop(owner);
    weak
}

#[cfg(test)]
mod tests {
    use super::super::CustomTabView;
    use super::super::core::ui::Control;
    use super::*;

    #[test]
    fn weak_self_helper_does_not_leak_owner_strong_reference() {
        let owner = Control::new();
        let original = Rc::downgrade(&owner);
        let strong_before = Rc::strong_count(&owner);

        let returned = weak_self_from_visual_owner(owner.as_ref());

        assert!(returned.upgrade().is_some());
        assert_eq!(Rc::strong_count(&owner), strong_before);

        drop(returned.upgrade());
        drop(owner);

        assert!(original.upgrade().is_none());
        assert!(returned.upgrade().is_none());
    }

    #[test]
    fn weak_self_helper_does_not_leak_component_owner_strong_reference() {
        let owner = CustomTabView::new_view();
        let original = Rc::downgrade(&owner);
        let strong_before = Rc::strong_count(&owner);
        let returned = weak_self_from_visual_owner(owner.as_ref());

        assert_eq!(Rc::strong_count(&owner), strong_before);
        drop(owner);

        assert!(original.upgrade().is_none());
        assert!(returned.upgrade().is_none());
    }
}
