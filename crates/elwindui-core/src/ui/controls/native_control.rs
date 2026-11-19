//! The abstract base every real OS-toolkit-backed leaf inherits — the marker that tells the tree a subtree is opaque native content, plus the `background`/`tooltip`/`#[text_style]` props shared by all of them.

use super::*;
use crate::graphics::{CascadedTextStyle, TextStyleStorage};
use crate::ui::native::NativePeer;
use std::cell::OnceCell;

/// The Core-owned base of every native leaf (`Button`, `TextBox`, `TabView`, ...). Issue #313,
/// `docs/design/runtime/native_peer_design.md`.
///
/// Always a leaf as far as this tree is concerned: whatever lives beneath it in its own
/// backend-managed hierarchy (for example `TabView`'s pages) is opaque here. The OS widget is a
/// [`NativePeer`] owned by this storage. Core keeps the logical `background`, `tooltip`, and text
/// style, and pushes them to the peer once it exists.
///
/// # Peer lifecycle
///
/// A control constructed with [`NativeControl::construct`] has no peer yet. The peer is realized
/// lazily, exactly once, the first time the control takes part in hosted layout or render
/// collection (`measure_override` / `try_as_native_control`). Both only happen once the control
/// is mounted under a backend host, so constructing controls never requires a backend.
/// Realization asks the most-derived class for its peer through
/// [`NativeControlExt::__create_native_peer`]. Unmounting, remounting, hiding, and relayout never
/// create a second peer. Dropping the control calls [`NativePeer::dispose`] once.
///
/// Native controls that still create their widget eagerly in their own constructor pass it to
/// `NativeControl::construct(Some(peer))` instead.
///
/// `collect_render_items<H>` (implemented by `engine::record_group_commands<H>`) downcasts a
/// leaf's `try_as_native_control()` result (the peer's [`NativePeer::handle`]) directly to `H`.
#[elwindui_macros::component(inherits crate::ui::UIElement, abstract_class)]
#[text_style]
#[prop(semantic_brush, background: Option<crate::graphics::Brush>)]
#[prop(tooltip: Option<String>)]
pub struct NativeControl {
    /// The most-derived control, so realization reaches its `__create_native_peer` override.
    self_weak: Weak<dyn NativeControlExt>,
    peer: OnceCell<Rc<dyn NativePeer>>,
    background: RefCell<Option<Brush>>,
    tooltip: RefCell<String>,
    text_style: TextStyleStorage,
    /// The style last accepted by the peer. Lets `sync_text_style` skip a redundant native call
    /// when nothing changed since the previous measure pass.
    applied: RefCell<Option<CascadedTextStyle>>,
}

#[elwindui_macros::component]
impl NativeControl {
    #[overrides]
    fn measure_override(&self, available: Size) -> Size {
        let peer = self.__native_peer();
        if peer.refresh_text_style_on_measure() {
            self.reapply_text_style();
        } else {
            self.sync_text_style();
        }
        peer.measure(available)
    }
    #[overrides]
    fn try_as_native_control(&self) -> Option<&dyn Any> {
        Some(self.__native_peer().handle())
    }
    #[overrides]
    fn as_text_style_owner(&self) -> Option<&dyn TextStyleOwner> {
        Some(self)
    }

    /// Creates this control's native peer. Called at most once, by the realization path.
    ///
    /// Each concrete native control overrides this to create its typed peer through
    /// [`crate::ui::native::create_native_peer`], push its own logical state, and install its
    /// native callbacks. The default returns `None`, which is only valid for a control that
    /// passed its peer to `NativeControl::construct(Some(peer))` or that overrides both
    /// `measure_override` and `try_as_native_control` itself.
    #[doc(hidden)]
    #[overridable]
    fn __create_native_peer(&self) -> Option<Rc<dyn NativePeer>> {
        None
    }

    /// Returns this control's native peer, realizing it first if needed.
    ///
    /// # Panics
    ///
    /// Panics when realization is needed and no native backend is registered on this thread, when
    /// the backend does not support this control, or when the control provides no peer at all.
    #[doc(hidden)]
    fn __native_peer(&self) -> &Rc<dyn NativePeer> {
        if let Some(peer) = self.peer.get() {
            return peer;
        }
        // `self` is this base storage; the override lives on the most-derived control.
        let owner: Rc<dyn NativeControlExt> = self
            .self_weak
            .upgrade()
            .expect("elwindui: a NativeControl was realized while its owner was being dropped");
        let peer = owner.__create_native_peer().unwrap_or_else(|| {
            panic!(
                "elwindui: native control `{}` has no native peer; it must override \
                 `__create_native_peer` or pass its peer to `NativeControl::construct`",
                owner.type_name()
            )
        });
        self.install_native_peer(peer)
    }

    /// Whether this control's native peer has been realized.
    #[doc(hidden)]
    fn __has_native_peer(&self) -> bool {
        self.peer.get().is_some()
    }

    /// Sets an explicit native-control background, or removes it (`None`) so the platform's own
    /// default appearance applies again. Stored as logical state and pushed to the peer when it
    /// exists (or at realization).
    fn set_background(&self, background: Option<Brush>) {
        let Some(background) = background else {
            self.clear_background();
            return;
        };
        *self.background.borrow_mut() = Some(background.clone());
        if let Some(peer) = self.peer.get() {
            peer.apply_background(Some(&background));
        }
        self.invalidate();
    }

    /// Removes an explicit background so the platform's own default appearance applies again.
    ///
    /// Kept distinct from `set_background(None)`: `emit_resync`'s generic `clear_<name>()`
    /// convention calls it directly wherever a DSL value transitions from `Some` to not-written.
    fn clear_background(&self) {
        self.background.borrow_mut().take();
        if let Some(peer) = self.peer.get() {
            peer.apply_background(None);
        }
        self.invalidate();
    }

    /// Sets the hover-delayed explanatory text shown for this control.
    ///
    /// Declared on every native leaf because `docs/specs/ui_spec.md#23-common-properties`
    /// specifies `tooltip` as an attribute any control may carry. Passing an empty string removes
    /// the tooltip, so there is no `clear_tooltip`. A tooltip has no layout effect, so nothing is
    /// invalidated.
    fn set_tooltip(&self, tooltip: &str) {
        *self.tooltip.borrow_mut() = tooltip.to_string();
        if let Some(peer) = self.peer.get() {
            peer.set_tooltip((!tooltip.is_empty()).then_some(tooltip));
        }
    }

    /// Constructs the native base. `peer` is `None` for a lazily realized control, or the
    /// already-created widget for a control that builds it eagerly in its own constructor (a
    /// control not yet migrated to a Core-owned Component). Internal: `NativeControl` is abstract
    /// and has no public constructor inputs.
    fn construct(peer: Option<Rc<dyn NativePeer>>) -> Self {
        let cell = OnceCell::new();
        if let Some(peer) = peer {
            let _ = cell.set(peer);
        }
        Self {
            base: UIElement::construct(),
            self_weak: __self_weak.clone(),
            peer: cell,
            background: RefCell::new(None),
            tooltip: RefCell::new(String::new()),
            text_style: TextStyleStorage::new(),
            applied: RefCell::new(None),
        }
    }
}

impl NativeControl {
    /// Installs a freshly created peer and pushes this base's logical state to it.
    fn install_native_peer(&self, peer: Rc<dyn NativePeer>) -> &Rc<dyn NativePeer> {
        if self.peer.set(peer).is_err() {
            panic!("elwindui: a NativeControl's native peer was realized more than once");
        }
        let peer = self.peer.get().expect("peer was just installed");
        // Clone the logical values out first: the peer may re-enter this control.
        let background = self.background.borrow().clone();
        if background.is_some() {
            peer.apply_background(background.as_ref());
        }
        let tooltip = self.tooltip.borrow().clone();
        if !tooltip.is_empty() {
            peer.set_tooltip(Some(&tooltip));
        }
        self.applied.borrow_mut().take();
        peer
    }

    /// Pulls this element's resolved text style and pushes it to the peer, but only when it
    /// differs from what the peer last accepted.
    ///
    /// Pull-based (called from `measure_override`, which runs every layout pass) because a text
    /// style can inherit from an ancestor that changed. Every text-style setter already calls
    /// `invalidate_measure()`, so this runs again before anything is drawn. Does nothing before
    /// the peer is realized.
    #[doc(hidden)]
    pub fn sync_text_style(&self) {
        let Some(peer) = self.peer.get() else {
            return;
        };
        if !peer.supports_text_style() {
            return;
        }
        let cascaded = self.cascaded_text_style();
        if self.applied.borrow().as_ref() != Some(&cascaded) && peer.apply_text_style(&cascaded) {
            // Only cache a style after the peer accepted it, so a transient native failure is
            // retried on a later layout pass.
            *self.applied.borrow_mut() = Some(cascaded);
        }
    }

    /// Reapplies the current resolved style after a native mutation that discards attributed
    /// text (for example AppKit `NSButton.setTitle`), where cache equality alone cannot prove the
    /// widget still carries the style.
    #[doc(hidden)]
    pub fn reapply_text_style(&self) {
        self.applied.borrow_mut().take();
        self.sync_text_style();
    }

    /// The realized peer, if any, without realizing it.
    #[doc(hidden)]
    pub fn native_peer_if_realized(&self) -> Option<&Rc<dyn NativePeer>> {
        self.peer.get()
    }
}

impl TextStyleOwner for NativeControl {
    fn text_style_storage(&self) -> &TextStyleStorage {
        &self.text_style
    }
}

impl Drop for NativeControl {
    fn drop(&mut self) {
        if let Some(peer) = self.peer.get() {
            peer.dispose();
        }
    }
}
