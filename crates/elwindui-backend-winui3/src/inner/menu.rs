//! `MenuBar`/`MenuFlyout` for the app menu bar and context menus.

use crate::bindings;
use crate::bindings::Microsoft::UI::Xaml::Controls::{
    IconElement, ImageIcon, MenuFlyout, MenuFlyoutItem, MenuFlyoutItemBase, Symbol, SymbolIcon,
};
use crate::bindings::Microsoft::UI::Xaml::Input::KeyboardAccelerator;
use crate::bindings::Microsoft::UI::Xaml::Media::ImageSource as XamlImageSource;
use crate::bindings::Microsoft::UI::Xaml::Media::Imaging::BitmapImage as XamlBitmapImage;
use crate::bindings::Microsoft::UI::Xaml::RoutedEventHandler;
use crate::ffi::{invoke_ui_event_callback, register_ui_event_callback};
use elwindui_core::graphics::{IconSource, ImageData, ImageSource, SystemIcon};
use std::cell::RefCell;
use std::rc::Rc;
use windows::Storage::Streams::{DataWriter, IRandomAccessStream, InMemoryRandomAccessStream};
use windows::System::{VirtualKey, VirtualKeyModifiers};
use windows::core::{HSTRING, Interface};

/// See `elwindui_backend_appkit::inner::InnerMenuItem`'s doc comment — same role, backed by a
/// `MenuFlyoutItem` (WinUI3's `MenuBarItem.Items` collection holds `MenuFlyoutItemBase`s).
/// Composed by `native_ui::MenuItem`.
#[derive(Clone)]
pub(crate) struct InnerMenuItem {
    xaml: MenuFlyoutItem,
    shortcut: Rc<RefCell<Option<String>>>,
    on_select: Rc<RefCell<Option<Rc<dyn Fn()>>>>,
    /// Semantic `MenuItem.icon` state — shared across `Clone`s (see AppKit's `InnerMenuItem::icon`
    /// doc comment for why) and read by `InnerMenu::create_flyout` to snapshot the icon onto each
    /// newly-realized Context Menu `MenuFlyoutItem` (§2.13 of `icon_source_design.md`).
    icon: Rc<RefCell<Option<IconSource>>>,
}

impl InnerMenuItem {
    pub(crate) fn new() -> Self {
        let xaml = MenuFlyoutItem::new().expect("MenuFlyoutItem::new");
        let this = Self {
            xaml,
            shortcut: Rc::new(RefCell::new(None)),
            on_select: Rc::new(RefCell::new(None)),
            icon: Rc::new(RefCell::new(None)),
        };
        {
            let callback = this.on_select.clone();
            let callback_id = register_ui_event_callback(Rc::new(move || {
                let cb = callback.borrow().clone();
                if let Some(cb) = cb {
                    cb();
                }
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

    pub(crate) fn enabled(&self) -> bool {
        self.xaml.IsEnabled().unwrap_or(true)
    }

    /// A bare key character (e.g. `"s"`), matching AppKit's `set_shortcut` convention — mapped to
    /// a `Ctrl`-modifier `KeyboardAccelerator` (WinUI3 has no single-string key-equivalent setter
    /// the way `NSMenuItem.keyEquivalent` does).
    pub(crate) fn set_shortcut(&self, key_equivalent: &str) {
        *self.shortcut.borrow_mut() = if key_equivalent.is_empty() {
            None
        } else {
            Some(key_equivalent.to_string())
        };
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

    pub(crate) fn shortcut(&self) -> Option<String> {
        self.shortcut.borrow().clone()
    }

    pub(crate) fn text(&self) -> String {
        self.xaml.Text().map(|h| h.to_string()).unwrap_or_default()
    }

    pub(crate) fn select(&self) {
        let cb = self.on_select.borrow().clone();
        if let Some(callback) = cb {
            callback();
        }
    }

    pub(crate) fn set_on_select(&self, callback: Box<dyn Fn()>) {
        *self.on_select.borrow_mut() = Some(Rc::from(callback));
    }

    pub(crate) fn icon(&self) -> Option<IconSource> {
        self.icon.borrow().clone()
    }

    /// Ordering per `icon_source_design.md` §7: semantic state first, then the live native
    /// reflection — a failed conversion (§2.11) only omits `MenuFlyoutItem.Icon`, it never rolls
    /// back the semantic state or touches text/enabled/shortcut/on_select.
    pub(crate) fn set_icon(&self, icon: Option<IconSource>) {
        *self.icon.borrow_mut() = icon.clone();
        let icon_element = icon.and_then(|icon| icon_source_to_icon_element(&icon));
        let _ = self.xaml.SetIcon(icon_element.as_ref());
    }
}

/// Exact mapping fixed by `docs/design/runtime/icon_source_design.md` §2 — every `SystemIcon`
/// variant maps to exactly one WinUI `Symbol`, no wildcard/typo fallback. The `_` arm exists only
/// because `SystemIcon` is `#[non_exhaustive]` (required by the compiler for a match in a
/// downstream crate); it can never fire for any variant that exists today — see
/// `elwindui_backend_appkit::inner::menu::sf_symbol_name`'s identical reasoning.
fn winui_symbol(icon: SystemIcon) -> Symbol {
    match icon {
        SystemIcon::Add => Symbol::Add,
        SystemIcon::Remove => Symbol::Remove,
        SystemIcon::Delete => Symbol::Delete,
        SystemIcon::Edit => Symbol::Edit,
        SystemIcon::Copy => Symbol::Copy,
        SystemIcon::Cut => Symbol::Cut,
        SystemIcon::Paste => Symbol::Paste,
        SystemIcon::Undo => Symbol::Undo,
        SystemIcon::Redo => Symbol::Redo,
        SystemIcon::Search => Symbol::Find,
        SystemIcon::Settings => Symbol::Setting,
        SystemIcon::Refresh => Symbol::Refresh,
        _ => unreachable!("SystemIcon variant not mapped to a WinUI Symbol in winui_symbol"),
    }
}

fn icon_source_to_icon_element(icon: &IconSource) -> Option<IconElement> {
    match icon {
        IconSource::System(system_icon) => system_icon_element(*system_icon),
        IconSource::Image(source) => user_image_icon_element(source),
    }
}

/// A lookup/construction failure (out-of-process activation error, symbol unavailable) simply
/// omits the icon (§2.11) rather than panicking.
fn system_icon_element(icon: SystemIcon) -> Option<IconElement> {
    let symbol_icon = SymbolIcon::new().ok()?;
    symbol_icon.SetSymbol(winui_symbol(icon)).ok()?;
    symbol_icon.cast::<IconElement>().ok()
}

/// Reuses `render/composition/cache.rs`'s exact `InMemoryRandomAccessStream`/`DataWriter`
/// byte-stream pattern (rather than a new decoder, §3.7) to feed `BitmapImage.SetSource`, then
/// wraps the result in an `ImageIcon`.
///
/// Two cases return `None` (icon omitted, §2.11 — never a panic, never touches the rest of the
/// item) rather than being handled, and are recorded as a known, disclosed gap in
/// `docs/design/runtime/icon_source_design.md` §5/§9 pending Windows-side follow-up, since neither
/// can be exercised or verified without a Windows build environment (this whole crate is
/// `#![cfg(target_os = "windows")]` and has never been built/type-checked on this machine):
///
/// - `ImageSource::Vector` — no existing WinUI3 code path rasterizes a `VectorImage` to a
///   XAML-compatible image source; doing so would need a `CanvasRenderTarget`-based Win2D
///   rasterize-then-encode round trip this crate does not have today.
/// - `ImageData::Rgba8`/`ImageData::Backend` — same limitation this backend's existing composition
///   image path already has (see `ImageSurfaceCache::surface_for`'s own "RGBA and backend image
///   handles require ... fallback" comment): there is no encoded container to hand
///   `BitmapImage.SetSource` for raw pixel data.
fn user_image_icon_element(source: &ImageSource) -> Option<IconElement> {
    let ImageSource::Raster(bitmap) = source else {
        return None;
    };
    let ImageData::Encoded { bytes, .. } = bitmap.data() else {
        return None;
    };
    let stream = InMemoryRandomAccessStream::new().ok()?;
    let writer = DataWriter::CreateDataWriter(&stream).ok()?;
    writer.WriteBytes(bytes).ok()?;
    writer.StoreAsync().ok()?.join().ok()?;
    let stream: IRandomAccessStream = stream.cast().ok()?;
    stream.Seek(0).ok()?;
    let bitmap_image = XamlBitmapImage::new().ok()?;
    bitmap_image.SetSource(&stream).ok()?;
    let image_source: XamlImageSource = bitmap_image.cast().ok()?;
    let image_icon = ImageIcon::new().ok()?;
    image_icon.SetSource(&image_source).ok()?;
    image_icon.cast::<IconElement>().ok()
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
    installed_into: Rc<RefCell<Option<windows_collections::IVector<MenuFlyoutItemBase>>>>,
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

    pub(crate) fn create_flyout(&self) -> Result<MenuFlyout, windows::core::Error> {
        let flyout = MenuFlyout::new()?;
        let items = flyout.Items()?;
        for item in self.items.borrow().iter() {
            let flyout_item = MenuFlyoutItem::new()?;
            flyout_item.SetText(&windows::core::HSTRING::from(item.text().as_str()))?;
            flyout_item.SetIsEnabled(item.enabled())?;
            if let Some(shortcut) = item.shortcut() {
                let _ = flyout_item.SetKeyboardAcceleratorTextOverride(
                    &windows::core::HSTRING::from(shortcut.as_str()),
                );
            }
            // §2.13/§8.14: this Context Menu realization is a brand-new `MenuFlyoutItem` distinct
            // from `item`'s own live `xaml` (PR #156's realization-ownership separation) — so the
            // current icon must be snapshotted onto it explicitly here, the same as
            // text/enabled/shortcut above. Missing this step is exactly the regression §8.14 tests
            // for.
            if let Some(icon) = item.icon() {
                if let Some(icon_element) = icon_source_to_icon_element(&icon) {
                    let _ = flyout_item.SetIcon(&icon_element);
                }
            }
            let item_clone = item.clone();
            let _ = flyout_item.Click(&RoutedEventHandler::new(move |_, _| {
                item_clone.select();
                Ok(())
            }));
            let base: MenuFlyoutItemBase = flyout_item.cast()?;
            let _ = items.Append(&base);
        }
        Ok(flyout)
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
    pub(crate) xaml: bindings::Microsoft::UI::Xaml::Controls::MenuBar,
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

#[cfg(test)]
mod icon_tests {
    use super::*;

    /// §8.12: every `SystemIcon` variant maps to exactly one WinUI `Symbol`, matching
    /// `docs/design/runtime/icon_source_design.md` §2's table exactly, no variant omitted. Pure
    /// value comparison only — no `MenuFlyoutItem`/`SymbolIcon` construction, so no UI-thread
    /// requirement (mirrors `elwindui_backend_appkit::inner::menu::icon_tests`'s equivalent).
    #[test]
    fn every_system_icon_variant_maps_to_its_documented_symbol() {
        let expected: &[(SystemIcon, Symbol)] = &[
            (SystemIcon::Add, Symbol::Add),
            (SystemIcon::Remove, Symbol::Remove),
            (SystemIcon::Delete, Symbol::Delete),
            (SystemIcon::Edit, Symbol::Edit),
            (SystemIcon::Copy, Symbol::Copy),
            (SystemIcon::Cut, Symbol::Cut),
            (SystemIcon::Paste, Symbol::Paste),
            (SystemIcon::Undo, Symbol::Undo),
            (SystemIcon::Redo, Symbol::Redo),
            (SystemIcon::Search, Symbol::Find),
            (SystemIcon::Settings, Symbol::Setting),
            (SystemIcon::Refresh, Symbol::Refresh),
        ];
        assert_eq!(expected.len(), 12, "must cover every SystemIcon variant");
        for (icon, symbol) in expected {
            assert_eq!(
                winui_symbol(*icon),
                *symbol,
                "{icon:?} must map to Symbol {symbol:?}"
            );
        }
    }
}
