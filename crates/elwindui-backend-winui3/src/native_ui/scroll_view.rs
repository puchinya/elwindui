//! `elwindui::ui::ScrollView` — the `ScrollViewExt` implementation.

use super::NativeControl;
use crate::AnyView;
use crate::inner::InnerScrollView;
use elwindui_core::ui::UIElementExt;
use std::rc::Rc;

/// `content: std::rc::Rc<dyn UIElement>` (`ScrollView`'s `#[class]` declaration, `#[content(content)]`)
/// resolves to `Rc<dyn UIElementExt>` here — the same type every other `visual_children()`/
/// `#[content(..)]` slot in this crate already uses. Mirrors
/// `elwindui_backend_appkit::native_ui::ScrollView`.
#[elwindui_macros::class(struct_only = elwindui_core::ui::ScrollViewExt, inherits = crate::NativeControl)]
pub struct ScrollView {
    inner: InnerScrollView,
}

#[elwindui_macros::class]
impl ScrollView {
    #[inherent]
    pub fn into_any_view(&self) -> AnyView {
        self.inner.handle()
    }

    fn set_content(&self, content: Rc<dyn UIElementExt>) {
        self.inner
            .set_content(content, self.as_ui_element().visual_collection.owner_rc());
    }
    fn set_horizontal_scroll_enabled(&self, enabled: bool) {
        self.inner.set_horizontal_scroll_enabled(enabled);
    }
    fn set_vertical_scroll_enabled(&self, enabled: bool) {
        self.inner.set_vertical_scroll_enabled(enabled);
    }

    /// The content lives in a nested host outside the Visual tree, so it is unmounted (popups
    /// first) when this ScrollView unmounts — see `TreeHost::unmount_hosted_tree`.
    fn on_constructed(&self) {
        let content_host = self.inner.content_host();
        self.add_unmount_hook(Box::new(move || content_host.unmount_hosted_tree()));
    }

    fn construct() -> Self {
        let inner = InnerScrollView::new();
        let handle = inner.handle();
        Self {
            base: NativeControl::construct(handle),
            inner,
        }
    }
}

/// Live XAML assertion helpers (Issue #157): replacing ScrollView content retires the previous
/// content (popup first, unmounted exactly once, logical parent released) before the new content
/// is attached. Run by `inner::button::hosted_xaml_regression_tests`.
#[cfg(test)]
pub(crate) mod hosted_lifecycle_tests {
    use super::*;
    use crate::host::hosted_context_lifecycle_tests::{
        Log, count, open_recording_popup, probe_content,
    };
    use elwindui_core::ui::ScrollViewExt;
    use std::cell::RefCell;

    pub(crate) fn content_replacement_retires_previous_content() {
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        let scroll_view = ScrollView::new();
        let host = scroll_view.inner.content_host();
        let a = probe_content(&log, "A");
        let b = probe_content(&log, "B");
        let c = probe_content(&log, "C");

        scroll_view.set_content(Rc::clone(&a));
        assert!(a.parent().is_some());
        open_recording_popup(&host, &log, "A");
        scroll_view.set_content(Rc::clone(&b));
        assert_eq!(*log.borrow(), ["A popup close", "A unmount"]);
        assert!(
            a.parent().is_none(),
            "replaced content keeps no logical parent"
        );
        assert!(b.parent().is_some());

        open_recording_popup(&host, &log, "B");
        scroll_view.set_content(Rc::clone(&c));
        assert_eq!(log.borrow()[2..], ["B popup close", "B unmount"]);
        assert!(
            host.hosted_tree_for_test()
                .is_some_and(|tree| Rc::ptr_eq(&tree, &c))
        );

        // Setting the same content again is not a replacement.
        scroll_view.set_content(Rc::clone(&c));
        assert_eq!(log.borrow().len(), 4);

        // (e) closing after replacements unmounts only the current content, once.
        open_recording_popup(&host, &log, "C");
        let as_element: Rc<dyn UIElementExt> = scroll_view.clone();
        elwindui_core::ui::unmount_subtree(&as_element);
        elwindui_core::ui::unmount_subtree(&as_element);
        assert_eq!(log.borrow()[4..], ["C popup close", "C unmount"]);
        for name in ["A", "B", "C"] {
            assert_eq!(
                count(&log, &format!("{name} unmount")),
                1,
                "{name} exactly once"
            );
        }
    }

    /// `set_content(B)` while A is published, where A's `on_unmount` sets C on the same
    /// ScrollView: the latest request (C) wins and stays published; B is superseded (never
    /// attached, unmounted once); A unmounts and its popup closes exactly once; no stale parent.
    pub(crate) fn reentrant_replacement_keeps_the_latest_content() {
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        let scroll_view = ScrollView::new();
        let host = scroll_view.inner.content_host();
        let a = probe_content(&log, "A");
        let b = probe_content(&log, "B");
        let c = probe_content(&log, "C");
        scroll_view.set_content(Rc::clone(&a));
        open_recording_popup(&host, &log, "A");

        let weak = Rc::downgrade(&scroll_view);
        let c_for_hook = Rc::clone(&c);
        a.add_unmount_hook(Box::new(move || {
            let scroll_view: Option<Rc<ScrollView>> = weak.upgrade();
            if let Some(scroll_view) = scroll_view {
                scroll_view.set_content(Rc::clone(&c_for_hook));
            }
        }));
        scroll_view.set_content(Rc::clone(&b));

        assert!(
            host.hosted_tree_for_test()
                .is_some_and(|tree| Rc::ptr_eq(&tree, &c)),
            "the reentrant (latest) content stays published"
        );
        assert_eq!(count(&log, "A popup close"), 1);
        assert_eq!(count(&log, "A unmount"), 1);
        assert_eq!(
            count(&log, "B unmount"),
            1,
            "superseded content is unmounted once"
        );
        assert_eq!(count(&log, "C unmount"), 0);
        assert!(a.parent().is_none() && b.parent().is_none());
        assert!(c.parent().is_some());

        let as_element: Rc<dyn UIElementExt> = scroll_view.clone();
        elwindui_core::ui::unmount_subtree(&as_element);
        assert_eq!(count(&log, "C unmount"), 1);
        assert_eq!(count(&log, "A unmount"), 1);
        assert_eq!(count(&log, "B unmount"), 1);
    }
}
