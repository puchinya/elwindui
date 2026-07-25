//! `MenuBar`/`MenuFlyout` for the app menu bar and context menus.

use crate::ffi::{register_ui_event_callback, invoke_ui_event_callback, register_ui_index_event_callback, invoke_ui_index_event_callback, register_ui_key_event_callback, invoke_ui_key_event_callback, register_ui_text_event_callback, invoke_ui_text_event_callback};
use crate::bindings;
use crate::render::composition::{
    CompositionClipSpec, CompositionPrimitive, CompositionRenderer, DesiredCompositionIsland,
    DesiredCompositionNode, IslandId,
};
use crate::bindings::Microsoft::UI::Input::InputKeyboardSource;
use crate::bindings::Microsoft::UI::Xaml::Controls::{
    Button as XamlButton, Canvas, MenuFlyoutItem, MenuFlyoutItemBase, PasswordBox as XamlPasswordBox,
    ScrollMode, ScrollViewer, TabView as XamlTabView, TabViewCloseButtonOverlayMode, TabViewItem,
    TabViewTabCloseRequestedEventArgs, TextBlock, TextBox as XamlTextBox,
};
use crate::bindings::Microsoft::UI::Xaml::Input::{
    CharacterReceivedRoutedEventArgs, KeyEventHandler, KeyboardAccelerator,
};
use crate::bindings::Microsoft::UI::Xaml::Media::SolidColorBrush;
use crate::bindings::Microsoft::UI::Xaml::{
    FrameworkElement, RoutedEventHandler, SizeChangedEventHandler, UIElement, Window as XamlWindow,
};
use crate::bindings::Microsoft::Graphics::Canvas::UI::Composition::CanvasComposition;
use crate::bindings::Microsoft::Graphics::Canvas::{
    CanvasActiveLayer, CanvasAntialiasing, CanvasBitmap, CanvasBlend, CanvasEdgeBehavior, CanvasImageInterpolation,
    ICanvasResourceCreator,
};
use crate::bindings::Microsoft::UI::Composition::CompositionDrawingSurface;
use crate::bindings::Microsoft::Graphics::Canvas::Brushes::{
    CanvasGradientStop, CanvasImageBrush, CanvasLinearGradientBrush, CanvasRadialGradientBrush,
    CanvasSolidColorBrush, ICanvasBrush,
};
use crate::bindings::Microsoft::Graphics::Canvas::Geometry::{
    CanvasArcSize, CanvasFigureLoop, CanvasFilledRegionDetermination, CanvasGeometry,
    CanvasPathBuilder, CanvasSweepDirection,
};
use crate::bindings::Microsoft::UI::Xaml::Controls::{
    SelectionChangedEventHandler, TextChangedEventHandler,
};
use windows::Foundation::{PropertyValue, Size, TypedEventHandler};
use windows::Graphics::{PointInt32, SizeInt32};
use windows::System::{VirtualKey, VirtualKeyModifiers};
use windows::UI::{Color, Core::CoreVirtualKeyStates};
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream, IRandomAccessStream};
use elwindui_core::input::{
    FocusState, Key, KeyModifiers, KeyboardDispatcher, RawKeyEvent, RawKeyEventKind,
    RawTextInputEvent, ShortcutRegistry,
};
use elwindui_core::ui::{FocusHost, UIElementExt as _};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicUsize, Ordering};
use windows::core::{HSTRING, Interface, Result};

/// See `elwindui_backend_appkit::inner::InnerMenuItem`'s doc comment — same role, backed by a
/// `MenuFlyoutItem` (WinUI3's `MenuBarItem.Items` collection holds `MenuFlyoutItemBase`s).
/// Composed by `native_ui::MenuItem`.
#[derive(Clone)]
pub(crate) struct InnerMenuItem {
    xaml: MenuFlyoutItem,
    on_select: Rc<RefCell<Option<Box<dyn Fn()>>>>,
}

impl InnerMenuItem {
    pub(crate) fn new() -> Self {
        let xaml = MenuFlyoutItem::new().expect("MenuFlyoutItem::new");
        let this = Self {
            xaml,
            on_select: Rc::new(RefCell::new(None)),
        };
        {
        let callback = this.on_select.clone();
        let callback_id = register_ui_event_callback(Rc::new(move || {
            if let Some(callback) = callback.borrow().as_ref() { callback(); }
        }));
        let _ = this.xaml.Click(&RoutedEventHandler::new(move |_, _| {
            invoke_ui_event_callback(callback_id);
            Ok(())
        }));
        }
        this
    }

    /// A real title setter — construction takes no title argument, so this is the only way a menu
    /// item's title is ever actually set.
    pub(crate) fn set_text(&self, text: &str) {
        let _ = self.xaml.SetText(&HSTRING::from(text));
    }

    pub(crate) fn set_enabled(&self, enabled: bool) {
        let _ = self.xaml.SetIsEnabled(enabled);
    }

    /// A bare key character (e.g. `"s"`), matching AppKit's `set_shortcut` convention — mapped to
    /// a `Ctrl`-modifier `KeyboardAccelerator` (WinUI3 has no single-string key-equivalent setter
    /// the way `NSMenuItem.keyEquivalent` does).
    pub(crate) fn set_shortcut(&self, key_equivalent: &str) {
        let Some(key) = key_equivalent.chars().next() else {
            return;
        };
        let Ok(accelerator) = KeyboardAccelerator::new() else {
            return;
        };
        let _ = accelerator.SetModifiers(VirtualKeyModifiers::Control);
        let virtual_key = VirtualKey(key.to_ascii_uppercase() as i32);
        let _ = accelerator.SetKey(virtual_key);
        if let Ok(accelerators) = self.xaml.KeyboardAccelerators() {
            let _ = accelerators.Append(&accelerator);
        }
    }

    pub(crate) fn set_on_select(&self, callback: Box<dyn Fn()>) {
        *self.on_select.borrow_mut() = Some(callback);
    }
}

/// A dropdown attached to a `MenuBarItem` — see `elwindui_backend_appkit::inner::InnerMenu`'s doc
/// comment. `items` is a plain `Vec` (not the native `MenuFlyoutItemBase` collection directly)
/// since a `Menu` only ever becomes real XAML elements once installed into a `MenuBarItem`
/// (`InnerMenuBarItem::set_submenu`) — `add_item`/`remove_item` mutate this `Vec` and, if already
/// installed, the live XAML collection too. Composed by `native_ui::Menu`.
///
/// `installed_into` (deferred-install tracking) has no AppKit counterpart — `NSMenu` needs no such
/// bookkeeping — so this type's shape is a genuine, backend-specific divergence from
/// `elwindui_backend_appkit::inner::InnerMenu`, not an oversight.
#[derive(Clone)]
pub(crate) struct InnerMenu {
    items: Rc<RefCell<Vec<InnerMenuItem>>>,
    installed_into: Rc<
        RefCell<Option<windows_collections::IVector<MenuFlyoutItemBase>>>,
    >,
}

impl InnerMenu {
    pub(crate) fn new() -> Self {
        Self {
            items: Rc::new(RefCell::new(Vec::new())),
            installed_into: Rc::new(RefCell::new(None)),
        }
    }

    /// A real `IVector<MenuFlyoutItemBase>.Append`-style call once this `Menu` is installed into a
    /// `MenuBarItem` (see `installed_into`'s doc comment), reachable post-construction so
    /// `native_ui::Menu::set_children` can reconcile a changed child list without rebuilding the
    /// native menu from scratch.
    pub(crate) fn add_item(&self, item: &InnerMenuItem) {
        self.items.borrow_mut().push(item.clone());
        if let Some(items) = self.installed_into.borrow().as_ref() {
            if let Ok(base) = item.xaml.cast::<MenuFlyoutItemBase>() {
                let _ = items.Append(&base);
            }
        }
    }
    pub(crate) fn remove_item(&self, item: &InnerMenuItem) {
        let mut items = self.items.borrow_mut();
        if let Some(pos) = items.iter().position(|i| i.xaml == item.xaml) {
            items.remove(pos);
        }
        if let Some(native_items) = self.installed_into.borrow().as_ref() {
            let Ok(base) = item.xaml.cast::<MenuFlyoutItemBase>() else {
                return;
            };
            let mut index = 0;
            if native_items.IndexOf(&base, &mut index) == Ok(true) {
                let _ = native_items.RemoveAt(index);
            }
        }
    }
}

/// One top-level entry in the menu bar (e.g. "File"), holding its dropdown `InnerMenu` — composed
/// by `native_ui::MenuBarItem`.
#[derive(Clone)]
pub(crate) struct InnerMenuBarItem {
    xaml: bindings::Microsoft::UI::Xaml::Controls::MenuBarItem,
}

impl InnerMenuBarItem {
    pub(crate) fn new() -> Self {
        let xaml =
            bindings::Microsoft::UI::Xaml::Controls::MenuBarItem::new().expect("MenuBarItem::new");
        Self { xaml }
    }

    pub(crate) fn set_text(&self, text: &str) {
        let _ = self.xaml.SetTitle(&HSTRING::from(text));
    }
    pub(crate) fn set_submenu(&self, submenu: &InnerMenu) {
        if let Ok(items) = self.xaml.Items() {
            for item in submenu.items.borrow().iter() {
                if let Ok(base) = item.xaml.cast::<MenuFlyoutItemBase>() {
                    let _ = items.Append(&base);
                }
            }
            *submenu.installed_into.borrow_mut() = Some(items);
        }
    }
}

/// The whole top menu bar, installed via `native_ui::Window::set_menu_bar` — composed by
/// `native_ui::MenuBar`. Unlike AppKit (one global `NSApplication.mainMenu`), WinUI3's `MenuBar`
/// is a per-window element — installed by `InnerWindow::set_menu_bar` above, not a shared
/// process-wide singleton, so (unlike the AppKit backend) there's no app-menu-slot/Quit-item
/// special-casing needed here.
#[derive(Clone)]
pub(crate) struct InnerMenuBar {
    xaml: bindings::Microsoft::UI::Xaml::Controls::MenuBar,
}

impl InnerMenuBar {
    pub(crate) fn new() -> Self {
        let xaml = bindings::Microsoft::UI::Xaml::Controls::MenuBar::new().expect("MenuBar::new");
        Self { xaml }
    }

    pub(crate) fn add_item(&self, item: &InnerMenuBarItem) {
        if let Ok(items) = self.xaml.Items() {
            let _ = items.Append(&item.xaml);
        }
    }
    pub(crate) fn remove_item(&self, item: &InnerMenuBarItem) {
        if let Ok(items) = self.xaml.Items() {
            let mut index = 0;
            if items.IndexOf(&item.xaml, &mut index) == Ok(true) {
                let _ = items.RemoveAt(index);
            }
        }
    }
}
