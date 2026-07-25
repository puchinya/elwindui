//! `TabView` and its per-tab content hosting.

use crate::ffi::{AnyView, register_ui_event_callback, invoke_ui_event_callback, register_ui_index_event_callback, invoke_ui_index_event_callback};
use crate::host::TreeHostPanel;
use crate::bindings;
use crate::bindings::Microsoft::UI::Xaml::Controls::{
    TabView as XamlTabView, TabViewCloseButtonOverlayMode, TabViewItem,
    TabViewTabCloseRequestedEventArgs,
};
use crate::bindings::Microsoft::UI::Xaml::SizeChangedEventHandler;
use crate::bindings::Microsoft::UI::Xaml::Controls::SelectionChangedEventHandler;
use windows::Foundation::{PropertyValue, TypedEventHandler};
use std::cell::RefCell;
use std::rc::Rc;
use windows::core::{HSTRING, Interface};

/// See docs/elwindui_builtins_spec.md 付録Y. `Microsoft.UI.Xaml.Controls.TabView` is a real native
/// tabbed-document control (unlike AppKit, which has none — `elwindui_backend_appkit::inner`'s
/// `TabStripImpl`/`TabChipImpl` hand-roll one from `Button`s), so this wraps it directly instead of
/// assembling a strip from scratch. Each tab's `TabViewItem.Content` is a `TreeHostPanel` holding
/// that tab's whole widget tree — composed by `native_ui::TabView`, which owns the mapping from
/// `items_source`/static `TabViewItem`s to entries; this type only knows about "N tabs, each with a
/// title and a content host", the same division AppKit's `InnerTabView` keeps.
pub(crate) struct InnerTabView {
    handle: AnyView,
    xaml: XamlTabView,
    on_select: Rc<RefCell<Option<Box<dyn Fn(usize)>>>>,
    on_close: Rc<RefCell<Option<Box<dyn Fn(usize)>>>>,
    on_new_tab: Rc<RefCell<Option<Box<dyn Fn()>>>>,
    /// Every tab's own content `TreeHostPanel` (`insert_tab`'s `content_host`), so `TabView`'s own
    /// `SizeChanged` (below) can keep every one of them sized *and freshly relaid-out* to match —
    /// not just the currently selected tab's — since an unselected tab may never itself receive a
    /// `SizeChanged` (its `ActualWidth`/`ActualHeight` may simply never be assigned at all while
    /// not the visible content), unlike `TabView` itself, which reliably resizes with the window.
    /// See `insert_tab`'s own doc comment for why this is needed at all, and
    /// `TreeHostPanel::force_relayout`'s for why setting the size alone isn't enough either.
    content_hosts: Rc<RefCell<Vec<TreeHostPanel>>>,
}

// `TabView` lays out each item content below its tab strip, but the manually
// sized TreeHostPanel is otherwise given the TabView's full height. Reserve the
// native strip height so a custom-drawn card keeps its lower margin and rounded
// corners inside the content presenter instead of being clipped by the window.
pub(crate) const TAB_VIEW_CONTENT_TOP_INSET: f64 = 40.0;

impl InnerTabView {
    pub(crate) fn new() -> Self {
        let xaml = XamlTabView::new().expect("NativeTabView::new");
        let _ = xaml.SetTabWidthMode(
            bindings::Microsoft::UI::Xaml::Controls::TabViewWidthMode::SizeToContent,
        );
        let _ = xaml.SetCloseButtonOverlayMode(TabViewCloseButtonOverlayMode::Always);
        let _ = xaml.SetIsAddTabButtonVisible(true);

        let handle = AnyView::from(xaml.clone());
        let this = Self {
            handle,
            xaml,
            on_select: Rc::new(RefCell::new(None)),
            on_close: Rc::new(RefCell::new(None)),
            on_new_tab: Rc::new(RefCell::new(None)),
            content_hosts: Rc::new(RefCell::new(Vec::new())),
        };

        {
        let content_hosts = this.content_hosts.clone();
        let xaml_for_resize = this.xaml.clone();
        let callback_id = register_ui_event_callback(Rc::new(move || {
            let width = xaml_for_resize.ActualWidth().unwrap_or(0.0);
            let height = (xaml_for_resize.ActualHeight().unwrap_or(0.0)
                - TAB_VIEW_CONTENT_TOP_INSET)
                .max(0.0);
            for content_host in content_hosts.borrow().iter() {
                let element = content_host.as_element();
                let _ = element.SetWidth(width);
                let _ = element.SetHeight(height);
                // Setting `Width`/`Height` alone (even with `InvalidateMeasure`/`InvalidateArrange`)
                // does not make this `Canvas`'s own `SizeChanged` fire on any later frame either —
                // confirmed by logging `ActualWidth`/`ActualHeight` immediately after, which stayed
                // `0` — so relayout can't be left to arrive on its own; force it directly instead.
                content_host.force_relayout();
            }
        }));
        let _ = this.xaml.SizeChanged(&SizeChangedEventHandler::new(move |_, _| {
            invoke_ui_event_callback(callback_id);
            Ok(())
        }));
        }

        {
        let on_select = this.on_select.clone();
        let callback_id = register_ui_index_event_callback(Rc::new(move |index| {
            if let Some(callback) = on_select.borrow().as_ref() { callback(index); }
        }));
        let _ = this.xaml.SelectionChanged(&SelectionChangedEventHandler::new(move |sender, _| {
            if let Some(sender) = sender.cloned().and_then(|sender| sender.cast::<XamlTabView>().ok()) {
                let index = sender.SelectedIndex().unwrap_or(-1);
                if index >= 0 {
                    invoke_ui_index_event_callback(callback_id, index as usize);
                }
            }
            Ok(())
        }));
        }

        {
        let on_close = this.on_close.clone();
        let callback_id = register_ui_index_event_callback(Rc::new(move |index| {
            if let Some(callback) = on_close.borrow().as_ref() { callback(index); }
        }));
        let _ = this.xaml.TabCloseRequested(&TypedEventHandler::<
            XamlTabView,
            TabViewTabCloseRequestedEventArgs,
        >::new(move |sender, args| {
            if let (Some(sender), Some(args)) = (
                sender.cloned().and_then(|sender| sender.cast::<XamlTabView>().ok()),
                args.cloned(),
            ) {
                if let Ok(items) = sender.TabItems() {
                    if let Ok(item) = args.Tab() {
                        let mut index = 0;
                        let item: windows::core::IInspectable = item.into();
                        if items.IndexOf(&item, &mut index).unwrap_or(false) {
                            invoke_ui_index_event_callback(callback_id, index as usize);
                        }
                    }
                }
            }
            Ok(())
        }));
        }

        {
        let on_new_tab = this.on_new_tab.clone();
        let callback_id = register_ui_event_callback(Rc::new(move || {
            if let Some(callback) = on_new_tab.borrow().as_ref() { callback(); }
        }));
        let _ = this
            .xaml
            .AddTabButtonClick(&TypedEventHandler::new(move |_, _| {
                invoke_ui_event_callback(callback_id);
                Ok(())
            }));
        }

        this
    }

    pub(crate) fn handle(&self) -> AnyView {
        self.handle.clone()
    }

    pub(crate) fn set_on_select(&self, callback: Box<dyn Fn(usize)>) {
        *self.on_select.borrow_mut() = Some(callback);
    }

    pub(crate) fn set_on_close(&self, callback: Box<dyn Fn(usize)>) {
        *self.on_close.borrow_mut() = Some(callback);
    }

    pub(crate) fn set_on_new_tab(&self, callback: Box<dyn Fn()>) {
        *self.on_new_tab.borrow_mut() = Some(callback);
    }

    pub(crate) fn insert_tab(&self, index: usize, title: &str, closable: bool) -> TreeHostPanel {
        let content_host = TreeHostPanel::new();
        let item = TabViewItem::new().expect("TabViewItem::new");
        if let Ok(value) = PropertyValue::CreateString(&HSTRING::from(title)) {
            let _ = item.SetHeader(&value);
        }
        let _ = item.SetIsClosable(closable);
        let _ = item.SetContent(&content_host.as_element());
        // `content_host` is a plain `Canvas`, and nothing else here ever gives it an explicit
        // `Width`/`Height` — unlike `Window.Content`, a `TabViewItem`'s own `ContentPresenter` does
        // not stretch its `Content` to fill it (same issue, and same fix, as
        // `InnerWindow::set_menu_bar`'s own doc comment describes for its menu-bar-wrapping outer
        // `Canvas`). Without this, `content_host.ActualWidth`/`ActualHeight` stay `0` forever and
        // this tab's whole widget tree never becomes visible, even though native property updates
        // (e.g. `TextArea::set_text`) keep reaching the controls inside it correctly.
        //
        // Sizing from *this* `TabViewItem`'s own `SizeChanged` (tried first) does not work: an
        // unselected tab's `ActualWidth`/`ActualHeight` apparently never gets assigned at all while
        // it isn't the visible one, so its `SizeChanged` may simply never fire — including for the
        // very first tab, before any selection change has ever happened. `TabView` itself, though,
        // reliably resizes with the window (confirmed — its own tab strip renders correctly), so
        // `content_hosts` tracks every tab's own `TreeHostPanel` and `new`'s `TabView.SizeChanged`
        // handler resizes *and force-relays-out* all of them together (see
        // `TreeHostPanel::force_relayout`'s doc comment), whether or not each one is currently
        // selected.
        self.content_hosts.borrow_mut().push(content_host.clone());
        let width = self.xaml.ActualWidth().unwrap_or(0.0);
        let height = (self.xaml.ActualHeight().unwrap_or(0.0) - TAB_VIEW_CONTENT_TOP_INSET)
            .max(0.0);
        let _ = content_host.as_element().SetWidth(width);
        let _ = content_host.as_element().SetHeight(height);
        content_host.force_relayout();
        if let Ok(items) = self.xaml.TabItems() {
            let item: windows::core::IInspectable = item.into();
            let _ = items.InsertAt(index as u32, &item);
        }
        content_host
    }

    pub(crate) fn remove_tab_at(&self, index: usize) {
        if let Ok(items) = self.xaml.TabItems() {
            let _ = items.RemoveAt(index as u32);
        }
        // Not removed from `content_hosts` — a closed tab's now-detached `Canvas` just keeps
        // getting harmlessly resized in the background along with the rest; not worth the
        // bookkeeping to prune it given `insert_tab`'s `index` and this list's (unordered, append-
        // only) positions don't otherwise need to correspond.
    }

    pub(crate) fn set_tab_title(&self, index: usize, title: &str) {
        if let Ok(items) = self.xaml.TabItems() {
            if let Ok(item) = items.GetAt(index as u32) {
                if let Ok(item) = item.cast::<TabViewItem>() {
                    if let Ok(value) = PropertyValue::CreateString(&HSTRING::from(title)) {
                        let _ = item.SetHeader(&value);
                    }
                }
            }
        }
    }

    pub(crate) fn set_selected_index(&self, index: usize) {
        let _ = self.xaml.SetSelectedIndex(index as i32);
    }
}
