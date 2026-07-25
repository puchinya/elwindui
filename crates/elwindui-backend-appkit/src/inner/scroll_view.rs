//! `NSScrollView` and its per-axis scroller enablement.

use crate::ffi::{AnyView, mtm};
use crate::host::TreeHostView;
use elwindui_core::ui::UIElementExt;
use objc2::rc::Retained;
use objc2_app_kit::NSScrollView;
use std::cell::Cell;
use std::rc::Rc;

/// Raw `NSScrollView` + nested `TreeHostView` (`ElwinduiContentRoot`) — composed by
/// `native_ui::ScrollView`. See `elwindui_core::ui::ScrollView`'s own doc comment for the
/// `ScrollView -> NativeScrollHost -> ElwinduiContentRoot -> content` structure this implements.
/// `content_host` is a second, independent `TreeHostView` instance — the same nested-hosting
/// pattern `InnerTabView::insert_tab`'s own per-tab `TreeHostView::new()` already establishes, not a
/// one-off special case — with its own `set_tree`, its own `AppKitRelayoutHost`/`AppKitFocusHost`
/// registration (falls out of `TreeHostView::set_tree` unchanged, no new focus-chain code needed:
/// `ElwinduiWindow::make_first_responder`'s responder-chain walk already finds *any* `TreeHostView`
/// ancestor, nested ones included), and — the one genuinely new piece — `unconstrained_axes` set on
/// whichever axis scrolls, so that axis measures/arranges at its true natural size instead of being
/// clamped to the viewport.
pub(crate) struct InnerScrollView {
    handle: AnyView,
    scroll: Retained<NSScrollView>,
    content_host: Retained<TreeHostView>,
    /// `(horizontal_scroll_enabled, vertical_scroll_enabled)` — mirrors the `.elwind`-visible
    /// property names directly (unlike `TreeHostIvars::unconstrained_axes`, which is phrased as
    /// "width/height unconstrained" — the same booleans, just named from the opposite perspective:
    /// scrolling enabled on an axis is exactly what makes that axis unconstrained).
    axes: Cell<(bool, bool)>,
}

impl InnerScrollView {
    pub(crate) fn new() -> Self {
        let m = mtm();
        let scroll = NSScrollView::new(m);
        let content_host = TreeHostView::new();
        content_host.setTranslatesAutoresizingMaskIntoConstraints(true);
        scroll.setDocumentView(Some(&content_host));
        let handle = AnyView::from(scroll.clone());
        let this = Self {
            handle,
            scroll,
            content_host,
            // Vertical-only scrolling by default — matches both platforms' own scroll-widget
            // defaults and the overwhelmingly common product case (`ScrollView` in
            // `builtins.elwind`'s own default).
            axes: Cell::new((false, true)),
        };
        this.apply_axes();
        this
    }

    /// Applies `axes` to the native scroller visibility, `content_host`'s own unconstrained-measure
    /// axes, and its autoresizing mask — the "classic pre-Auto-Layout fill" technique
    /// `InnerTabView::insert_tab`/`InnerWindow::new` already use elsewhere in this file, here used
    /// to track the *non*-scrolling axis (axes) to the scroll view's own viewport size automatically
    /// on every resize, no `NSNotificationCenter` observation needed: a scrolling axis gets no
    /// autoresizing bit at all (so it stays whatever size `relayout`'s own unconstrained measurement
    /// just set it to), while a non-scrolling axis keeps its `ViewWidthSizable`/`ViewHeightSizable`
    /// bit so AppKit's ordinary autoresizing machinery keeps that axis synced to the clip view.
    fn apply_axes(&self) {
        let (horizontal, vertical) = self.axes.get();
        self.content_host.set_unconstrained_axes(horizontal, vertical);
        let mut mask = objc2_app_kit::NSAutoresizingMaskOptions::ViewNotSizable;
        if !horizontal {
            mask |= objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable;
        }
        if !vertical {
            mask |= objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable;
        }
        self.content_host.setAutoresizingMask(mask);
        self.scroll.setHasHorizontalScroller(horizontal);
        self.scroll.setHasVerticalScroller(vertical);
    }

    pub(crate) fn handle(&self) -> AnyView {
        self.handle.clone()
    }

    pub(crate) fn set_content(&self, content: Rc<dyn UIElementExt>) {
        self.content_host.set_tree(content);
    }

    pub(crate) fn set_horizontal_scroll_enabled(&self, enabled: bool) {
        let (_, vertical) = self.axes.get();
        self.axes.set((enabled, vertical));
        self.apply_axes();
    }

    pub(crate) fn set_vertical_scroll_enabled(&self, enabled: bool) {
        let (horizontal, _) = self.axes.get();
        self.axes.set((horizontal, enabled));
        self.apply_axes();
    }
}
