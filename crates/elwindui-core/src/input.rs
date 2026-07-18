use crate::base::Point;
use crate::ui::UIElementExt;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

/// Passed to every handler `elwindui_core::ui::dispatch_routed` calls along a bubble path —
/// pure propagation control, deliberately without a payload (`dispatch_routed`'s own `payload: &T`
/// argument carries that, so this stays the same shape for every `#[routed]` field regardless of
/// its own callback signature). A handler sets `handled` to stop further bubbling — WinUI3's
/// `RoutedEventArgs.Handled`. See docs/elwindui_spec.md 4章 (`#[routed]`).
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KeyModifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub meta: bool,
}

/// Payload for `on_pointer_pressed`/`on_pointer_released`/`on_pointer_moved`/`on_pointer_entered`/
/// `on_pointer_exited` (docs/elwindui_gui_framework_design.md §5.10). `position` is in the hosting
/// tree's own root-relative coordinate space (the same space `elwindui_core::ui::hit_test`'s `at`
/// argument uses) — not relative to whichever ancestor happens to handle the bubbled event, since a
/// single payload value is shared across every handler on the bubble path. `button` is `Some` only
/// for `Pressed`/`Released`; `None` for `Moved`/`Entered`/`Exited`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointerEventArgs {
    pub position: Point,
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
    WheelChanged { delta_x: f32, delta_y: f32 },
}

/// A single raw mouse event, in the hosting tree's own root-relative coordinate space (see
/// `PointerEventArgs::position`'s own doc comment). `timestamp_ms` is any monotonically increasing
/// clock in milliseconds (AppKit's `NSEvent.timestamp * 1000.0`, say) — only ever compared against
/// other `RawPointerEvent`s from the same dispatcher, never interpreted as wall-clock time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RawPointerEvent {
    pub kind: RawPointerEventKind,
    pub position: Point,
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

/// `elem`'s own logical-parent chain (`UIElement::parent`, matching what `dispatch_routed` bubbles
/// along), innermost (`elem` itself) first, root last. `None` yields an empty chain.
fn ancestor_chain(elem: Option<Rc<dyn UIElementExt>>) -> Vec<Rc<dyn UIElementExt>> {
    let mut chain = Vec::new();
    let mut current = elem;
    while let Some(e) = current {
        current = e.parent();
        chain.push(e);
    }
    chain
}

/// State kept for the button that started the current implicit capture — see
/// `PointerDispatcher`'s own doc comment.
struct PressState {
    target: Rc<dyn UIElementExt>,
    initiating_button: MouseButton,
    start_position: Point,
    held_buttons: HashSet<MouseButton>,
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
/// Modeled on WinUI3's input manager + `GestureRecognizer` (docs/elwindui_gui_framework_design.md
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
    last_tap: RefCell<Option<TapRecord>>,
}

impl PointerDispatcher {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds one raw mouse event through hit-testing/hover-diffing/gesture-recognition and
    /// dispatches whichever `on_pointer_*`/`on_tapped`/`on_double_tapped`/`on_right_tapped` routed
    /// events result, bubbling from the affected element via `elwindui_core::ui::dispatch_routed`
    /// (see that function's own doc comment).
    pub fn handle(&self, root: &Rc<dyn UIElementExt>, event: RawPointerEvent) {
        match event.kind {
            RawPointerEventKind::Moved => {
                let hit = crate::ui::hit_test(root, event.position);
                self.update_hover(hit.clone(), event.position, event.modifiers);
                if let Some(target) = self.captured_or(hit) {
                    let payload = PointerEventArgs {
                        position: event.position,
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
                self.update_hover(hit.clone(), event.position, event.modifiers);
                let target = self.captured_or(hit);
                if let Some(target) = &target {
                    let payload = PointerEventArgs {
                        position: event.position,
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
                self.begin_or_extend_press(button, target, event.position);
            }
            RawPointerEventKind::Released(button) => {
                let hit = crate::ui::hit_test(root, event.position);
                self.update_hover(hit.clone(), event.position, event.modifiers);
                let target = self.captured_or(hit);
                if let Some(target) = &target {
                    let payload = PointerEventArgs {
                        position: event.position,
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
                self.finish_press(button, event.position, event.modifiers, event.timestamp_ms);
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
        position: Point,
    ) {
        let mut press = self.press.borrow_mut();
        match press.as_mut() {
            Some(existing) => {
                existing.held_buttons.insert(button);
            }
            None => {
                if let Some(target) = target {
                    let mut held_buttons = HashSet::new();
                    held_buttons.insert(button);
                    *press = Some(PressState {
                        target,
                        initiating_button: button,
                        start_position: position,
                        held_buttons,
                    });
                }
            }
        }
    }

    fn finish_press(
        &self,
        button: MouseButton,
        release_position: Point,
        modifiers: KeyModifiers,
        timestamp_ms: f64,
    ) {
        let mut press_slot = self.press.borrow_mut();
        let Some(press) = press_slot.as_mut() else {
            return;
        };
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
        if distance(release_position, start_position) > TAP_MOVE_THRESHOLD_PX {
            // A real drag, not a tap — also cancels any pending double-tap streak.
            *self.last_tap.borrow_mut() = None;
            return;
        }
        let tap_event_name = match button {
            MouseButton::Left => "on_tapped",
            MouseButton::Right => "on_right_tapped",
            // WinUI3 has no middle-button tap gesture.
            MouseButton::Middle => return,
        };
        let tapped_payload = TappedEventArgs {
            position: release_position,
            modifiers,
        };
        crate::ui::dispatch_routed(
            &press_target,
            tap_event_name,
            &tapped_payload,
            &RoutedEventArgs::default(),
        );

        let mut last_tap = self.last_tap.borrow_mut();
        let is_double = last_tap.as_ref().is_some_and(|prev| {
            prev.button == button
                && Rc::ptr_eq(&prev.target, &press_target)
                && (timestamp_ms - prev.at_ms).abs() <= DOUBLE_TAP_INTERVAL_MS
                && distance(release_position, prev.position) <= DOUBLE_TAP_DISTANCE_PX
        });
        if is_double {
            crate::ui::dispatch_routed(
                &press_target,
                "on_double_tapped",
                &tapped_payload,
                &RoutedEventArgs::default(),
            );
            *last_tap = None;
        } else {
            *last_tap = Some(TapRecord {
                target: press_target,
                button,
                position: release_position,
                at_ms: timestamp_ms,
            });
        }
    }

    /// Fires `on_pointer_exited`/`on_pointer_entered` (non-bubbling per element —
    /// `elwindui_core::ui::dispatch_direct` — see this type's own doc comment) for every element
    /// whose hover state actually changed, by diffing the previous and current ancestor chains.
    /// An element present in both chains (a still-hovered common ancestor) gets neither call.
    fn update_hover(
        &self,
        new_hit: Option<Rc<dyn UIElementExt>>,
        position: Point,
        modifiers: KeyModifiers,
    ) {
        let new_chain = ancestor_chain(new_hit);
        let old_chain = self.last_hover.replace(new_chain.clone());
        let payload = PointerEventArgs {
            position,
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
