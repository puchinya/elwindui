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
/// Realization is a two-phase protocol (`native_peer_design.md` §6):
///
/// 1. The most-derived [`NativeControlExt::__create_native_peer`] only creates the peer through
///    the factory and records its typed handle. It pushes no state and installs no callback.
/// 2. This base installs the peer, so every later lookup (including a reentrant one) sees it,
///    then pushes its own logical state (`background`, `tooltip`).
/// 3. The most-derived [`NativeControlExt::__initialize_native_peer`] pushes the control's own
///    logical state and installs native callbacks last, so no OS event reaches Core before the
///    peer carries the complete initial state.
///
/// Unmounting, remounting, hiding, and relayout never create a second peer. Dropping the control
/// calls [`NativePeer::dispose`] once.
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
    /// Set when the peer rejected the current background (`apply_background` returned `false`).
    /// Retried on every later measure pass until the peer accepts it or a newer value replaces it.
    background_pending: Cell<bool>,
    /// Bumped by every background push. A push records its value before calling the peer and
    /// writes `background_pending` only if no other push (a reentrant one) ran meanwhile, so a
    /// stale result never overwrites a newer one (`native_peer_design.md` §6).
    background_generation: Cell<u64>,
    tooltip: RefCell<String>,
    text_style: TextStyleStorage,
    /// The style last accepted by the peer. Lets `sync_text_style` skip a redundant native call
    /// when nothing changed since the previous measure pass.
    applied: RefCell<Option<CascadedTextStyle>>,
    /// Bumped by every text-style push and every cache invalidation. A push caches its style
    /// only if the generation is unchanged when the peer returns, so a reentrant push or
    /// invalidation is never overwritten by an older style.
    text_style_generation: Cell<u64>,
}

#[elwindui_macros::component]
impl NativeControl {
    #[overrides]
    fn measure_override(&self, available: Size) -> Size {
        // Read before realizing: realization itself pushes the background once.
        let retry_background = self.background_pending.get();
        let peer = self.__native_peer();
        if retry_background {
            self.push_background(peer);
        }
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

    /// Realization phase 1: creates this control's native peer. Called at most once.
    ///
    /// Each concrete native control overrides this to create its typed peer through
    /// [`crate::ui::native::create_native_peer`] and record the typed handle it needs for its own
    /// setters. It must not push state or install callbacks: that is
    /// [`NativeControlExt::__initialize_native_peer`]'s job, after the base state is pushed. The
    /// default returns `None`, which is only valid for a control that passed its peer to
    /// `NativeControl::construct(Some(peer))` or that overrides both `measure_override` and
    /// `try_as_native_control` itself.
    #[doc(hidden)]
    #[overridable]
    fn __create_native_peer(&self) -> Option<Rc<dyn NativePeer>> {
        None
    }

    /// Realization phase 3: pushes the control's own logical state to the just-installed peer,
    /// then installs its native callbacks. Called exactly once, right after this base pushed its
    /// own state. The peer is already installed, so a reentrant setter pushes straight to it and a
    /// reentrant `__native_peer()` returns it without creating another.
    #[doc(hidden)]
    #[overridable]
    fn __initialize_native_peer(&self) {}

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
        let peer = self.install_native_peer(peer);
        owner.__initialize_native_peer();
        peer
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
        *self.background.borrow_mut() = Some(background);
        if let Some(peer) = self.peer.get() {
            self.push_background(peer);
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
            self.push_background(peer);
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
            background_pending: Cell::new(false),
            background_generation: Cell::new(0),
            tooltip: RefCell::new(String::new()),
            text_style: TextStyleStorage::new(),
            applied: RefCell::new(None),
            text_style_generation: Cell::new(0),
        }
    }
}

impl NativeControl {
    /// Realization phase 2: installs a freshly created peer and pushes this base's logical
    /// state to it.
    fn install_native_peer(&self, peer: Rc<dyn NativePeer>) -> &Rc<dyn NativePeer> {
        if self.peer.set(peer).is_err() {
            panic!("elwindui: a NativeControl's native peer was realized more than once");
        }
        let peer = self.peer.get().expect("peer was just installed");
        if self.background.borrow().is_some() {
            self.push_background(peer);
        }
        // Clone the logical value out first: the peer may re-enter this control.
        let tooltip = self.tooltip.borrow().clone();
        if !tooltip.is_empty() {
            peer.set_tooltip(Some(&tooltip));
        }
        self.bump_text_style_generation();
        self.applied.borrow_mut().take();
        peer
    }

    /// Pushes the current logical background (or its absence) to `peer` and records whether the
    /// peer rejected it, so the next measure pass retries it. A rejected value is never treated
    /// as applied; a newer `set_background`/`clear_background` simply replaces the pending one.
    ///
    /// Reentrancy (`native_peer_design.md` §6): when the peer re-enters a background setter while
    /// this push is in flight, the inner push is newer, so this outer result is stale. It is
    /// discarded, and the current value is marked pending instead, because the outer native call
    /// may have completed after the inner one. The next measure pass re-pushes the newest value
    /// once, so the native widget converges on the last logical write.
    fn push_background(&self, peer: &Rc<dyn NativePeer>) {
        let generation = self.background_generation.get().wrapping_add(1);
        self.background_generation.set(generation);
        // Clone the logical value out first: the peer may re-enter this control.
        let background = self.background.borrow().clone();
        let accepted = peer.apply_background(background.as_ref());
        if self.background_generation.get() == generation {
            self.background_pending.set(!accepted);
        } else {
            self.background_pending.set(true);
        }
    }

    fn bump_text_style_generation(&self) -> u64 {
        let generation = self.text_style_generation.get().wrapping_add(1);
        self.text_style_generation.set(generation);
        generation
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
        // Compare in its own statement so no `RefCell` borrow is held across the peer call.
        let unchanged = self.applied.borrow().as_ref() == Some(&cascaded);
        if unchanged {
            return;
        }
        let generation = self.bump_text_style_generation();
        let accepted = peer.apply_text_style(&cascaded);
        if self.text_style_generation.get() != generation {
            // A reentrant push or invalidation ran during this call, so this result is stale and
            // this outer native call may have completed after the newer one. Drop the cache so
            // the next measure pass re-pushes the newest resolved style once (§6).
            self.applied.borrow_mut().take();
        } else if accepted {
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
        self.bump_text_style_generation();
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

/// Intrinsic accessibility semantics for a native leaf: `role` plus the actions it supports.
pub(crate) fn native_semantics(
    role: AccessibilityRole,
    actions: &[AccessibilityActionKind],
) -> AccessibilitySemantics {
    let mut semantics = AccessibilitySemantics::new(role);
    semantics.actions = actions.to_vec();
    semantics
}

/// Updates `node`'s intrinsic accessibility semantics in place. Does nothing before they were
/// first set (every native leaf sets them in `on_constructed`).
pub(crate) fn update_intrinsic_semantics(
    node: &dyn UIElementExt,
    update: impl FnOnce(&mut AccessibilitySemantics),
) {
    let Some(mut semantics) = node.intrinsic_accessibility_semantics() else {
        return;
    };
    update(&mut semantics);
    node.set_intrinsic_accessibility_semantics(semantics);
}

/// Mirrors a native leaf's `enabled` state into its accessibility `disabled` state.
pub(crate) fn sync_native_enabled(node: &dyn UIElementExt, enabled: bool) {
    update_intrinsic_semantics(node, |semantics| semantics.state.disabled = !enabled);
}

/// Calls a stored user change callback without holding the `RefCell` borrow during the call, so
/// the callback may replace itself or re-enter the control (§6).
pub(crate) fn notify_change<T>(slot: &RefCell<Option<Rc<dyn Fn(T)>>>, value: T) {
    let callback = slot.borrow().clone();
    if let Some(callback) = callback {
        callback(value);
    }
}

/// A native-to-Core callback that holds only a `Weak` to the owning node. It upgrades first and
/// keeps the strong reference for the whole call (§6 in-flight callbacks), downcasts to the
/// sealed concrete control `C`, and does nothing once the control is gone.
pub(crate) fn weak_native_callback<C: 'static, A: 'static>(
    node: &dyn UIElementExt,
    on_event: impl Fn(&C, A) + 'static,
) -> Box<dyn Fn(A)> {
    let node = node
        .as_ui_element()
        .visual_collection
        .owner_rc()
        .map(|node| Rc::downgrade(&node));
    Box::new(move |arg: A| {
        let Some(node) = node.as_ref().and_then(Weak::upgrade) else {
            return;
        };
        let Some(control) = node.as_any().downcast_ref::<C>() else {
            return;
        };
        on_event(control, arg);
        drop(node);
    })
}
