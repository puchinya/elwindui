//! `ScrollViewer` and its per-axis scrollbar visibility.

use crate::ffi::{AnyView, register_ui_event_callback, invoke_ui_event_callback};
use crate::host::TreeHostPanel;
use crate::bindings::Microsoft::UI::Xaml::Controls::{
    ScrollMode, ScrollViewer,
};
use crate::bindings::Microsoft::UI::Xaml::SizeChangedEventHandler;
use std::cell::Cell;
use std::rc::Rc;

/// Raw `ScrollViewer` + nested `TreeHostPanel` (`ElwinduiContentRoot`) — composed by
/// `native_ui::ScrollView`. See `elwindui_core::ui::ScrollView`'s own doc comment for the
/// `ScrollView -> NativeScrollHost -> ElwinduiContentRoot -> content` structure this implements.
/// Structurally mirrors `elwindui-backend-appkit::inner::InnerScrollView`; unverified on this
/// machine (no Windows environment — see `docs/status/control_status.md`).
/// `content_host` is a second, independent `TreeHostPanel` instance — the same nested-hosting
/// pattern `InnerTabView::insert_tab`'s own per-tab `TreeHostPanel::new()` already establishes, not
/// a one-off special case. Unlike AppKit (where a plain `NSAutoresizingMaskOptions` bit keeps the
/// cross axis tracking the clip view automatically, no notification/event wiring needed), WinUI3's
/// `Canvas` has no autoresizing equivalent — its `Width`/`Height` must be pushed in explicitly,
/// the exact same issue (and the exact same `SizeChanged` + `force_relayout` fix)
/// `InnerTabView::insert_tab`'s own doc comment already documents for `TabViewItem.Content`.
pub(crate) struct InnerScrollView {
    handle: AnyView,
    scroll_viewer: ScrollViewer,
    content_host: TreeHostPanel,
    /// `(horizontal_scroll_enabled, vertical_scroll_enabled)` — see
    /// `elwindui_backend_appkit::inner::InnerScrollView::axes`'s own doc comment for the naming
    /// rationale (same booleans `TreeHostPanel::unconstrained_axes` uses, phrased from the opposite
    /// perspective). `Rc<Cell<..>>`, not a plain `Cell<..>`, so the `SizeChanged` closure below can
    /// read the current value at fire time rather than a snapshot from construction — the same
    /// reason `TreeHostPanel::unconstrained_axes` itself is `Rc`-wrapped.
    axes: Rc<Cell<(bool, bool)>>,
}

impl InnerScrollView {
    pub(crate) fn new() -> Self {
        let scroll_viewer = ScrollViewer::new().expect("ScrollViewer::new");
        let content_host = TreeHostPanel::new();
        let _ = scroll_viewer.SetContent(&content_host.as_element());
        let handle = AnyView::from(scroll_viewer.clone());
        // Vertical-only scrolling by default — matches `ScrollView`'s own `#[class]` declaration and its
        // default.
        let axes = Rc::new(Cell::new((false, true)));
        let this = Self {
            handle,
            scroll_viewer,
            content_host,
            axes,
        };
        this.apply_axes();
        {
            let content_host_for_handler = this.content_host.clone();
            let scroll_viewer_for_handler = this.scroll_viewer.clone();
            let axes_for_handler = this.axes.clone();
            let callback_id = register_ui_event_callback(Rc::new(move || {
                sync_scroll_view_cross_axis(
                    &content_host_for_handler,
                    &scroll_viewer_for_handler,
                    axes_for_handler.get(),
                );
            }));
            let _ = this
                .scroll_viewer
                .SizeChanged(&SizeChangedEventHandler::new(move |_, _| {
                    invoke_ui_event_callback(callback_id);
                    Ok(())
                }));
        }
        this
    }

    /// Applies `axes` to the native scroll-mode properties and `content_host`'s own
    /// unconstrained-measure axes, then immediately re-syncs the cross axis and force-relays-out —
    /// needed here too (not just from the `SizeChanged` handler above), since toggling an axis at
    /// runtime via `set_horizontal_scroll_enabled`/`set_vertical_scroll_enabled` doesn't itself fire
    /// `SizeChanged`.
    fn apply_axes(&self) {
        let (horizontal, vertical) = self.axes.get();
        self.content_host.set_unconstrained_axes(horizontal, vertical);
        let _ = self.scroll_viewer.SetHorizontalScrollMode(if horizontal {
            ScrollMode::Auto
        } else {
            ScrollMode::Disabled
        });
        let _ = self.scroll_viewer.SetVerticalScrollMode(if vertical {
            ScrollMode::Auto
        } else {
            ScrollMode::Disabled
        });
        sync_scroll_view_cross_axis(&self.content_host, &self.scroll_viewer, (horizontal, vertical));
    }

    pub(crate) fn handle(&self) -> AnyView {
        self.handle.clone()
    }

    pub(crate) fn set_content(&self, content: Rc<dyn elwindui_core::ui::UIElementExt>) {
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

/// Pushes `scroll_viewer`'s own current viewport size into `content_host`'s explicit `Width`/
/// `Height` on whichever axis does *not* scroll, resets the scrolling axis/axes back to the
/// `NaN` ("unset") sentinel (so a stale explicit size doesn't linger across a runtime axis-toggle —
/// `relayout_static`'s own `explicit_width.is_finite()` check, this function's counterpart), and
/// force-relays-out. Shared by `InnerScrollView::new`'s `SizeChanged` handler and
/// `InnerScrollView::apply_axes`, rather than duplicated between them.
pub(crate) fn sync_scroll_view_cross_axis(
    content_host: &TreeHostPanel,
    scroll_viewer: &ScrollViewer,
    (horizontal, vertical): (bool, bool),
) {
    let element = content_host.as_element();
    if horizontal {
        let _ = element.SetWidth(f64::NAN);
    } else {
        let _ = element.SetWidth(scroll_viewer.ActualWidth().unwrap_or(0.0));
    }
    if vertical {
        let _ = element.SetHeight(f64::NAN);
    } else {
        let _ = element.SetHeight(scroll_viewer.ActualHeight().unwrap_or(0.0));
    }
    content_host.force_relayout();
}
