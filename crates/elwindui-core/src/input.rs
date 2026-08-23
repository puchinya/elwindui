use crate::base::Point;
use crate::ui::UIElementExt;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

/// Passed to every handler `elwindui_core::ui::dispatch_routed` calls along a bubble path —
/// pure propagation control, deliberately without a payload (`dispatch_routed`'s own `payload: &T`
/// argument carries that, so this stays the same shape for every `#[routed]` field regardless of
/// its own callback signature). A handler sets `handled` to stop further bubbling — WinUI3's
/// `RoutedEventArgs.Handled`. See docs/specs/dsl_spec.md §12 (`#[routed]`).
#[derive(Debug, Default)]
pub struct RoutedEventArgs {
    pub handled: Cell<bool>,
}

/// WinUI3's `VirtualKey`-adjacent `PointerPointProperties.IsXButtonPressed`/mouse-button set,
/// scoped down to what a mouse actually reports. `Eq`/`Hash` so `PointerDispatcher` can track
/// which buttons are currently held in a `HashSet`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// WinUI3's `VirtualKeyModifiers` (`PointerRoutedEventArgs`'s modifier-key snapshot), scoped down
/// to the four keys every desktop platform exposes uniformly. `meta` is the Windows/Command key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct KeyModifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub meta: bool,
}

/// Payload for `on_pointer_pressed`/`on_pointer_released`/`on_pointer_moved`/
/// `on_pointer_canceled`/`on_pointer_entered`/`on_pointer_exited`
/// (docs/design/runtime/ui_tree_design.md). `position` is in the hosting
/// tree's own root-relative coordinate space (the same space `elwindui_core::ui::hit_test`'s `at`
/// argument uses) — not relative to whichever ancestor happens to handle the bubbled event, since a
/// single payload value is shared across every handler on the bubble path. `screen_position`, when
/// available, is the same point in top-left/Y-down logical desktop coordinates. Backends return
/// `None` rather than estimating it when their native conversion fails. `button` is `Some` only for
/// `Pressed`/`Released`; `None` for `Moved`/`Canceled`/`Entered`/`Exited`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointerEventArgs {
    pub position: Point,
    /// The same pointer position in normalized logical desktop coordinates, when the backend can
    /// obtain it without estimation.
    pub screen_position: Option<Point>,
    pub button: Option<MouseButton>,
    pub modifiers: KeyModifiers,
}

/// Payload for `on_pointer_wheel_changed`. `delta_x`/`delta_y` are platform-reported scroll deltas,
/// forwarded unscaled — a backend's own units (AppKit's `NSEvent.scrollingDeltaX/Y`, say) pass
/// through as-is rather than being normalized to some fixed "lines" unit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointerWheelEventArgs {
    pub position: Point,
    pub delta_x: f32,
    pub delta_y: f32,
    pub modifiers: KeyModifiers,
}

/// Payload for `on_tapped`/`on_double_tapped`/`on_right_tapped` — WinUI3's `TappedRoutedEventArgs`/
/// `DoubleTappedRoutedEventArgs`/`RightTappedRoutedEventArgs`, unified into one shape since which
/// gesture occurred is already implied by which field fired. `position` is the release position (in
/// the same root-relative space as `PointerEventArgs::position`), not the original press position.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TappedEventArgs {
    pub position: Point,
    pub modifiers: KeyModifiers,
}

/// The backend-reported half of a mouse event — everything a `PointerDispatcher` needs to decide
/// what to hit-test/dispatch, but with no framework-tree knowledge of its own (a backend constructs
/// one straight from its native event, e.g. AppKit's `NSEvent`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RawPointerEventKind {
    Pressed(MouseButton),
    Released(MouseButton),
    Moved,
    /// Terminates the current implicit capture without producing a normal release or tap.
    Canceled,
    WheelChanged {
        delta_x: f32,
        delta_y: f32,
    },
}

/// A single raw mouse event. `position` is in the hosting tree's root-relative coordinate space;
/// `screen_position` is the optional normalized logical desktop coordinate supplied by the same
/// backend event. `timestamp_ms` is any monotonically increasing clock in milliseconds (AppKit's
/// `NSEvent.timestamp * 1000.0`, say) — only ever compared against other `RawPointerEvent`s from the
/// same dispatcher, never interpreted as wall-clock time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RawPointerEvent {
    pub kind: RawPointerEventKind,
    pub position: Point,
    /// Backend-normalized logical desktop position for this native event.
    pub screen_position: Option<Point>,
    pub modifiers: KeyModifiers,
    pub timestamp_ms: f64,
}

/// A tap is recognized when a press and release of the same button, on the same target, land
/// within this many (root-relative) pixels of each other — WinUI3's `GestureRecognizer` uses an
/// equivalent movement threshold to distinguish a tap from the start of a drag/manipulation.
const TAP_MOVE_THRESHOLD_PX: f32 = 4.0;
/// A second tap only pairs into `on_double_tapped` if it lands within this many milliseconds of the
/// first — mirrors typical desktop double-click timing.
const DOUBLE_TAP_INTERVAL_MS: f64 = 500.0;
/// ...and within this many (root-relative) pixels of the first tap's own release position.
const DOUBLE_TAP_DISTANCE_PX: f32 = 8.0;

fn distance(a: Point, b: Point) -> f32 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    (dx * dx + dy * dy).sqrt()
}

/// `elem`'s own Visual-parent chain (`UIElement::visual_parent`, matching what `dispatch_routed`
/// bubbles along — see that function's own doc comment), innermost (`elem` itself) first, root
/// last. `None` yields an empty chain.
fn ancestor_chain(elem: Option<Rc<dyn UIElementExt>>) -> Vec<Rc<dyn UIElementExt>> {
    let mut chain = Vec::new();
    let mut current = elem;
    while let Some(e) = current {
        current = e.visual_parent();
        chain.push(e);
    }
    chain
}

fn is_in_subtree(target: &Rc<dyn UIElementExt>, subtree: &Rc<dyn UIElementExt>) -> bool {
    let mut current = Some(Rc::clone(target));
    while let Some(element) = current {
        if Rc::ptr_eq(&element, subtree) {
            return true;
        }
        current = element.visual_parent();
    }
    false
}

/// State kept for the button that started the current implicit capture — see
/// `PointerDispatcher`'s own doc comment.
struct PressState {
    target: Rc<dyn UIElementExt>,
    initiating_button: MouseButton,
    start_position: Point,
    held_buttons: HashSet<MouseButton>,
    last_position: Point,
    last_screen_position: Option<Point>,
    last_modifiers: KeyModifiers,
}

/// Tap recognition deferred across `on_pointer_released` dispatch so reentrant cancellation or
/// unmount can suppress it before it becomes observable.
struct PendingTap {
    target: Rc<dyn UIElementExt>,
    button: MouseButton,
    position: Point,
    modifiers: KeyModifiers,
    at_ms: f64,
}

/// The most recent tap this dispatcher fired, kept only long enough to decide whether the *next*
/// one pairs into a `on_double_tapped`.
struct TapRecord {
    target: Rc<dyn UIElementExt>,
    button: MouseButton,
    position: Point,
    at_ms: f64,
}

/// Turns raw mouse input into `elwindui_core::ui::hit_test`/`dispatch_routed` calls against a
/// hosted tree — one instance per hosted tree (owned by a backend's own host view, e.g.
/// `elwindui-backend-appkit`'s `TreeHostView`), fed every native mouse event via [`Self::handle`].
/// Modeled on WinUI3's input manager + `GestureRecognizer` (docs/design/README.md
/// §5.10), with two deliberate simplifications from real WinUI3, both documented where they apply:
///
/// - **Implicit-only capture**: while a button is held, `Moved`/`Released` are redirected to the
///   element that was hit on `Pressed` rather than being re-hit-tested — this reproduces the
///   *effect* of WinUI3's `CapturePointer` (dragging out of an element and releasing back inside it
///   still counts as a tap) without exposing a public capture API on `UIElement` at all. Hover
///   (`Entered`/`Exited`) is computed independently of capture, from the real cursor position, same
///   as WinUI3.
/// - **Single mouse pointer, no multi-touch**: capture is keyed by "any button held", not per
///   pointer-id. If a second button is pressed while the first is still held, it doesn't restart
///   capture or move the tracked press position — only releasing the *initiating* button (the one
///   that started the capture) is evaluated for a tap; capture itself ends once every held button
///   has been released.
#[derive(Default)]
pub struct PointerDispatcher {
    /// Previous call's hover chain (innermost first) — see `ancestor_chain`.
    last_hover: RefCell<Vec<Rc<dyn UIElementExt>>>,
    press: RefCell<Option<PressState>>,
    pending_tap: RefCell<Option<PendingTap>>,
    last_tap: RefCell<Option<TapRecord>>,
}

impl PointerDispatcher {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds one raw mouse event through hit-testing/hover-diffing/gesture-recognition and
    /// dispatches whichever `on_pointer_*`/`on_tapped`/`on_double_tapped`/`on_right_tapped` routed
    /// events result, bubbling from the affected element via `elwindui_core::ui::dispatch_routed`
    /// (see that function's own doc comment). `focus` is the sibling `KeyboardDispatcher`'s own
    /// `FocusTracker` (both dispatchers are owned by the same host, e.g.
    /// `elwindui-backend-appkit`'s `TreeHostIvars`) — on `Pressed`, a hit target that
    /// `is_tab_stop()` is focused with `FocusState::Pointer`, WinUI3's own click-to-focus behavior.
    /// Native leaves (`Button`/`TextArea`/`TabView`) never reach this dispatcher at all (they
    /// receive the OS pointer event directly via ordinary native hit-testing — see
    /// `TreeHostIvars::pointer`'s own doc comment), so their own focus-on-click is handled entirely
    /// separately (`elwindui_core::focus::native_focus_gained`).
    pub fn handle(
        &self,
        root: &Rc<dyn UIElementExt>,
        focus: &crate::focus::FocusTracker,
        event: RawPointerEvent,
    ) {
        match event.kind {
            RawPointerEventKind::Moved => {
                let hit = crate::ui::hit_test(root, event.position);
                self.update_hover(
                    hit.clone(),
                    event.position,
                    event.screen_position,
                    event.modifiers,
                );
                self.update_active_position(&event);
                if let Some(target) = self.captured_or(hit) {
                    let payload = PointerEventArgs {
                        position: event.position,
                        screen_position: event.screen_position,
                        button: None,
                        modifiers: event.modifiers,
                    };
                    crate::ui::dispatch_routed(
                        &target,
                        "on_pointer_moved",
                        &payload,
                        &RoutedEventArgs::default(),
                    );
                }
            }
            RawPointerEventKind::Pressed(button) => {
                let hit = crate::ui::hit_test(root, event.position);
                self.update_hover(
                    hit.clone(),
                    event.position,
                    event.screen_position,
                    event.modifiers,
                );
                let target = self.captured_or(hit);
                self.begin_or_extend_press(button, target.clone(), &event);
                if let Some(target) = &target {
                    if target.is_tab_stop() {
                        focus.set_focus(target, FocusState::Pointer);
                    }
                    let payload = PointerEventArgs {
                        position: event.position,
                        screen_position: event.screen_position,
                        button: Some(button),
                        modifiers: event.modifiers,
                    };
                    crate::ui::dispatch_routed(
                        target,
                        "on_pointer_pressed",
                        &payload,
                        &RoutedEventArgs::default(),
                    );
                }
            }
            RawPointerEventKind::Released(button) => {
                let hit = crate::ui::hit_test(root, event.position);
                self.update_hover(
                    hit.clone(),
                    event.position,
                    event.screen_position,
                    event.modifiers,
                );
                let target = self.captured_or(hit);
                self.prepare_release(button, &event);
                if let Some(target) = &target {
                    let payload = PointerEventArgs {
                        position: event.position,
                        screen_position: event.screen_position,
                        button: Some(button),
                        modifiers: event.modifiers,
                    };
                    crate::ui::dispatch_routed(
                        target,
                        "on_pointer_released",
                        &payload,
                        &RoutedEventArgs::default(),
                    );
                }
                self.finish_pending_tap();
            }
            RawPointerEventKind::Canceled => {
                self.update_active_position(&event);
                self.cancel();
            }
            RawPointerEventKind::WheelChanged { delta_x, delta_y } => {
                if let Some(target) = crate::ui::hit_test(root, event.position) {
                    let payload = PointerWheelEventArgs {
                        position: event.position,
                        delta_x,
                        delta_y,
                        modifiers: event.modifiers,
                    };
                    crate::ui::dispatch_routed(
                        &target,
                        "on_pointer_wheel_changed",
                        &payload,
                        &RoutedEventArgs::default(),
                    );
                }
            }
        }
    }

    /// The currently-captured target, if any button is held; otherwise `hit` (the fresh
    /// hit-test result) — see this type's own doc comment on implicit capture.
    fn captured_or(&self, hit: Option<Rc<dyn UIElementExt>>) -> Option<Rc<dyn UIElementExt>> {
        self.press
            .borrow()
            .as_ref()
            .map(|p| Rc::clone(&p.target))
            .or(hit)
    }

    fn begin_or_extend_press(
        &self,
        button: MouseButton,
        target: Option<Rc<dyn UIElementExt>>,
        event: &RawPointerEvent,
    ) {
        let mut press = self.press.borrow_mut();
        match press.as_mut() {
            Some(existing) => {
                existing.held_buttons.insert(button);
                existing.last_position = event.position;
                existing.last_screen_position = event.screen_position;
                existing.last_modifiers = event.modifiers;
            }
            None => {
                if let Some(target) = target {
                    let mut held_buttons = HashSet::new();
                    held_buttons.insert(button);
                    *press = Some(PressState {
                        target,
                        initiating_button: button,
                        start_position: event.position,
                        held_buttons,
                        last_position: event.position,
                        last_screen_position: event.screen_position,
                        last_modifiers: event.modifiers,
                    });
                }
            }
        }
    }

    fn update_active_position(&self, event: &RawPointerEvent) {
        if let Some(press) = self.press.borrow_mut().as_mut() {
            press.last_position = event.position;
            press.last_screen_position = event.screen_position;
            press.last_modifiers = event.modifiers;
        }
    }

    fn prepare_release(&self, button: MouseButton, event: &RawPointerEvent) {
        *self.pending_tap.borrow_mut() = None;
        let mut press_slot = self.press.borrow_mut();
        let Some(press) = press_slot.as_mut() else {
            return;
        };
        press.last_position = event.position;
        press.last_screen_position = event.screen_position;
        press.last_modifiers = event.modifiers;
        press.held_buttons.remove(&button);
        let is_initiating = button == press.initiating_button;
        let press_target = Rc::clone(&press.target);
        let start_position = press.start_position;
        if press.held_buttons.is_empty() {
            *press_slot = None;
        }
        drop(press_slot);

        if !is_initiating {
            return;
        }
        if distance(event.position, start_position) > TAP_MOVE_THRESHOLD_PX {
            // A real drag, not a tap — also cancels any pending double-tap streak.
            *self.last_tap.borrow_mut() = None;
            return;
        }
        if button == MouseButton::Middle {
            return;
        }
        *self.pending_tap.borrow_mut() = Some(PendingTap {
            target: press_target,
            button,
            position: event.position,
            modifiers: event.modifiers,
            at_ms: event.timestamp_ms,
        });
    }

    fn finish_pending_tap(&self) {
        let Some(pending) = self.pending_tap.borrow_mut().take() else {
            return;
        };
        let tap_event_name = match pending.button {
            MouseButton::Left => "on_tapped",
            MouseButton::Right => "on_right_tapped",
            MouseButton::Middle => return,
        };
        let tapped_payload = TappedEventArgs {
            position: pending.position,
            modifiers: pending.modifiers,
        };
        let is_double = {
            let mut last_tap = self.last_tap.borrow_mut();
            let is_double = last_tap.as_ref().is_some_and(|prev| {
                prev.button == pending.button
                    && Rc::ptr_eq(&prev.target, &pending.target)
                    && (pending.at_ms - prev.at_ms).abs() <= DOUBLE_TAP_INTERVAL_MS
                    && distance(pending.position, prev.position) <= DOUBLE_TAP_DISTANCE_PX
            });
            *last_tap = if is_double {
                None
            } else {
                Some(TapRecord {
                    target: Rc::clone(&pending.target),
                    button: pending.button,
                    position: pending.position,
                    at_ms: pending.at_ms,
                })
            };
            is_double
        };
        // Tap state is installed before invoking user handlers, so a reentrant subtree unmount can
        // clear every retained reference without either a RefCell conflict or a stale write-back.
        crate::ui::dispatch_routed(
            &pending.target,
            tap_event_name,
            &tapped_payload,
            &RoutedEventArgs::default(),
        );
        if is_double {
            crate::ui::dispatch_routed(
                &pending.target,
                "on_double_tapped",
                &tapped_payload,
                &RoutedEventArgs::default(),
            );
        }
    }

    /// Cancels this dispatcher's active implicit capture, if any.
    ///
    /// State is cleared before `on_pointer_canceled` bubbles, so cancellation is idempotent and
    /// reentrant handlers cannot observe or cancel the same gesture twice. The event uses the most
    /// recently observed pointer position and modifiers. Returns `true` only when an active capture
    /// was canceled and notified.
    pub fn cancel(&self) -> bool {
        let Some(press) = self.press.borrow_mut().take() else {
            return false;
        };
        *self.pending_tap.borrow_mut() = None;
        *self.last_tap.borrow_mut() = None;
        let payload = PointerEventArgs {
            position: press.last_position,
            screen_position: press.last_screen_position,
            button: None,
            modifiers: press.last_modifiers,
        };
        crate::ui::dispatch_routed(
            &press.target,
            "on_pointer_canceled",
            &payload,
            &RoutedEventArgs::default(),
        );
        true
    }

    /// Cancels capture only when the captured target belongs to `subtree`, and removes every
    /// retained tap/hover reference into that subtree before it unmounts.
    ///
    /// Returns `true` when an active captured gesture was canceled and notified.
    pub fn cancel_for_subtree(&self, subtree: &Rc<dyn UIElementExt>) -> bool {
        let press_matches = self
            .press
            .borrow()
            .as_ref()
            .is_some_and(|press| is_in_subtree(&press.target, subtree));
        let pending_matches = self
            .pending_tap
            .borrow()
            .as_ref()
            .is_some_and(|tap| is_in_subtree(&tap.target, subtree));
        if pending_matches {
            *self.pending_tap.borrow_mut() = None;
        }
        let last_tap_matches = self
            .last_tap
            .borrow()
            .as_ref()
            .is_some_and(|tap| is_in_subtree(&tap.target, subtree));
        if last_tap_matches {
            *self.last_tap.borrow_mut() = None;
        }
        self.last_hover
            .borrow_mut()
            .retain(|element| !is_in_subtree(element, subtree));
        press_matches && self.cancel()
    }

    /// Fires `on_pointer_exited`/`on_pointer_entered` (non-bubbling per element —
    /// `elwindui_core::ui::dispatch_direct` — see this type's own doc comment) for every element
    /// whose hover state actually changed, by diffing the previous and current ancestor chains.
    /// An element present in both chains (a still-hovered common ancestor) gets neither call.
    fn update_hover(
        &self,
        new_hit: Option<Rc<dyn UIElementExt>>,
        position: Point,
        screen_position: Option<Point>,
        modifiers: KeyModifiers,
    ) {
        let new_chain = ancestor_chain(new_hit);
        let old_chain = self.last_hover.replace(new_chain.clone());
        let payload = PointerEventArgs {
            position,
            screen_position,
            button: None,
            modifiers,
        };
        // Innermost-first: a no-longer-hovered leaf sees its own Exited before its (also
        // no-longer-hovered) ancestors see theirs.
        for elem in old_chain.iter() {
            if !new_chain.iter().any(|n| Rc::ptr_eq(n, elem)) {
                crate::ui::dispatch_direct(
                    elem,
                    "on_pointer_exited",
                    &payload,
                    &RoutedEventArgs::default(),
                );
            }
        }
        // Outermost-first: a newly-hovered container sees its own Entered before its (also
        // newly-hovered) descendants see theirs.
        for elem in new_chain.iter().rev() {
            if !old_chain.iter().any(|o| Rc::ptr_eq(o, elem)) {
                crate::ui::dispatch_direct(
                    elem,
                    "on_pointer_entered",
                    &payload,
                    &RoutedEventArgs::default(),
                );
            }
        }
    }
}

/// WinUI3's `VirtualKey`, scoped down to the subset every desktop platform reports uniformly —
/// see docs/design/runtime/input_focus_design.md. `Character` covers ordinary printable keys
/// (layout-dependent, best-effort — a backend maps its own native keycode/character to this
/// directly; no keyboard-layout remapping is attempted by `elwindui-core` itself).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    Character(char),
    Enter,
    Escape,
    Tab,
    Backspace,
    Delete,
    Space,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
}

/// Payload for `on_key_down`/`on_key_up` (docs/design/runtime/input_focus_design.md). Dispatched
/// only to whichever element `FocusTracker::focused` currently names — unlike the pointer events,
/// there is no hit-testing involved.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KeyEventArgs {
    pub key: Key,
    pub modifiers: KeyModifiers,
    pub is_repeat: bool,
}

/// Payload for `on_text_input` — the IME-committed string, or a directly-typed character when no
/// IME is involved. Only ever carries already-committed text; in-progress IME composition previews
/// are not exposed to the DSL (see docs/design/runtime/input_focus_design.md's own caveat).
#[derive(Debug, Clone, PartialEq)]
pub struct TextInputEventArgs {
    pub text: String,
}

/// The backend-reported half of a raw key event — mirrors `RawPointerEventKind`'s role for mouse
/// input.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RawKeyEventKind {
    Down { is_repeat: bool },
    Up,
}

/// A single raw key event, backend-agnostic — mirrors `RawPointerEvent`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RawKeyEvent {
    pub kind: RawKeyEventKind,
    pub key: Key,
    pub modifiers: KeyModifiers,
    pub timestamp_ms: f64,
}

/// A single raw committed-text event, fed to `KeyboardDispatcher::handle_text_input` — mirrors
/// `RawKeyEvent`'s role for `on_text_input`.
#[derive(Debug, Clone, PartialEq)]
pub struct RawTextInputEvent {
    pub text: String,
}

/// WinUI3's `Control.FocusState` — not just "focused or not", but *how* focus was acquired, so a
/// component can (e.g.) only show a focus ring for keyboard navigation and not for a mouse click.
/// See `crate::focus::FocusTracker::set_focus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusState {
    Unfocused,
    Pointer,
    Keyboard,
    Programmatic,
}

/// A single key combination a `#[shortcut(...)]`-annotated field registers into a
/// `ShortcutRegistry` — docs/design/runtime/input_focus_design.md.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyChord {
    pub key: Key,
    pub modifiers: KeyModifiers,
}

/// Whether a registered shortcut fires regardless of which element (if any) is focused (`Global`,
/// the default — matches a menu accelerator), or only while its own declaring element is on the
/// current focus chain (`Local`, `#[shortcut(.., scope: local)]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutScope {
    Global,
    Local,
}

/// One `#[shortcut(...)]`-annotated field, registered by `elwindui-codegen`'s generated `new()`
/// onto the declaring element itself (`UIElement::declared_shortcuts`) — not yet reachable from any
/// `ShortcutRegistry` at that point, since the element doesn't know which tree/window it'll end up
/// hosted under until it's actually attached. A host's own `set_tree` walks the whole freshly-set
/// tree once and feeds every element's own `declared_shortcuts` into its `ShortcutRegistry`
/// (mirrors how `UIElement::routed_handlers` is populated at construction but only actually fires
/// once wired to a live dispatcher).
#[derive(Debug, Clone)]
pub struct ShortcutDecl {
    pub chord: KeyChord,
    pub scope: ShortcutScope,
    pub event_name: &'static str,
}

/// Matches raw key chords against every `#[shortcut(...)]` registered across a hosted tree — one
/// instance per hosted tree, owned by the same host as its sibling `KeyboardDispatcher`
/// (`ShortcutRegistry` itself has no tree-walking knowledge; `KeyboardDispatcher::handle_key`
/// consults it before bubbling `on_key_down` to the focused element, same ordering WinUI3 uses for
/// `KeyboardAccelerator`s versus ordinary `KeyDown`).
#[derive(Default)]
pub struct ShortcutRegistry {
    bindings: RefCell<Vec<(KeyChord, ShortcutScope, Rc<dyn UIElementExt>, &'static str)>>,
}

impl ShortcutRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&self) {
        self.bindings.borrow_mut().clear();
    }

    pub fn register(
        &self,
        chord: KeyChord,
        scope: ShortcutScope,
        target: Rc<dyn UIElementExt>,
        event_name: &'static str,
    ) {
        self.bindings
            .borrow_mut()
            .push((chord, scope, target, event_name));
    }

    /// Registers every `#[shortcut(...)]` declared anywhere under `tree`, via a depth-first
    /// `visual_children()` walk feeding each element's own `UIElementExt::declared_shortcuts()`
    /// into [`Self::register`] — see `ShortcutDecl`'s own doc comment for why this can't happen at
    /// construction time. Backends call this from their tree host after `set_tree`, so the walk
    /// itself belongs here rather than being re-implemented identically per backend.
    pub fn collect_from_tree(&self, tree: &Rc<dyn UIElementExt>) {
        for decl in tree.declared_shortcuts() {
            self.register(decl.chord, decl.scope, tree.clone(), decl.event_name);
        }
        for child in tree.visual_children() {
            self.collect_from_tree(&child);
        }
    }

    /// `Global` bindings are always eligible. `Local` bindings are only eligible while their own
    /// `target` is somewhere on `focused`'s own ancestor chain (`target` itself, or an ancestor of
    /// it) — matching `#[shortcut(.., scope: local)]`'s documented "only while the declaring
    /// element has focus" semantics, where "has focus" is read the same way `on_key_down` bubbling
    /// would already reach it. Fires the first matching binding's own `event_name` via
    /// `dispatch_direct` (not bubbling — the binding's own `target` already *is* the intended
    /// recipient, e.g. a `Button`'s `on_click`) and returns whether anything matched.
    ///
    /// A `target` that isn't currently reachable — itself or any visual ancestor failing
    /// `UIElementExt::participates_in_layout` (e.g. `Visibility::Collapsed`) — never fires,
    /// `Global` scope included. `collect_from_tree` only runs once (when a backend host calls it
    /// after `set_tree`, see that method's own doc comment), so a binding stays registered across
    /// later visibility changes; this check is what keeps a hidden element's shortcut from firing
    /// instead of also pruning at collection time and permanently losing bindings under
    /// currently-Collapsed elements that might become visible later.
    pub fn try_dispatch(&self, chord: KeyChord, focused: Option<&Rc<dyn UIElementExt>>) -> bool {
        let bindings = self.bindings.borrow();
        for (bound_chord, scope, target, event_name) in bindings.iter() {
            if *bound_chord != chord {
                continue;
            }
            let target_chain = ancestor_chain(Some(Rc::clone(target)));
            if !target_chain.iter().all(|e| e.participates_in_layout()) {
                continue;
            }
            let eligible = match scope {
                ShortcutScope::Global => true,
                ShortcutScope::Local => focused.is_some_and(|focused| {
                    ancestor_chain(Some(Rc::clone(focused)))
                        .iter()
                        .any(|e| Rc::ptr_eq(e, target))
                }),
            };
            if eligible {
                crate::ui::dispatch_direct(target, event_name, &(), &RoutedEventArgs::default());
                return true;
            }
        }
        false
    }
}

/// Turns raw keyboard input into `elwindui_core::ui::dispatch_routed`/`dispatch_direct` calls
/// against a hosted tree's currently-focused element — the keyboard counterpart to
/// `PointerDispatcher`, owned the same way (one instance per hosted tree, fed every native key
/// event via [`Self::handle_key`]/[`Self::handle_text_input`]). Modeled on WinUI3's input manager +
/// `FocusManager` (docs/design/runtime/input_focus_design.md).
#[derive(Default)]
pub struct KeyboardDispatcher {
    pub focus: crate::focus::FocusTracker,
    shortcuts: ShortcutRegistry,
}

impl KeyboardDispatcher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn shortcuts(&self) -> &ShortcutRegistry {
        &self.shortcuts
    }

    /// Evaluates `ShortcutRegistry` first (matching WinUI3's accelerator-before-`KeyDown`
    /// ordering), then — if nothing consumed it — bubbles `on_key_down`/`on_key_up` from the
    /// currently-focused element (a no-op if nothing is focused). If the event is an unhandled
    /// `Down` on `Key::Tab`, moves focus via `FocusTracker::move_focus` (`Previous` if `Shift` is
    /// held, `Next` otherwise) — WinUI3's default `Tab`-cycles-focus behavior.
    pub fn handle_key(&self, root: &Rc<dyn UIElementExt>, event: RawKeyEvent) {
        let chord = KeyChord {
            key: event.key,
            modifiers: event.modifiers,
        };
        let focused = self.focus.focused();
        if self.shortcuts.try_dispatch(chord, focused.as_ref()) {
            return;
        }
        let is_repeat = matches!(event.kind, RawKeyEventKind::Down { is_repeat } if is_repeat);
        let event_name = match event.kind {
            RawKeyEventKind::Down { .. } => "on_key_down",
            RawKeyEventKind::Up => "on_key_up",
        };
        let payload = KeyEventArgs {
            key: event.key,
            modifiers: event.modifiers,
            is_repeat,
        };
        let args = RoutedEventArgs::default();
        if let Some(target) = &focused {
            crate::ui::dispatch_routed(target, event_name, &payload, &args);
        }
        if !args.handled.get()
            && matches!(event.kind, RawKeyEventKind::Down { .. })
            && event.key == Key::Tab
        {
            let direction = if event.modifiers.shift {
                crate::focus::FocusDirection::Previous
            } else {
                crate::focus::FocusDirection::Next
            };
            self.focus.move_focus(root, direction);
        }
    }

    /// Bubbles `on_text_input` from the currently-focused element — a no-op if nothing is focused.
    pub fn handle_text_input(&self, _root: &Rc<dyn UIElementExt>, event: RawTextInputEvent) {
        let Some(target) = self.focus.focused() else {
            return;
        };
        let payload = TextInputEventArgs { text: event.text };
        crate::ui::dispatch_routed(
            &target,
            "on_text_input",
            &payload,
            &RoutedEventArgs::default(),
        );
    }
}

#[cfg(test)]
mod keyboard_tests {
    use super::*;
    use crate::ui::{LayoutExt, VerticalLayout};

    fn tab_stop() -> Rc<VerticalLayout> {
        let node = VerticalLayout::new();
        node.set_tab_stop(true);
        node
    }

    #[test]
    fn tab_moves_focus_to_next_tab_stop() {
        let root = VerticalLayout::new();
        let a = tab_stop();
        let b = tab_stop();
        root.children().add(a.clone());
        root.children().add(b.clone());
        let root: Rc<dyn UIElementExt> = root;

        let dispatcher = KeyboardDispatcher::new();
        dispatcher.handle_key(
            &root,
            RawKeyEvent {
                kind: RawKeyEventKind::Down { is_repeat: false },
                key: Key::Tab,
                modifiers: KeyModifiers::default(),
                timestamp_ms: 0.0,
            },
        );
        let a_dyn: Rc<dyn UIElementExt> = a;
        assert!(Rc::ptr_eq(&dispatcher.focus.focused().unwrap(), &a_dyn));
    }

    #[test]
    fn key_down_bubbles_to_focused_element_and_ancestors() {
        let root = VerticalLayout::new();
        let child = tab_stop();
        root.children().add(child.clone());

        let seen_on_root = Rc::new(std::cell::Cell::new(false));
        {
            let seen_on_root = seen_on_root.clone();
            root.register_routed_handler::<KeyEventArgs>(
                "on_key_down",
                Box::new(move |_payload, _args| {
                    seen_on_root.set(true);
                }),
            );
        }
        let root: Rc<dyn UIElementExt> = root;
        let child: Rc<dyn UIElementExt> = child;

        let dispatcher = KeyboardDispatcher::new();
        assert!(
            dispatcher
                .focus
                .set_focus(&child, crate::input::FocusState::Programmatic)
        );
        dispatcher.handle_key(
            &root,
            RawKeyEvent {
                kind: RawKeyEventKind::Down { is_repeat: false },
                key: Key::Character('a'),
                modifiers: KeyModifiers::default(),
                timestamp_ms: 0.0,
            },
        );
        assert!(seen_on_root.get());
    }

    #[test]
    fn global_shortcut_fires_without_focus() {
        let target = tab_stop();
        let fired = Rc::new(std::cell::Cell::new(false));
        {
            let fired = fired.clone();
            target.register_routed_handler::<()>("on_click", Box::new(move |_, _| fired.set(true)));
        }
        let target: Rc<dyn UIElementExt> = target;

        let registry = ShortcutRegistry::new();
        let chord = KeyChord {
            key: Key::Character('s'),
            modifiers: KeyModifiers {
                control: true,
                ..Default::default()
            },
        };
        registry.register(chord, ShortcutScope::Global, target, "on_click");
        assert!(registry.try_dispatch(chord, None));
        assert!(fired.get());
    }

    #[test]
    fn local_shortcut_requires_focus_chain() {
        let target = tab_stop();
        let other = tab_stop();
        let fired = Rc::new(std::cell::Cell::new(false));
        {
            let fired = fired.clone();
            target.register_routed_handler::<()>("on_click", Box::new(move |_, _| fired.set(true)));
        }
        let target: Rc<dyn UIElementExt> = target;
        let other: Rc<dyn UIElementExt> = other;

        let registry = ShortcutRegistry::new();
        let chord = KeyChord {
            key: Key::Character('f'),
            modifiers: KeyModifiers::default(),
        };
        registry.register(chord, ShortcutScope::Local, target.clone(), "on_click");

        assert!(!registry.try_dispatch(chord, Some(&other)));
        assert!(!fired.get());
        assert!(registry.try_dispatch(chord, Some(&target)));
        assert!(fired.get());
    }

    #[test]
    fn global_shortcut_does_not_fire_for_a_collapsed_target() {
        use crate::layout::Visibility;

        let target = tab_stop();
        target.as_ui_element().set_visibility(Visibility::Collapsed);
        let fired = Rc::new(std::cell::Cell::new(false));
        {
            let fired = fired.clone();
            target.register_routed_handler::<()>("on_click", Box::new(move |_, _| fired.set(true)));
        }
        let target: Rc<dyn UIElementExt> = target;

        let registry = ShortcutRegistry::new();
        let chord = KeyChord {
            key: Key::Character('s'),
            modifiers: KeyModifiers {
                control: true,
                ..Default::default()
            },
        };
        // `Global` scope would normally fire regardless of focus (see
        // `global_shortcut_fires_without_focus` above) — a Collapsed target must still suppress
        // it, matching every other non-participating exclusion (render, hit-test, tab order).
        registry.register(chord, ShortcutScope::Global, target, "on_click");
        assert!(!registry.try_dispatch(chord, None));
        assert!(!fired.get());
    }
}
