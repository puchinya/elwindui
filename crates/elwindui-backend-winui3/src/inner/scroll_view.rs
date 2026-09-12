//! `ScrollViewer` and its per-axis scrollbar visibility.

use crate::bindings::Microsoft::UI::Xaml::Controls::{ScrollMode, ScrollViewer};
use crate::bindings::Microsoft::UI::Xaml::SizeChangedEventHandler;
use crate::ffi::{AnyView, invoke_ui_event_callback, register_ui_event_callback};
use crate::host::{TreeHost, TreeHostViewport};
use std::cell::Cell;
use std::rc::Rc;

/// Raw `ScrollViewer` + nested `TreeHost` (`ElwinduiContentRoot`) — composed by
/// `native_ui::ScrollView`. See `elwindui_core::ui::ScrollView`'s own doc comment for the
/// `ScrollView -> NativeScrollHost -> ElwinduiContentRoot -> content` structure this implements.
/// Structurally mirrors `elwindui-backend-appkit::inner::InnerScrollView`; unverified on this
/// machine (no Windows environment — see `docs/status/control_status.md`).
/// `content_host` is a second, independent `TreeHost` instance — the same nested-hosting
/// pattern `InnerTabView::insert_tab`'s own per-tab `TreeHost::new()` already establishes, not
/// a one-off special case. Unlike AppKit (where a plain `NSAutoresizingMaskOptions` bit keeps the
/// cross axis tracking the clip view automatically, no notification/event wiring needed), WinUI3's
/// `Canvas` has no autoresizing equivalent — its viewport must be pushed in explicitly via
/// `TreeHost::set_viewport`, the same pattern `InnerTabView::insert_tab`'s own doc comment
/// documents for `TabViewItem.Content`, this being the `ScrollViewer` content-host viewport
/// authority (Issue #261 review remediation §2.4).
pub(crate) struct InnerScrollView {
    handle: AnyView,
    scroll_viewer: ScrollViewer,
    content_host: TreeHost,
    /// `(horizontal_scroll_enabled, vertical_scroll_enabled)` — see
    /// `elwindui_backend_appkit::inner::InnerScrollView::axes`'s own doc comment for the naming
    /// rationale. `Rc<Cell<..>>`, not a plain `Cell<..>`, so the `SizeChanged` closure below can
    /// read the current value at fire time rather than a snapshot from construction.
    axes: Rc<Cell<(bool, bool)>>,
}

impl InnerScrollView {
    pub(crate) fn new() -> Self {
        let scroll_viewer = ScrollViewer::new().expect("ScrollViewer::new");
        let content_host = TreeHost::new();
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

    /// Applies `axes` to the native scroll-mode properties, then immediately re-syncs the cross
    /// axis viewport — needed here too (not just from the `SizeChanged` handler above), since
    /// toggling an axis at runtime via `set_horizontal_scroll_enabled`/`set_vertical_scroll_enabled`
    /// doesn't itself fire `SizeChanged`. `sync_scroll_view_cross_axis` below is the sole place
    /// that encodes which axis is unconstrained, via `TreeHostViewport`'s own `None` — there is no
    /// separate "unconstrained axes" state to keep in sync with it.
    fn apply_axes(&self) {
        let (horizontal, vertical) = self.axes.get();
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
        sync_scroll_view_cross_axis(
            &self.content_host,
            &self.scroll_viewer,
            (horizontal, vertical),
        );
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

/// Pushes `scroll_viewer`'s own current viewport size into `content_host` as the constrained
/// cross axis, encoding whichever axis *does* scroll as `TreeHostViewport`'s `None` (unconstrained)
/// — the `ScrollViewer` content-host viewport authority (Issue #261 review remediation §2.4).
/// Shared by `InnerScrollView::new`'s `SizeChanged` handler and `InnerScrollView::apply_axes`,
/// rather than duplicated between them.
pub(crate) fn sync_scroll_view_cross_axis(
    content_host: &TreeHost,
    scroll_viewer: &ScrollViewer,
    (horizontal, vertical): (bool, bool),
) {
    let _ = content_host.set_viewport(TreeHostViewport {
        width: if horizontal {
            None
        } else {
            Some(scroll_viewer.ActualWidth().unwrap_or(0.0))
        },
        height: if vertical {
            None
        } else {
            Some(scroll_viewer.ActualHeight().unwrap_or(0.0))
        },
    });
}
