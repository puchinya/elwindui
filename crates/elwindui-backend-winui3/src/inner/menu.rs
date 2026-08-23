//! `MenuBar`/`MenuFlyout` for the app menu bar and context menus.

use crate::bindings;
use crate::bindings::Microsoft::Graphics::Canvas::ICanvasResourceCreator;
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

/// Fixed native menu icon raster size for a rasterized `VectorImage` (§2.10 of
/// `icon_source_design.md`: 16 DIP base size; rasterized at 2x for Retina/high-DPI crispness,
/// matching AppKit's `MENU_ICON_PIXEL_SIZE`).
const MENU_ICON_RASTER_SIZE: f32 = 32.0;

/// PR #171 delta remediation: completes user `ImageSource` menu icon support for every case —
/// `ImageData::Encoded` keeps the direct fast path (no re-encode); `ImageData::Rgba8` and a
/// Win2D-`CanvasBitmap`-backed `ImageData::Backend` reuse the crate's existing
/// `render::win2d_bitmap` conversion (never re-implemented here, §3.1/§13.1 of the delta
/// contract); `ImageSource::Vector` reuses the crate's existing vector rendering pipeline via
/// `render::rasterize_vector_image_to_canvas_bitmap` (§3.3/§13.2). A `CanvasBitmap` produced by
/// either of the latter two paths is bridged to a XAML image source via
/// `canvas_bitmap_to_xaml_image_source`, then wrapped in an `ImageIcon` by
/// `xaml_image_source_to_icon_element` — the same terminal step the encoded fast path also uses,
/// so every user-image case ends in identical `ImageIcon`/`IconElement` construction.
///
/// A conversion failure at any step (incompatible backend handle, offscreen rasterization error,
/// XAML source creation error) returns `None` — icon omitted, `MenuItem` otherwise unaffected
/// (§2.11) — never a panic, never a state rollback (`set_icon` above keeps ordering: semantic
/// state first, native reflection second).
fn user_image_icon_element(source: &ImageSource) -> Option<IconElement> {
    match source {
        ImageSource::Raster(bitmap) => match bitmap.data() {
            ImageData::Encoded { .. } => encoded_raster_icon_element(bitmap),
            ImageData::Rgba8 { .. } | ImageData::Backend(..) => win2d_raster_icon_element(bitmap),
        },
        ImageSource::Vector(vector) => vector_icon_element(vector),
    }
}

/// The direct `ImageData::Encoded` fast path: no decode/re-encode round trip, same
/// `InMemoryRandomAccessStream`/`DataWriter` byte-stream pattern
/// `render/composition/cache.rs`'s `ImageSurfaceCache::surface_for` already uses (§3.7 — no new
/// decoder).
fn encoded_raster_icon_element(
    bitmap: &elwindui_core::graphics::BitmapImage,
) -> Option<IconElement> {
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
    xaml_image_source_to_icon_element(image_source)
}

/// `ImageData::Rgba8` and a Win2D-`CanvasBitmap`-backed `ImageData::Backend` — reuses
/// `render::win2d_bitmap` (the crate's one, already-existing raw-pixel/backend-handle conversion;
/// never duplicated here, §13.1) to obtain a `CanvasBitmap`, then bridges it to XAML the same way
/// the vector path does. An incompatible `ImageData::Backend` handle (resolved via
/// `win2d_bitmap`'s own `downcast_ref::<CanvasBitmap>()` — see that function's `Err` arm) fails
/// this call with `None`: icon omitted, no panic (§2.11, §8.1 of this delta contract — the
/// approved resolution of the original contract's `ImageData::Backend` ambiguity).
fn win2d_raster_icon_element(bitmap: &elwindui_core::graphics::BitmapImage) -> Option<IconElement> {
    let device =
        crate::bindings::Microsoft::Graphics::Canvas::CanvasDevice::GetSharedDevice().ok()?;
    let creator: ICanvasResourceCreator = device.cast().ok()?;
    let canvas_bitmap = crate::render::win2d_bitmap(&creator, bitmap).ok()?;
    let image_source = canvas_bitmap_to_xaml_image_source(&canvas_bitmap)?;
    xaml_image_source_to_icon_element(image_source)
}

/// `ImageSource::Vector` — reuses `render::rasterize_vector_image_to_canvas_bitmap`, which itself
/// reuses the crate's existing `emit_vector_image`/`replay_win2d_primitives` vector-drawing
/// pipeline (no second VectorScene traversal, §3.3/§13.2). Rasterizes into a fixed
/// `MENU_ICON_RASTER_SIZE`x`MENU_ICON_RASTER_SIZE` transparent offscreen target, matching the
/// system-icon and encoded-raster paths' fixed native menu icon sizing (§2.10).
fn vector_icon_element(vector: &elwindui_core::graphics::VectorImage) -> Option<IconElement> {
    let device =
        crate::bindings::Microsoft::Graphics::Canvas::CanvasDevice::GetSharedDevice().ok()?;
    let creator: ICanvasResourceCreator = device.cast().ok()?;
    let render_target = crate::render::rasterize_vector_image_to_canvas_bitmap(
        &creator,
        vector,
        MENU_ICON_RASTER_SIZE,
        MENU_ICON_RASTER_SIZE,
    )
    .ok()?;
    let canvas_bitmap: crate::bindings::Microsoft::Graphics::Canvas::CanvasBitmap =
        render_target.cast().ok()?;
    let image_source = canvas_bitmap_to_xaml_image_source(&canvas_bitmap)?;
    xaml_image_source_to_icon_element(image_source)
}

/// Encodes a `CanvasBitmap`/`CanvasRenderTarget` to an in-memory PNG stream and hands that stream
/// to a fresh XAML `BitmapImage`, matching this crate's established
/// `InMemoryRandomAccessStream`-based bridging convention (§4.5 of the delta contract) — no
/// temporary filesystem file, no retained stream cache (menu icons are set rarely, not once per
/// frame; the stream/temporary Canvas resources are released once the XAML source has consumed
/// them, same reasoning as AppKit's per-call `user_image_nsimage` cache).
///
/// Calls the stream-overload projection of Win2D's `ICanvasBitmap` save surface — the Win2D ABI
/// names this overload `SaveToStreamAsync(IRandomAccessStream, CanvasBitmapFileFormat)`,
/// distinct from the filename-based `SaveToFileAsync`/`SaveToFileWithBitmapFileFormatAsync` and
/// the quality-parameterized `SaveToStreamWithQualityAsync` overloads (second PR #171 review
/// delta §A1 — a plain `SaveAsync` name was an unjustified guess given this crate's own
/// `CanvasBitmap::LoadAsyncFromStream` already demonstrates `windows_bindgen` projects
/// ABI-specific overload names, not a collapsed friendly name). Windows build/execution itself
/// remains deferred to Issue #157; if the actual generated binding differs mechanically from
/// `SaveToStreamAsync`, correct the spelling there — not a reason to revert this call site to a
/// name already known to be wrong.
fn canvas_bitmap_to_xaml_image_source(
    bitmap: &crate::bindings::Microsoft::Graphics::Canvas::CanvasBitmap,
) -> Option<XamlImageSource> {
    use crate::bindings::Microsoft::Graphics::Canvas::CanvasBitmapFileFormat;

    let stream = InMemoryRandomAccessStream::new().ok()?;
    let random_access_stream: IRandomAccessStream = stream.cast().ok()?;
    bitmap
        .SaveToStreamAsync(&random_access_stream, CanvasBitmapFileFormat::Png)
        .ok()?
        .join()
        .ok()?;
    random_access_stream.Seek(0).ok()?;
    let bitmap_image = XamlBitmapImage::new().ok()?;
    bitmap_image.SetSource(&random_access_stream).ok()?;
    bitmap_image.cast::<XamlImageSource>().ok()
}

/// The terminal step every user-image path shares: wrap an already-produced XAML
/// `Microsoft.UI.Xaml.Media.ImageSource` in an `ImageIcon`, then cast up to the `IconElement`
/// `MenuFlyoutItem.Icon` expects — factored out so encoded/Rgba8/Backend/Vector all end in
/// identical construction (§6.4 of the delta contract).
fn xaml_image_source_to_icon_element(image_source: XamlImageSource) -> Option<IconElement> {
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

/// §9.7/§9.8/§9.9 of the PR #171 delta remediation contract. Unlike every existing test in this
/// crate (pure logic — see `render::win2d_bitmap_tests`'s own doc comment for the same point about
/// `render/win2d.rs`), these construct live `MenuFlyoutItem`/`SymbolIcon` XAML objects, which (per
/// this crate's established test conventions — no prior test in this crate does this) may need a
/// WinUI `DispatcherQueue`/`Application` context this test binary does not currently set up.
/// **Unverified**: written but not run — no Windows build/runtime environment available in this
/// remediation session. If XAML object construction fails inside `cargo test`'s default worker
/// threads, that is the concrete "WinUI UI-thread test harness" gap the delta contract's §9.7
/// anticipates; report it as a finding rather than assuming these pass.
#[cfg(test)]
mod live_menu_item_icon_tests {
    use super::*;

    /// §9.7: `InnerMenuItem::set_icon` set / replace / clear against the live `MenuFlyoutItem`,
    /// leaving `text`/`enabled`/`shortcut`/the select callback untouched.
    #[test]
    fn live_inner_menu_item_icon_set_replace_clear() {
        let item = InnerMenuItem::new();
        item.set_text("Copy");
        item.set_enabled(true);
        item.set_shortcut("c");
        let selected = std::rc::Rc::new(std::cell::Cell::new(false));
        let selected_clone = std::rc::Rc::clone(&selected);
        item.set_on_select(Box::new(move || selected_clone.set(true)));

        // 1. SystemIcon::Copy -> the live native Icon must actually be a SymbolIcon carrying
        //    Symbol::Copy, not merely "something non-null" (second PR #171 review delta §A2 — a
        //    broken implementation that updates semantic state but leaves a stale/wrong native
        //    icon installed would still pass an `is_some()`-only check).
        item.set_icon(Some(IconSource::System(SystemIcon::Copy)));
        let icon_after_copy = item
            .xaml
            .Icon()
            .ok()
            .flatten()
            .expect("MenuFlyoutItem.Icon must be non-null after set_icon(System(Copy))");
        let symbol_icon: SymbolIcon = icon_after_copy
            .cast()
            .expect("native icon after System(Copy) must be a SymbolIcon");
        assert_eq!(
            symbol_icon.Symbol().expect("SymbolIcon.Symbol"),
            Symbol::Copy,
            "native SymbolIcon must carry Symbol::Copy"
        );
        match item.icon() {
            Some(IconSource::System(SystemIcon::Copy)) => {}
            other => panic!("expected Some(System(Copy)), got {other:?}"),
        }

        // 2. Replace with a user Rgba8 raster icon -> the live native Icon must now be an
        //    ImageIcon (exercising the win2d_bitmap-backed menu path this remediation added, not
        //    just re-checking the SystemIcon path).
        let rgba8_bitmap = elwindui_core::graphics::BitmapImage::from_rgba8(
            2,
            2,
            8,
            vec![
                10u8, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 255, 100, 110, 120, 255,
            ],
            elwindui_core::graphics::AlphaMode::Opaque,
        )
        .expect("well-formed 2x2 rgba8 buffer");
        item.set_icon(Some(IconSource::Image(ImageSource::Raster(rgba8_bitmap))));
        let icon_after_rgba8 = item
            .xaml
            .Icon()
            .ok()
            .flatten()
            .expect("MenuFlyoutItem.Icon must be non-null after set_icon(Image(Rgba8))");
        let _image_icon: ImageIcon = icon_after_rgba8
            .cast()
            .expect("native icon after a user Rgba8 image must be an ImageIcon, not a SymbolIcon");

        // 3. Clear -> live native Icon must become null again.
        item.set_icon(None);
        assert!(item.icon().is_none());
        assert!(
            item.xaml.Icon().ok().flatten().is_none(),
            "MenuFlyoutItem.Icon must be null after set_icon(None)"
        );

        assert_eq!(item.text(), "Copy");
        assert!(item.enabled());
        assert_eq!(item.shortcut().as_deref(), Some("c"));
        item.select();
        assert!(selected.get(), "select() must still invoke the callback");
    }

    /// §9.8: mandatory PR #156 regression — `InnerMenu::create_flyout()` snapshots the current
    /// icon onto a *newly*-realized `MenuFlyoutItem`, distinct from the live MenuBar-side item's
    /// own `xaml`, and reflects the latest semantic value on each subsequent call.
    #[test]
    fn create_flyout_snapshots_icon_onto_a_distinct_realization() {
        let item = InnerMenuItem::new();
        item.set_text("Copy");
        item.set_icon(Some(IconSource::System(SystemIcon::Copy)));

        let menu = InnerMenu::new();
        menu.add_item(&item);

        let flyout = menu
            .create_flyout()
            .expect("create_flyout must succeed for a single-item menu");
        let items = flyout.Items().expect("MenuFlyout.Items");
        assert_eq!(items.Size().unwrap_or(0), 1);
        let realized: MenuFlyoutItem = items
            .GetAt(0)
            .expect("realized flyout item")
            .cast()
            .expect("realized item casts to MenuFlyoutItem");
        // Explicitly Symbol::Copy, not merely "has some icon" (second PR #171 review delta §A3 —
        // a regression that always copied the *first* icon onto every later realization would
        // still pass an `is_some()`-only check).
        let realized_icon = realized
            .Icon()
            .ok()
            .flatten()
            .expect("the newly-realized MenuFlyoutItem must have an icon");
        let realized_symbol: SymbolIcon = realized_icon
            .cast()
            .expect("realized icon must be a SymbolIcon");
        assert_eq!(
            realized_symbol.Symbol().expect("SymbolIcon.Symbol"),
            Symbol::Copy,
            "first realization must carry the current semantic icon, Symbol::Copy"
        );
        assert_ne!(
            realized, item.xaml,
            "create_flyout() must not reuse the live MenuBar-side MenuFlyoutItem instance (PR #156)"
        );

        // Latest semantic value flows into the next realization: change to Delete before the
        // second create_flyout() call.
        item.set_icon(Some(IconSource::System(SystemIcon::Delete)));
        let flyout2 = menu
            .create_flyout()
            .expect("second create_flyout must also succeed");
        let items2 = flyout2.Items().expect("MenuFlyout.Items");
        let realized2: MenuFlyoutItem = items2
            .GetAt(0)
            .expect("second realized flyout item")
            .cast()
            .expect("second realized item casts to MenuFlyoutItem");
        let realized2_icon = realized2
            .Icon()
            .ok()
            .flatten()
            .expect("second realization must have an icon");
        let realized2_symbol: SymbolIcon = realized2_icon
            .cast()
            .expect("second realized icon must be a SymbolIcon");
        assert_eq!(
            realized2_symbol.Symbol().expect("SymbolIcon.Symbol"),
            Symbol::Delete,
            "second realization must carry the *updated* semantic icon, Symbol::Delete — not the \
             first realization's stale Symbol::Copy"
        );
        assert_ne!(
            realized, realized2,
            "each create_flyout() call must produce its own new realization, not reuse the previous one"
        );
    }

    /// §9.9: a failed icon conversion (an unsupported user `ImageSource` case, before this
    /// remediation — kept here as a regression guard using the still-legitimately-fallible
    /// incompatible `ImageData::Backend` case) omits the icon but leaves the item fully
    /// functional: text preserved, still selectable, callback still fires exactly once.
    #[test]
    fn failed_icon_conversion_does_not_remove_the_action() {
        let item = InnerMenuItem::new();
        item.set_text("Broken Icon Action");
        let selected = std::rc::Rc::new(std::cell::Cell::new(0u32));
        let selected_clone = std::rc::Rc::clone(&selected);
        item.set_on_select(Box::new(move || {
            selected_clone.set(selected_clone.get() + 1)
        }));

        let incompatible = elwindui_core::graphics::BitmapImage::from_backend_handle(
            elwindui_core::graphics::BackendImageHandle(std::sync::Arc::new(42u32)),
        );
        item.set_icon(Some(IconSource::Image(ImageSource::Raster(incompatible))));

        assert!(
            item.xaml.Icon().ok().flatten().is_none(),
            "an incompatible backend handle must omit the icon, not synthesize one"
        );
        assert_eq!(item.text(), "Broken Icon Action");
        item.select();
        assert_eq!(
            selected.get(),
            1,
            "the callback must still fire exactly once"
        );
    }
}
