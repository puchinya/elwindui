//! Issue #225: a `Window`'s content host must give a real, non-zero logical viewport to the Core
//! layout root before/at first show, and keep it synchronized across resize. Before the fix, the
//! root `TreeHostPanel` `Canvas`'s own `SizeChanged` never fired for a plain (no-menu-bar)
//! `Window.Content`, so `TreeHostPanel::relayout_static` only ever ran once, with
//! `available = 0x0` — every descendant, including a bare self-drawn `UIElement` leaf with no
//! `measure_override` beyond honoring its own explicit `width`/`height`, stayed permanently
//! `arranged_width == 0` and un-hit-testable.
//!
//! A WinUI3 `Application`/message loop is process-lifetime, one-shot state (`elwindui::init()` +
//! `elwindui::application::run(..)`) — a second, independent `init()`/`run()` in the same test
//! *binary* process fails to load `XamlControlsResources` (confirmed directly: attempting a
//! second `#[test]` with its own `init()`/`run()` in one file reported `MenuBar`'s own required
//! resource, `AccentAcrylicBackgroundFillColorDefaultBrush`, missing). Since every `tests/*.rs`
//! file compiles to its own separate binary/process, this regression lives in its own file
//! (rather than joining `window_mount_hide_close.rs`, which already owns one such `run()` call)
//! and both scenarios below run inside a single `run()` call, mirroring that file's own
//! multi-scenario-per-`run()` convention.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]
#![cfg(all(feature = "backend-winui3", target_os = "windows"))]

use elwindui::core::base::Size;
use elwindui::core::graphics::RenderContext;
use elwindui::core::ui::{UIElementExt, WindowExt};
use std::cell::Cell;
use std::rc::Rc;

thread_local! {
    static SIZE_SYNC_BUILD_COUNT: Cell<u32> = const { Cell::new(0) };
    static MENU_SIZE_SYNC_BUILD_COUNT: Cell<u32> = const { Cell::new(0) };
}

/// A minimal self-drawn leaf reproducing the #225 probe topology: no `render()`-visible content
/// is needed to observe the bug, only that `arranged_width`/`arranged_height` reflect a real
/// Window viewport once laid out under a real `Window`.
#[elwindui::class(inherits = elwindui::core::ui::UIElement)]
pub struct SizeProbeCanvas {}

#[elwindui::class]
impl SizeProbeCanvas {
    fn construct() -> Self {
        Self {
            base: elwindui::core::ui::UIElement::construct(),
        }
    }

    #[overrides]
    fn measure_override(&self, _available: Size) -> Size {
        Size {
            width: self.width().unwrap_or(0.0),
            height: self.height().unwrap_or(0.0),
        }
    }

    #[overrides]
    fn render(&self, _context: &mut RenderContext<'_>) {}
}

#[elwindui::component(inherits Window)]
struct SizeSyncWindow {
    #[param]
    root: Rc<SizeProbeCanvas>,

    body: view! {
        on_mount {
            SIZE_SYNC_BUILD_COUNT.with(|c| c.set(c.get() + 1));
        }
        title: "issue 225 size sync"
        width: 640.0
        height: 480.0
        content: root
    },
}

#[elwindui::component]
impl SizeSyncWindow {}

#[elwindui::component(inherits Window)]
struct MenuBarSizeSyncWindow {
    #[param]
    root: Rc<SizeProbeCanvas>,
    #[param]
    menu_bar: Rc<elwindui::ui::MenuBar>,

    body: view! {
        on_mount {
            MENU_SIZE_SYNC_BUILD_COUNT.with(|c| c.set(c.get() + 1));
        }
        title: "issue 225 menu-bar size sync"
        width: 640.0
        height: 480.0
        menu_bar: menu_bar
        content: root
    },
}

#[elwindui::component]
impl MenuBarSizeSyncWindow {}

/// D1/T1 (contract §8.1) and D2 (contract §8.2), combined into one `#[test]` — see the module doc
/// comment for why both scenarios share a single `elwindui::init()`/`application::run()` call.
#[test]
fn winui3_window_content_host_gets_nonzero_arranged_size_and_tracks_resize() {
    elwindui::init().expect("initialize WinUI3");
    SIZE_SYNC_BUILD_COUNT.with(|c| c.set(0));
    MENU_SIZE_SYNC_BUILD_COUNT.with(|c| c.set(0));

    elwindui::application::run(|| {
        // Part 1 (D1, contract §8.1): direct `Window -> self-drawn UIElement` content gets a
        // non-zero arranged size at first show, and that size tracks a subsequent real Window
        // resize — with no extra `on_mount` (no duplicate build/remount from the resize path).
        //
        // Deliberately no explicit `width`/`height` on the probe: `UIElement::arrange`'s own
        // "explicit size always wins via `min()` against the available slot" rule means a fixed
        // explicit size would clamp `arranged_width`/`arranged_height` to that constant
        // regardless of the real viewport, masking the very defect this test exists to catch.
        // Default `HorizontalAlignment`/`VerticalAlignment::Stretch` is what must pick up the
        // Window's real content-host viewport here — the #225 bug scenario exactly.
        let probe = SizeProbeCanvas::new();

        let window: Rc<SizeSyncWindow> = SizeSyncWindow::new(probe.clone());
        window.show();
        assert_eq!(
            SIZE_SYNC_BUILD_COUNT.with(Cell::get),
            1,
            "first show() must mount and build exactly once"
        );

        let width_after_show = probe.arranged_width().unwrap_or(0.0);
        let height_after_show = probe.arranged_height().unwrap_or(0.0);
        assert!(
            width_after_show > 0.0,
            "direct self-drawn content must get a non-zero arranged width after show() \
             (was {width_after_show}) — the Window content host must supply a real viewport \
             to the first Core layout pass"
        );
        assert!(
            height_after_show > 0.0,
            "direct self-drawn content must get a non-zero arranged height after show() \
             (was {height_after_show})"
        );

        // R3: a real native resize must propagate through Window.SizeChanged to a new arranged
        // rect, not leave a stale viewport behind.
        let resized_width = window.width() + 120.0;
        let resized_height = window.height() + 80.0;
        window.set_width(resized_width);
        window.set_height(resized_height);

        let width_after_resize = probe.arranged_width().unwrap_or(0.0);
        let height_after_resize = probe.arranged_height().unwrap_or(0.0);
        assert!(
            width_after_resize > 0.0 && height_after_resize > 0.0,
            "arranged size must remain non-zero after resize (was {width_after_resize}x{height_after_resize})"
        );
        assert_ne!(
            (width_after_show, height_after_show),
            (width_after_resize, height_after_resize),
            "resize must actually change the arranged rect, not leave a stale viewport \
             (before: {width_after_show}x{height_after_show}, after: \
             {width_after_resize}x{height_after_resize})"
        );
        assert_eq!(
            SIZE_SYNC_BUILD_COUNT.with(Cell::get),
            1,
            "a resize must not trigger an extra mount/build"
        );

        window.close();

        // Part 2 (D2, contract §8.2): a `Window` with a menu bar must exclude the menu bar's own
        // height from the content host's viewport, and keep doing so across resize — the
        // wrapping `Canvas` and the content host must not have two independent (and potentially
        // disagreeing) sizing authorities.
        //
        // See Part 1's comment above: no explicit size on the probe, so `Stretch` picks up the
        // real (menu-bar-inset) content-host viewport instead of clamping to a fixed constant
        // that would mask a resize regression.
        let probe = SizeProbeCanvas::new();
        let menu_bar = elwindui::ui::MenuBar::new();

        let window: Rc<MenuBarSizeSyncWindow> = MenuBarSizeSyncWindow::new(probe.clone(), menu_bar);
        window.show();
        assert_eq!(MENU_SIZE_SYNC_BUILD_COUNT.with(Cell::get), 1);

        let content_height_after_show = probe.arranged_height().unwrap_or(0.0);
        assert!(
            content_height_after_show > 0.0,
            "menu-bar content host must get a non-zero arranged height after show()"
        );
        assert!(
            (content_height_after_show as f64) < (window.height() as f64),
            "content host height ({content_height_after_show}) must be strictly less than \
             the Window's own height ({}) — it must exclude the menu bar's extent",
            window.height()
        );

        let resized_height = window.height() + 90.0;
        window.set_height(resized_height);

        let content_height_after_resize = probe.arranged_height().unwrap_or(0.0);
        assert!(
            content_height_after_resize > 0.0,
            "content host height must remain non-zero after resize"
        );
        assert!(
            (content_height_after_resize as f64) < (window.height() as f64),
            "content host height ({content_height_after_resize}) must still be strictly less \
             than the resized Window's own height ({}) after resize",
            window.height()
        );
        assert_ne!(
            content_height_after_show, content_height_after_resize,
            "resize must actually change the content host's arranged height"
        );
        assert_eq!(
            MENU_SIZE_SYNC_BUILD_COUNT.with(Cell::get),
            1,
            "a resize must not trigger an extra mount/build"
        );

        window.close();
    });
}
