//! Popup surface implementation for Windows WinUI 3.
//!
//! Hosts arbitrary `UIElement` subtrees inside a lightweight, light-dismissable XAML `Popup`.

use crate::bindings::Microsoft::UI::Xaml::Controls::Primitives::Popup;
use crate::bindings::Microsoft::UI::Xaml::FrameworkElement;
use crate::ffi::{UiCallbackRegistryOwner, invoke_ui_event_callback};
use crate::host::TreeHostPanel;
use elwindui_core::ui::popup::{
    PopupDismissPolicy, PopupFocusPolicy, PopupHost, PopupRequest, PopupSurfaceHandle,
};
use elwindui_core::ui::{UIElementExt, unmount_subtree};
use std::cell::RefCell;
use std::rc::Rc;
use windows::core::Interface;

/// Internal WinUI 3 representation of a standalone popup surface.
pub(crate) struct InnerPopupSurface {
    popup: Popup,
    content_host: TreeHostPanel,
    // `RefCell<Option<..>>`, not a bare `Rc`: `close()`/`on_native_closed()` (via
    // `unmount_owned_content`) must release this surface's own strong reference to the popup
    // content root once teardown completes, not merely unmount it — see AppKit's
    // `InnerPopupSurface::content` (same shape) for the full rationale.
    content: RefCell<Option<Rc<dyn UIElementExt>>>,
    is_open: RefCell<bool>,
    callback_owner: UiCallbackRegistryOwner,
}

impl InnerPopupSurface {
    /// Creates and immediately displays a new popup surface, or `None` if any structural native
    /// setup step fails.
    ///
    /// `request.content` is deliberately **not** attached to `content_host` (`set_tree`) until
    /// every fallible structural step — coordinate conversion, `Popup::new()`, casts, all
    /// `FrameworkElement`/`Popup` property setters, `Closed` handler registration, and
    /// `SetIsOpen(true)` itself — has already succeeded. Attaching content earlier and then
    /// returning `None` on a later failure would let backend/native objects detach/drop before
    /// `ContextMenuService` ever regains control to run `unmount_subtree(content)`, violating
    /// teardown-before-detach. Every early-return path below leaves `content` at its initial
    /// `RefCell::new(None)`, so `request.content` is never backend-attached on any failure path;
    /// the caller (`WinUI3PopupHost::show_popup`, then `ContextMenuService`) unmounts it itself.
    pub(crate) fn show(
        request: PopupRequest,
        owner_canvas: &crate::bindings::Microsoft::UI::Xaml::Controls::Canvas,
    ) -> Option<Rc<Self>> {
        // 1. Core screen -> WinUI local coordinate conversion.
        let local = TreeHostPanel::screen_logical_to_xaml_local(owner_canvas, request.position)?;
        // 2. Native Popup construction.
        let popup = Popup::new().ok()?;
        // 3. Empty content host only — no `set_tree` yet.
        let content_host = TreeHostPanel::new();

        // 4. Casts (of the still-empty host's own Canvas, not of `request.content`).
        let canvas = content_host.canvas();
        let fe: FrameworkElement = canvas.cast().ok()?;
        let uie: crate::bindings::Microsoft::UI::Xaml::UIElement = canvas.cast().ok()?;

        // 5. Configure every structural popup property on the still-empty host/native popup.
        // `request.content` is not reachable through any of this yet.
        fe.SetWidth(request.size.width as f64).ok()?;
        fe.SetHeight(request.size.height as f64).ok()?;
        popup.SetChild(&uie).ok()?;
        popup.SetHorizontalOffset(local.x as f64).ok()?;
        popup.SetVerticalOffset(local.y as f64).ok()?;
        popup.SetShouldConstrainToRootBounds(false).ok()?;
        let is_light_dismiss = request.dismiss_policy == PopupDismissPolicy::LightDismiss;
        popup.SetIsLightDismissEnabled(is_light_dismiss).ok()?;

        // 6. Construct the surface so the `Closed` handler below can weak-reference it. Still not
        // hosting `request.content` — `content` starts `None`.
        let surface = Rc::new(Self {
            popup,
            content_host,
            content: RefCell::new(None),
            is_open: RefCell::new(true),
            callback_owner: UiCallbackRegistryOwner::default(),
        });

        // 7. Register the native `Closed` handler. On registration failure, `surface` is dropped
        // (nothing else holds a strong reference yet) — `Drop::drop` -> `close()` runs with
        // `content == None`, a safe no-op beyond releasing already-configured native objects.
        // `on_native_closed`, not `close`: WinUI has already set `Popup.IsOpen = false` by the time
        // this fires (native light-dismiss included), so this path must not call `SetIsOpen(false)`
        // again — see `on_native_closed`'s own doc comment.
        let weak_surface = Rc::downgrade(&surface);
        let callback_id = surface.callback_owner.register_event(Rc::new(move || {
            let surface: Option<Rc<Self>> = weak_surface.upgrade();
            if let Some(s) = surface {
                s.on_native_closed();
            }
        }));
        surface
            .popup
            .Closed(&windows::Foundation::EventHandler::new(move |_, _| {
                invoke_ui_event_callback(callback_id);
                Ok(())
            }))
            .ok()?;

        // 8. Open the native popup.
        surface.popup.SetIsOpen(true).ok()?;

        // 9. A synchronous `Closed` may already have fired during `SetIsOpen(true)` (e.g.
        // immediate light-dismiss). Content is still unattached in that case — nothing to unmount,
        // `on_native_closed` (already run by the handler) has nothing further to do.
        if !*surface.is_open.borrow() {
            return None;
        }

        // 10. Only now attach the ElwindUI popup content. `content` is stored *before* `set_tree`,
        // so any future/reentrant `close()`/`on_native_closed()` from this point on always has a
        // root to pass to `unmount_subtree`.
        *surface.content.borrow_mut() = Some(Rc::clone(&request.content));
        surface.content_host.set_tree(Rc::clone(&request.content));

        // 11. Re-check after attachment: `set_tree` itself can dispatch synchronously (layout/
        // focus wiring). If a close raced in during step 10, `on_native_closed` already unmounted
        // the now-attached content — nothing further to do here beyond reporting failure.
        if !*surface.is_open.borrow() {
            return None;
        }

        // 12. Focus policy — best-effort, non-fatal (unchanged from before this fix; focus
        // semantics are out of scope here).
        if request.focus_policy == PopupFocusPolicy::Root {
            let uie: crate::bindings::Microsoft::UI::Xaml::UIElement = surface
                .content_host
                .canvas()
                .cast()
                .expect("Canvas as UIElement");
            let _ = uie.Focus(crate::bindings::Microsoft::UI::Xaml::FocusState::Programmatic);
            surface.content_host.focus_element(&request.content);
        }

        // 13. Final check before publishing the surface as successfully open.
        if !*surface.is_open.borrow() {
            return None;
        }

        // 14.
        Some(surface)
    }

    /// Exactly-once close guard shared by [`Self::close`] (framework-controlled) and
    /// [`Self::on_native_closed`] (native-originated). Returns `true` (and marks the surface
    /// closed) only the first time it is called; every subsequent call — from either path, in any
    /// order or reentrant combination — returns `false` and does nothing.
    fn begin_close(&self) -> bool {
        if *self.is_open.borrow() {
            *self.is_open.borrow_mut() = false;
            true
        } else {
            false
        }
    }

    /// Takes and unmounts this surface's own strong reference to the popup content root — see
    /// `content`'s own field doc comment for why a bare `Rc` field would be wrong here. `None`
    /// means either the surface never got as far as attaching content (an aborted `show()`) or this
    /// is a second call after content was already taken; both are no-ops.
    fn unmount_owned_content(&self) {
        if let Some(content) = self.content.borrow_mut().take() {
            unmount_subtree(&content);
        }
    }

    /// Closes and hides the popup surface **on ElwindUI's own initiative** — `PopupDismissAction`,
    /// menu item selection, popup replacement, explicit `PopupSurfaceHandle::close()`, `Drop`.
    ///
    /// Teardown-before-detach, and — because this path controls the native close itself — before
    /// native visibility changes too: `unmount_subtree` (via `unmount_owned_content`) runs before
    /// `SetIsOpen(false)` (visibility) and `TreeHostPanel::clear_tree()` (host tree/native resource
    /// release), matching AppKit's ordering. Unlike AppKit, WinUI3's `clear_tree()` has no existing
    /// deferred-dispatch workaround, so everything here runs synchronously. `begin_close` marks the
    /// surface closed *before* `unmount_subtree` runs, so a reentrant `close()`/`on_native_closed()`
    /// call from inside an `on_unmount` hook — or from `SetIsOpen(false)` below synchronously
    /// re-raising `Popup.Closed`, which now routes to `on_native_closed`, not back into `close` — is
    /// a no-op via that guard.
    pub(crate) fn close(&self) {
        if !self.begin_close() {
            return;
        }
        self.unmount_owned_content();
        self.popup.SetIsOpen(false).ok();
        self.content_host.clear_tree();
    }

    /// Handles a **native-originated** `Popup.Closed` event — critically, including WinUI3's own
    /// automatic light-dismiss (`IsLightDismissEnabled`), which ElwindUI does not control or get
    /// advance notice of. By the time this fires, WinUI has *already* set `Popup.IsOpen = false`
    /// (per Microsoft's own `Popup`/`Popup.Closed` documentation — `Closed` is a post-transition
    /// notification, never a pre-close hook), so:
    ///
    /// - this must **not** call `SetIsOpen(false)` again (WinUI already did it; doing so here would
    ///   be redundant at best and is not part of this surface's own close-initiation responsibility);
    /// - `on_unmount` on this path is *not* guaranteed to observe the native popup as still open —
    ///   only the portable invariant holds: `unmount_subtree` still runs exactly once, still before
    ///   this surface's own `TreeHostPanel::clear_tree()` and content-ownership release.
    ///
    /// This is the documented WinUI3-specific exception to `close()`'s stronger
    /// unmount-before-native-visibility-change ordering — see
    /// `docs/design/runtime/popup_context_menu_design.md` §7 and Issue #161's design-correction
    /// section for the full rationale (Microsoft's `Popup.Closed` contract makes the stronger
    /// ordering unachievable specifically for this one path).
    fn on_native_closed(&self) {
        if !self.begin_close() {
            return;
        }
        self.unmount_owned_content();
        self.content_host.clear_tree();
    }
}

impl Drop for InnerPopupSurface {
    fn drop(&mut self) {
        self.close();
        self.callback_owner.clear();
    }
}

/// Handle implementing [`PopupSurfaceHandle`] for programmatic dismissal.
///
/// Always wraps a live surface — unlike before this revision, `WinUI3PopupHost::show_popup` no
/// longer constructs a handle at all when `InnerPopupSurface::show` fails (it returns `None`
/// instead, per `PopupHost::show_popup`'s fallible contract), so there is no "dummy closed handle"
/// case to represent here anymore.
#[derive(Clone)]
pub struct WinUI3PopupHandle {
    surface: Rc<InnerPopupSurface>,
}

impl PopupSurfaceHandle for WinUI3PopupHandle {
    fn close(&self) {
        self.surface.close();
    }
}

/// Default [`PopupHost`] implementation for WinUI 3.
#[derive(Clone)]
pub struct WinUI3PopupHost {
    owner_canvas: crate::bindings::Microsoft::UI::Xaml::Controls::Canvas,
}

impl WinUI3PopupHost {
    /// Creates a new popup host associated with an owner canvas context.
    pub fn new(owner_canvas: crate::bindings::Microsoft::UI::Xaml::Controls::Canvas) -> Self {
        Self { owner_canvas }
    }
}

impl PopupHost for WinUI3PopupHost {
    fn show_popup(&self, request: PopupRequest) -> Option<Rc<dyn PopupSurfaceHandle>> {
        // `InnerPopupSurface::show` can fail (coordinate conversion, `Popup::new()`, XAML setup) —
        // propagate that as `None` rather than a handle wrapping a nonexistent surface. The caller
        // (`ContextMenuService::open_custom_popup`/`open_custom_menu`) is responsible for unmounting
        // the already-built popup content in that case.
        let surface = InnerPopupSurface::show(request, &self.owner_canvas)?;
        Some(Rc::new(WinUI3PopupHandle { surface }))
    }
}
