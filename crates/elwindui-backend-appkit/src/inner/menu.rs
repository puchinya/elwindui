//! `NSMenu`/`NSMenuItem` for both the app menu bar and context menus, plus the target/action
//! trampoline that turns a menu selection into a routed event.

use crate::ffi::mtm;
use elwindui_core::graphics::{IconSource, ImageSource, SystemIcon};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AnyThread, DefinedClass, define_class, msg_send, sel};
use objc2_app_kit::{NSImage, NSMenu, NSMenuItem};
use objc2_foundation::{NSObjectProtocol, NSSize, NSString};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// See docs/specs/ui_spec.md#menu. A single application-wide `NSMenu` (top menu bar
/// item / `File`, `Edit`, ...) entry — composed by `native_ui::MenuItem`.
#[derive(Clone)]
pub(crate) struct InnerMenuItem {
    ns: Retained<NSMenuItem>,
    target_storage: Rc<RefCell<Option<Retained<MenuItemTarget>>>>,
    /// Semantic `MenuItem.icon` state, kept alongside the native `NSMenuItem.image` it drives
    /// (§2.8 of `docs/design/runtime/icon_source_design.md`) — shared across `Clone`s of this
    /// `InnerMenuItem` so every handle observes the same latest value.
    icon: Rc<RefCell<Option<IconSource>>>,
}

impl InnerMenuItem {
    pub(crate) fn new() -> Self {
        let m = mtm();
        let ns = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                m.alloc::<NSMenuItem>(),
                &NSString::from_str(""),
                None,
                &NSString::from_str(""),
            )
        };
        Self {
            ns,
            target_storage: Rc::new(RefCell::new(None)),
            icon: Rc::new(RefCell::new(None)),
        }
    }

    /// A real `NSMenuItem.title` setter — construction takes no title argument, so this is the
    /// only way a menu item's title is ever actually set.
    pub(crate) fn set_text(&self, text: &str) {
        self.ns.setTitle(&NSString::from_str(text));
    }

    pub(crate) fn set_enabled(&self, enabled: bool) {
        self.ns.setEnabled(enabled);
    }

    /// A bare key character (e.g. `"s"`); macOS defaults a menu item's modifier mask to Cmd,
    /// which matches the common `Cmd+<letter>` shortcuts notepad needs.
    pub(crate) fn set_shortcut(&self, key_equivalent: &str) {
        self.ns
            .setKeyEquivalent(&NSString::from_str(key_equivalent));
    }

    pub(crate) fn text(&self) -> String {
        self.ns.title().to_string()
    }

    pub(crate) fn enabled(&self) -> bool {
        self.ns.isEnabled()
    }

    pub(crate) fn shortcut(&self) -> Option<String> {
        let eq = self.ns.keyEquivalent().to_string();
        if eq.is_empty() { None } else { Some(eq) }
    }

    pub(crate) fn select(&self) {
        let target = self.target_storage.borrow().clone();
        if let Some(target) = target {
            (target.ivars().callback)();
        }
    }

    pub(crate) fn set_on_select(&self, callback: Box<dyn Fn()>) {
        let target = MenuItemTarget::new(MenuItemTargetIvars { callback });
        unsafe {
            self.ns.setTarget(Some(&target));
            self.ns.setAction(Some(sel!(perform:)));
        }
        *self.target_storage.borrow_mut() = Some(target);
    }

    pub(crate) fn icon(&self) -> Option<IconSource> {
        self.icon.borrow().clone()
    }

    /// Ordering per `icon_source_design.md` §7: semantic state first, then the live native
    /// reflection — a failed conversion (§2.11) only omits `NSMenuItem.image`, it never rolls
    /// back the semantic state or touches title/enabled/shortcut/target.
    pub(crate) fn set_icon(&self, icon: Option<IconSource>) {
        *self.icon.borrow_mut() = icon.clone();
        let native_image = icon.and_then(|icon| icon_source_to_nsimage(&icon));
        self.ns.setImage(native_image.as_deref());
    }
}

/// 16pt is this Issue's fixed native menu icon size (§2.10 of `icon_source_design.md`); the
/// backing bitmap is rasterized at 2x that so the image stays crisp on Retina displays.
const MENU_ICON_POINT_SIZE: f64 = 16.0;
const MENU_ICON_PIXEL_SIZE: usize = 32;

fn icon_source_to_nsimage(icon: &IconSource) -> Option<Retained<NSImage>> {
    match icon {
        IconSource::System(system_icon) => system_icon_nsimage(*system_icon),
        IconSource::Image(source) => user_image_nsimage(source),
    }
}

/// Exact mapping fixed by `docs/design/runtime/icon_source_design.md` §2 — every `SystemIcon`
/// variant maps to exactly one SF Symbol name, no wildcard/typo fallback. The `_` arm exists only
/// because `SystemIcon` is `#[non_exhaustive]` (required by the compiler for a match in a
/// downstream crate); it can never fire for any variant that exists today; a variant added to
/// `SystemIcon` without a matching arm here is exactly the "mapping completeness" defect
/// §2.11/§8.9 call out, so it panics loudly instead of silently omitting the icon.
fn sf_symbol_name(icon: SystemIcon) -> &'static str {
    match icon {
        SystemIcon::Add => "plus",
        SystemIcon::Remove => "minus",
        SystemIcon::Delete => "trash",
        SystemIcon::Edit => "pencil",
        SystemIcon::Copy => "doc.on.doc",
        SystemIcon::Cut => "scissors",
        SystemIcon::Paste => "doc.on.clipboard",
        SystemIcon::Undo => "arrow.uturn.backward",
        SystemIcon::Redo => "arrow.uturn.forward",
        SystemIcon::Search => "magnifyingglass",
        SystemIcon::Settings => "gearshape",
        SystemIcon::Refresh => "arrow.clockwise",
        _ => unreachable!("SystemIcon variant not mapped to an SF Symbol name in sf_symbol_name"),
    }
}

/// A lookup failure (old OS, or a symbol name the running OS doesn't recognize) simply omits the
/// icon (§2.11) rather than panicking — `imageWithSystemSymbolName:accessibilityDescription:`
/// itself already returns `None` in that case, so no separate `respondsToSelector:` probe is
/// needed here. (`InnerButton::set_system_symbol_or_text`, `inner/button.rs`, gates the same call
/// behind `NSImage::class().responds_to(..)` first — verified against a real window during this
/// Issue's AppKit runtime check that this particular `responds_to` probe reports `false` on this
/// machine's OS/objc2 version even though the call it's guarding succeeds immediately afterward;
/// tracked as a pre-existing, out-of-scope finding rather than fixed here, since `button.rs` is
/// unrelated to Menu icons.)
fn system_icon_nsimage(icon: SystemIcon) -> Option<Retained<NSImage>> {
    NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str(sf_symbol_name(icon)),
        None,
    )
}

/// Reuses the crate's existing raster/vector decode paths (`render::resolve_cgimage`,
/// `render::rasterize_vector_image_to_cgimage`) rather than a new decoder (§3.7) — a decode/
/// rasterize failure returns `None` here, which `set_icon` above turns into "no icon, item
/// otherwise unaffected" per §2.11. The cache each call builds is intentionally scoped to that one
/// call: menu icons are set rarely (not once per frame), so there is nothing worth keeping an
/// unbounded process-lifetime cache alive for (§6.11).
fn user_image_nsimage(source: &ImageSource) -> Option<Retained<NSImage>> {
    let cg_image = match source {
        ImageSource::Raster(bitmap) => {
            let mut cache = HashMap::new();
            crate::render::resolve_cgimage(bitmap, &mut cache)?
        }
        ImageSource::Vector(vector) => {
            let mut cache = HashMap::new();
            crate::render::rasterize_vector_image_to_cgimage(
                vector,
                vector.view_box(),
                MENU_ICON_PIXEL_SIZE,
                MENU_ICON_PIXEL_SIZE,
                &mut cache,
            )?
        }
    };
    Some(NSImage::initWithCGImage_size(
        NSImage::alloc(),
        &cg_image,
        NSSize::new(MENU_ICON_POINT_SIZE, MENU_ICON_POINT_SIZE),
    ))
}

struct MenuItemTargetIvars {
    callback: Box<dyn Fn()>,
}

define_class!(
    #[unsafe(super(objc2_foundation::NSObject))]
    #[ivars = MenuItemTargetIvars]
    struct MenuItemTarget;

    unsafe impl NSObjectProtocol for MenuItemTarget {}

    impl MenuItemTarget {
        #[unsafe(method(perform:))]
        fn perform(&self, _sender: &AnyObject) {
            (self.ivars().callback)();
        }
    }
);

impl MenuItemTarget {
    fn new(ivars: MenuItemTargetIvars) -> Retained<Self> {
        let this = Self::alloc().set_ivars(ivars);
        unsafe { msg_send![super(this), init] }
    }
}

/// A dropdown attached to a `MenuBarItem` (or, per 付録M, a right-click context menu — not used
/// that way here, but the same type covers both) — composed by `native_ui::Menu`.
#[derive(Clone)]
pub(crate) struct InnerMenu {
    ns: Retained<NSMenu>,
}

impl InnerMenu {
    pub(crate) fn new() -> Self {
        let m = mtm();
        let ns = NSMenu::initWithTitle(m.alloc::<NSMenu>(), &NSString::from_str(""));
        Self { ns }
    }

    pub(crate) fn add_item(&self, item: &InnerMenuItem) {
        self.ns.addItem(&item.ns);
    }
    pub(crate) fn remove_item(&self, item: &InnerMenuItem) {
        self.ns.removeItem(&item.ns);
    }
    pub(crate) fn ns(&self) -> Retained<NSMenu> {
        self.ns.clone()
    }
}

/// One top-level entry in the menu bar (e.g. "File"), holding its dropdown `InnerMenu` — composed
/// by `native_ui::MenuBarItem`.
#[derive(Clone)]
pub(crate) struct InnerMenuBarItem {
    ns: Retained<NSMenuItem>,
}

impl InnerMenuBarItem {
    pub(crate) fn new() -> Self {
        let m = mtm();
        let ns = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                m.alloc::<NSMenuItem>(),
                &NSString::from_str(""),
                None,
                &NSString::from_str(""),
            )
        };
        Self { ns }
    }

    pub(crate) fn set_text(&self, text: &str) {
        self.ns.setTitle(&NSString::from_str(text));
    }
    pub(crate) fn set_submenu(&self, submenu: &InnerMenu) {
        self.ns.setSubmenu(Some(&submenu.ns));
    }
}

/// The whole top menu bar, installed via `native_ui::Window::set_menu_bar` — composed by
/// `native_ui::MenuBar`.
#[derive(Clone)]
pub(crate) struct InnerMenuBar {
    pub(crate) ns: Retained<NSMenu>,
}

impl InnerMenuBar {
    pub(crate) fn new() -> Self {
        let m = mtm();
        let ns = NSMenu::initWithTitle(m.alloc::<NSMenu>(), &NSString::from_str(""));

        // macOS convention: `mainMenu`'s *first* item is always displayed as the bold app name
        // (whatever title it's given is ignored/overridden by the OS) and its submenu is "the app
        // menu". Without one, the DSL's first real top-level item (e.g. "File") gets silently
        // absorbed into that slot instead of showing up as its own menu — so this app-menu slot,
        // with at minimum a working Quit item, is provided here rather than asked of the DSL
        // author, since it's a platform detail of `NSApp.mainMenu`, not something 付録X's
        // `MenuBar`/`MenuBarItem` DSL shape should need to know about.
        let app_menu_item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                m.alloc::<NSMenuItem>(),
                &NSString::from_str(""),
                None,
                &NSString::from_str(""),
            )
        };
        let app_menu = NSMenu::initWithTitle(m.alloc::<NSMenu>(), &NSString::from_str(""));
        let quit_item = unsafe {
            // No target: leaving it nil dispatches through the responder chain to
            // `NSApplication`, which implements `terminate:` itself — the standard way to wire a
            // Quit item without the app needing to be its own `NSApplicationDelegate`.
            NSMenuItem::initWithTitle_action_keyEquivalent(
                m.alloc::<NSMenuItem>(),
                &NSString::from_str("Quit"),
                Some(sel!(terminate:)),
                &NSString::from_str("q"),
            )
        };
        app_menu.addItem(&quit_item);
        app_menu_item.setSubmenu(Some(&app_menu));
        ns.addItem(&app_menu_item);
        Self { ns }
    }

    pub(crate) fn add_item(&self, item: &InnerMenuBarItem) {
        self.ns.addItem(&item.ns);
    }
    pub(crate) fn remove_item(&self, item: &InnerMenuBarItem) {
        self.ns.removeItem(&item.ns);
    }
}

#[cfg(test)]
mod icon_tests {
    use super::*;

    /// §8.9: every `SystemIcon` variant maps to exactly one SF Symbol name, matching
    /// `docs/design/runtime/icon_source_design.md` §2's table exactly, no variant omitted. Pure
    /// logic only (no `NSMenuItem`/`NSImage` construction, so no main-thread requirement) —
    /// runtime `NSMenuItem.image` set/replace/clear coverage lives in the `controls-demo` AppKit
    /// runtime check instead, since `MainThreadMarker` is unavailable inside `cargo test`'s
    /// worker threads (§8.10's own escape hatch).
    #[test]
    fn every_system_icon_variant_maps_to_its_documented_sf_symbol_name() {
        let expected: &[(SystemIcon, &str)] = &[
            (SystemIcon::Add, "plus"),
            (SystemIcon::Remove, "minus"),
            (SystemIcon::Delete, "trash"),
            (SystemIcon::Edit, "pencil"),
            (SystemIcon::Copy, "doc.on.doc"),
            (SystemIcon::Cut, "scissors"),
            (SystemIcon::Paste, "doc.on.clipboard"),
            (SystemIcon::Undo, "arrow.uturn.backward"),
            (SystemIcon::Redo, "arrow.uturn.forward"),
            (SystemIcon::Search, "magnifyingglass"),
            (SystemIcon::Settings, "gearshape"),
            (SystemIcon::Refresh, "arrow.clockwise"),
        ];
        assert_eq!(expected.len(), 12, "must cover every SystemIcon variant");
        for (icon, symbol_name) in expected {
            assert_eq!(
                sf_symbol_name(*icon),
                *symbol_name,
                "{icon:?} must map to SF Symbol {symbol_name:?}"
            );
        }
    }
}
