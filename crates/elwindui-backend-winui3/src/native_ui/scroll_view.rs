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
        Log, capture_events, count, open_recording_popup, probe_content,
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

    /// `set_content(B)` while A is published, where A's first extra `on_unmount` hook sets C on the
    /// same ScrollView and a second hook runs after it: nothing is attached and no native child is
    /// detached until A's last hook has run (the second hook still sees A published); then the
    /// latest request (C) is attached, B is superseded (never attached, unmounted once), A unmounts
    /// and its popup closes exactly once; no stale parent.
    pub(crate) fn reentrant_replacement_keeps_the_latest_content() {
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        let _events = capture_events(&log);
        let scroll_view = ScrollView::new();
        let host = scroll_view.inner.content_host();
        let a = probe_content(&log, "A");
        let b = probe_content(&log, "B");
        let c = probe_content(&log, "C");
        scroll_view.set_content(Rc::clone(&a));
        open_recording_popup(&host, &log, "A");

        let weak = Rc::downgrade(&scroll_view);
        let c_for_hook = Rc::clone(&c);
        let log_for_hook = Rc::clone(&log);
        a.add_unmount_hook(Box::new(move || {
            log_for_hook.borrow_mut().push("A hook sets C".to_owned());
            let scroll_view: Option<Rc<ScrollView>> = weak.upgrade();
            if let Some(scroll_view) = scroll_view {
                scroll_view.set_content(Rc::clone(&c_for_hook));
            }
        }));
        let host_for_hook = host.clone();
        let a_for_hook = Rc::downgrade(&a);
        let log_for_hook = Rc::clone(&log);
        a.add_unmount_hook(Box::new(move || {
            let still_a = match (host_for_hook.hosted_tree_for_test(), a_for_hook.upgrade()) {
                (Some(tree), Some(a)) => Rc::ptr_eq(&tree, &a),
                _ => false,
            };
            log_for_hook
                .borrow_mut()
                .push(format!("A last hook (A still published: {still_a})"));
        }));
        log.borrow_mut().clear();
        scroll_view.set_content(Rc::clone(&b));

        let events = log.borrow().clone();
        let position = |entry: &str| events.iter().position(|e| e == entry);
        let last_hook = position("A last hook (A still published: true)")
            .unwrap_or_else(|| panic!("A's last hook must still see A published: {events:?}"));
        let detach = position("native children detach").expect("A's native children detach");
        assert!(position("A hook sets C").unwrap() < last_hook);
        assert!(
            last_hook < detach,
            "native detach follows A's last hook: {events:?}"
        );
        assert!(
            host.hosted_tree_for_test()
                .is_some_and(|tree| Rc::ptr_eq(&tree, &c)),
            "the latest content stays published"
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

    /// `set_content(B)` while A is published, where A's `on_unmount` sets the same B again: B is
    /// published once and is never unmounted while live; owner close unmounts it exactly once.
    pub(crate) fn reentrant_same_content_is_not_unmounted() {
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        let scroll_view = ScrollView::new();
        let host = scroll_view.inner.content_host();
        let a = probe_content(&log, "A");
        let b = probe_content(&log, "B");
        scroll_view.set_content(Rc::clone(&a));

        let weak = Rc::downgrade(&scroll_view);
        let b_for_hook = Rc::clone(&b);
        a.add_unmount_hook(Box::new(move || {
            let scroll_view: Option<Rc<ScrollView>> = weak.upgrade();
            if let Some(scroll_view) = scroll_view {
                scroll_view.set_content(Rc::clone(&b_for_hook));
            }
        }));
        scroll_view.set_content(Rc::clone(&b));

        assert!(
            host.hosted_tree_for_test()
                .is_some_and(|tree| Rc::ptr_eq(&tree, &b)),
            "B is published"
        );
        assert_eq!(count(&log, "B unmount"), 0, "live B is never unmounted");
        assert_eq!(count(&log, "A unmount"), 1);
        assert!(b.parent().is_some(), "B keeps its owner link");
        // The host's relayout / input / focus wiring is installed by `set_tree` and cleared by
        // `unmount`, so its presence shows B is attached and live.
        assert!(
            b.as_ui_element().invalidate_host.borrow().is_some(),
            "B keeps its host wiring (relayout / input)"
        );

        let as_element: Rc<dyn UIElementExt> = scroll_view.clone();
        elwindui_core::ui::unmount_subtree(&as_element);
        elwindui_core::ui::unmount_subtree(&as_element);
        assert_eq!(
            count(&log, "B unmount"),
            1,
            "owner close unmounts B exactly once"
        );
    }

    /// Makes `content`'s unmount call `set_content(next)` on `scroll_view`.
    fn on_unmount_set(
        content: &Rc<dyn UIElementExt>,
        scroll_view: &Rc<ScrollView>,
        next: &Rc<dyn UIElementExt>,
    ) {
        let weak = Rc::downgrade(scroll_view);
        let next = Rc::clone(next);
        content.add_unmount_hook(Box::new(move || {
            let scroll_view: Option<Rc<ScrollView>> = weak.upgrade();
            if let Some(scroll_view) = scroll_view {
                scroll_view.set_content(Rc::clone(&next));
            }
        }));
    }

    fn published(host: &crate::host::TreeHost, content: &Rc<dyn UIElementExt>) -> bool {
        host.hosted_tree_for_test()
            .is_some_and(|tree| Rc::ptr_eq(&tree, content))
    }

    /// Start `set_content(B)` with A published; A's `on_unmount` requests C; superseded B's
    /// `on_unmount` requests D. The newest request (D) is attached once all teardown is done;
    /// A, B and C unmount exactly once; D unmounts only when the owner closes. Variant: B's hook
    /// requests C again (same `Rc`) — C is attached and never unmounted while live.
    pub(crate) fn superseded_hooks_cannot_override_the_newest_request() {
        for same_rc in [false, true] {
            let log: Log = Rc::new(RefCell::new(Vec::new()));
            let scroll_view = ScrollView::new();
            let host = scroll_view.inner.content_host();
            let a = probe_content(&log, "A");
            let b = probe_content(&log, "B");
            let c = probe_content(&log, "C");
            let d = if same_rc {
                Rc::clone(&c)
            } else {
                probe_content(&log, "D")
            };
            scroll_view.set_content(Rc::clone(&a));
            open_recording_popup(&host, &log, "A");
            on_unmount_set(&a, &scroll_view, &c);
            on_unmount_set(&b, &scroll_view, &d);

            scroll_view.set_content(Rc::clone(&b));

            let label = if same_rc { "C/D same Rc" } else { "distinct D" };
            assert!(
                published(&host, &d),
                "{label}: the newest request is published"
            );
            assert_eq!(count(&log, "A popup close"), 1, "{label}");
            assert_eq!(count(&log, "A unmount"), 1, "{label}");
            assert_eq!(count(&log, "B unmount"), 1, "{label}");
            if same_rc {
                assert_eq!(
                    count(&log, "C unmount"),
                    0,
                    "{label}: live C never unmounted"
                );
            } else {
                assert_eq!(count(&log, "C unmount"), 1, "{label}");
                assert_eq!(count(&log, "D unmount"), 0, "{label}");
            }
            assert!(
                a.parent().is_none() && b.parent().is_none(),
                "{label}: no stale parent"
            );
            assert!(d.parent().is_some(), "{label}: D linked to its owner");
            assert!(
                d.as_ui_element().invalidate_host.borrow().is_some(),
                "{label}: D wired to the host"
            );

            let as_element: Rc<dyn UIElementExt> = scroll_view.clone();
            elwindui_core::ui::unmount_subtree(&as_element);
            let final_name = if same_rc { "C" } else { "D" };
            assert_eq!(
                count(&log, &format!("{final_name} unmount")),
                1,
                "{label}: owner close"
            );
            assert!(
                host.hosted_tree_for_test().is_none(),
                "{label}: host empty after close"
            );
        }
    }

    /// Owner close with A published; A's `on_unmount` requests B; B's `on_unmount` requests C.
    /// Nothing is re-attached at any stage: the host ends empty, A, B and C unmount exactly once,
    /// no logical parent is left, and later requests are rejected (unmounted, never attached).
    pub(crate) fn owner_close_never_reattaches_reentrant_requests() {
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        let scroll_view = ScrollView::new();
        let host = scroll_view.inner.content_host();
        let a = probe_content(&log, "A");
        let b = probe_content(&log, "B");
        let c = probe_content(&log, "C");
        scroll_view.set_content(Rc::clone(&a));
        open_recording_popup(&host, &log, "A");
        on_unmount_set(&a, &scroll_view, &b);
        on_unmount_set(&b, &scroll_view, &c);

        let as_element: Rc<dyn UIElementExt> = scroll_view.clone();
        elwindui_core::ui::unmount_subtree(&as_element);

        assert!(host.hosted_tree_for_test().is_none(), "host stays empty");
        assert_eq!(count(&log, "A popup close"), 1);
        for name in ["A", "B", "C"] {
            assert_eq!(
                count(&log, &format!("{name} unmount")),
                1,
                "{name} exactly once"
            );
        }
        assert!(a.parent().is_none() && b.parent().is_none() && c.parent().is_none());

        let late = probe_content(&log, "late");
        scroll_view.set_content(Rc::clone(&late));
        assert!(
            host.hosted_tree_for_test().is_none(),
            "a closed host never re-attaches"
        );
        assert_eq!(
            count(&log, "late unmount"),
            1,
            "rejected content is unmounted once"
        );
    }

    /// Content torn down during a transition is never re-published, even when the newest request
    /// names it again. (1) A→B, A.on_unmount→C, B.on_unmount→A: the host ends empty; A, B, C
    /// unmount once; A keeps no parent / host wiring. (2) A superseded content re-requested by a
    /// later hook: A→B, A→C, B→D, C→B: the newest request (B) is dead, so the host ends empty and
    /// A, B, C, D unmount once. (3) The same chain during owner close keeps the host closed and
    /// empty. Afterwards a direct request for a retired content is ignored.
    pub(crate) fn retired_content_is_never_republished() {
        // (1)
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        let scroll_view = ScrollView::new();
        let host = scroll_view.inner.content_host();
        let a = probe_content(&log, "A");
        let b = probe_content(&log, "B");
        let c = probe_content(&log, "C");
        scroll_view.set_content(Rc::clone(&a));
        open_recording_popup(&host, &log, "A");
        on_unmount_set(&a, &scroll_view, &c);
        on_unmount_set(&b, &scroll_view, &a);
        scroll_view.set_content(Rc::clone(&b));
        assert!(
            host.hosted_tree_for_test().is_none(),
            "(1) dead A is not re-published"
        );
        assert_eq!(count(&log, "A popup close"), 1);
        for name in ["A", "B", "C"] {
            assert_eq!(
                count(&log, &format!("{name} unmount")),
                1,
                "(1) {name} once"
            );
        }
        assert!(a.parent().is_none() && b.parent().is_none() && c.parent().is_none());
        assert!(
            a.as_ui_element().invalidate_host.borrow().is_none(),
            "(1) A has no host wiring"
        );
        // A later direct request for retired A is ignored; a live content still works.
        scroll_view.set_content(Rc::clone(&a));
        assert!(host.hosted_tree_for_test().is_none());
        assert_eq!(count(&log, "A unmount"), 1);
        let e = probe_content(&log, "E");
        scroll_view.set_content(Rc::clone(&e));
        assert!(
            published(&host, &e),
            "(1) a live content is attached afterwards"
        );

        // (2)
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        let scroll_view = ScrollView::new();
        let host = scroll_view.inner.content_host();
        let a = probe_content(&log, "A");
        let b = probe_content(&log, "B");
        let c = probe_content(&log, "C");
        let d = probe_content(&log, "D");
        scroll_view.set_content(Rc::clone(&a));
        on_unmount_set(&a, &scroll_view, &c);
        on_unmount_set(&b, &scroll_view, &d);
        on_unmount_set(&c, &scroll_view, &b);
        scroll_view.set_content(Rc::clone(&b));
        assert!(
            host.hosted_tree_for_test().is_none(),
            "(2) dead B is not re-published"
        );
        for name in ["A", "B", "C", "D"] {
            assert_eq!(
                count(&log, &format!("{name} unmount")),
                1,
                "(2) {name} once"
            );
        }

        // (3)
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        let scroll_view = ScrollView::new();
        let host = scroll_view.inner.content_host();
        let a = probe_content(&log, "A");
        let b = probe_content(&log, "B");
        scroll_view.set_content(Rc::clone(&a));
        on_unmount_set(&a, &scroll_view, &b);
        on_unmount_set(&b, &scroll_view, &a);
        let as_element: Rc<dyn UIElementExt> = scroll_view.clone();
        elwindui_core::ui::unmount_subtree(&as_element);
        assert!(
            host.hosted_tree_for_test().is_none(),
            "(3) closed host stays empty"
        );
        for name in ["A", "B"] {
            assert_eq!(
                count(&log, &format!("{name} unmount")),
                1,
                "(3) {name} once"
            );
        }
    }
}
