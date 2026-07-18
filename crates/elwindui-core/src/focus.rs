use crate::input::{FocusState, RoutedEventArgs};
use crate::ui::UIElementExt;
use std::cell::RefCell;
use std::rc::Rc;

/// See docs/elwindui_gui_framework_design.md §5.5. `Up`/`Down`/`Left`/`Right` are declared for API
/// completeness but not yet implemented by `FocusTracker::move_focus` — 2D spatial navigation needs
/// each tab stop's own arranged rect, which isn't threaded through here yet. Only `Next`/`Previous`
/// (`Tab`/`Shift+Tab`) are wired, via `KeyboardDispatcher::handle_key`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusDirection {
    Next,
    Previous,
    Up,
    Down,
    Left,
    Right,
}

/// `scope`'s own subtree (`scope` included), depth-first over `visual_children()`, filtered to
/// `is_tab_stop()` elements only, ordered by `(focus_order().unwrap_or(i32::MAX), encounter order)`
/// — WinUI3's tab order, without needing any element to declare `#[focus(order: ..)]` at all (tree
/// order alone is a legitimate default, matching WinUI3's own "unset TabIndex falls back to
/// visual/declaration order" behavior).
fn tab_order(scope: &Rc<dyn UIElementExt>) -> Vec<Rc<dyn UIElementExt>> {
    fn walk(elem: &Rc<dyn UIElementExt>, out: &mut Vec<Rc<dyn UIElementExt>>) {
        if elem.is_tab_stop() {
            out.push(Rc::clone(elem));
        }
        for child in elem.visual_children() {
            walk(&child, out);
        }
    }
    let mut out = Vec::new();
    walk(scope, &mut out);
    out.sort_by_key(|e| e.focus_order().unwrap_or(i32::MAX));
    out
}

/// Tracks which element (if any) currently has keyboard focus within a hosted tree — the
/// `Rc<dyn UIElementExt>`-based counterpart to `elwindui_core::input::PointerDispatcher`, and the
/// concrete runtime backing for `docs/elwindui_gui_framework_design.md` §5.5. Owned by
/// `KeyboardDispatcher` (one per hosted tree), the same "host owns exactly one instance" pattern
/// `PointerDispatcher` already uses. Deliberately not keyed by an `ElementId` string — see
/// docs/elwindui_gui_framework_design.md §5.2's note that a string-id-based `find_by_id` is
/// intentionally not provided, so an id-keyed API would have no way to resolve back to a real tree
/// node in the first place.
#[derive(Default)]
pub struct FocusTracker {
    focused: RefCell<Option<Rc<dyn UIElementExt>>>,
    /// Innermost (most recently pushed) scope last — `Dialog`-style focus traps push their own
    /// root here (`docs/elwindui_builtins_spec.md` 付録M, not yet implemented) so `tab_order`/
    /// `move_focus` stay confined to that subtree until popped. Unused until some builtin actually
    /// calls `push_trap`.
    trap_stack: RefCell<Vec<Rc<dyn UIElementExt>>>,
}

impl FocusTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn focused(&self) -> Option<Rc<dyn UIElementExt>> {
        self.focused.borrow().clone()
    }

    /// Moves focus to `target`. Returns `false` (and does nothing) if `target` isn't a tab stop
    /// (`UIElementExt::is_tab_stop`) — matching WinUI3's `Control.Focus` returning `false` for a
    /// non-focusable control. On success, fires `on_lost_focus` (non-bubbling, `dispatch_direct`)
    /// at the previously-focused element (if any) with its own `focus_state` reset to `Unfocused`
    /// first, then `on_got_focus` at `target` with its `focus_state` set to `state` first — matching
    /// real `GotFocus`/`LostFocus`, where the property is already updated by the time the handler
    /// observes it. A no-op (returns `true`) if `target` is already the focused element.
    pub fn set_focus(&self, target: &Rc<dyn UIElementExt>, state: FocusState) -> bool {
        if !target.is_tab_stop() {
            return false;
        }
        if self
            .focused
            .borrow()
            .as_ref()
            .is_some_and(|f| Rc::ptr_eq(f, target))
        {
            return true;
        }
        let previous = self.focused.borrow_mut().take();
        if let Some(previous) = &previous {
            previous.set_focus_state(FocusState::Unfocused);
            crate::ui::dispatch_direct(
                previous,
                "on_lost_focus",
                &(),
                &RoutedEventArgs::default(),
            );
        }
        target.set_focus_state(state);
        *self.focused.borrow_mut() = Some(Rc::clone(target));
        crate::ui::dispatch_direct(target, "on_got_focus", &(), &RoutedEventArgs::default());
        true
    }

    /// Unfocuses the current element (if any), firing `on_lost_focus` the same way `set_focus`
    /// does when replacing a previous focus.
    pub fn clear_focus(&self) {
        let previous = self.focused.borrow_mut().take();
        if let Some(previous) = &previous {
            previous.set_focus_state(FocusState::Unfocused);
            crate::ui::dispatch_direct(
                previous,
                "on_lost_focus",
                &(),
                &RoutedEventArgs::default(),
            );
        }
    }

    pub fn push_trap(&self, scope: Rc<dyn UIElementExt>) {
        self.trap_stack.borrow_mut().push(scope);
    }

    pub fn pop_trap(&self) {
        self.trap_stack.borrow_mut().pop();
    }

    /// Computes `tab_order()` over the current trap scope (if any) or `root`, and moves focus to
    /// the next/previous entry relative to the currently-focused element — wrapping around at
    /// either end. If nothing is currently focused, moves to the first (`Next`) or last
    /// (`Previous`) tab stop. `Up`/`Down`/`Left`/`Right` are not yet implemented (see
    /// `FocusDirection`'s own doc comment) and always return `false`. Always focuses with
    /// `FocusState::Keyboard`. Returns `false` if there is no tab stop to move to.
    pub fn move_focus(&self, root: &Rc<dyn UIElementExt>, direction: FocusDirection) -> bool {
        let scope = self
            .trap_stack
            .borrow()
            .last()
            .cloned()
            .unwrap_or_else(|| Rc::clone(root));
        let order = tab_order(&scope);
        if order.is_empty() {
            return false;
        }
        let current = self.focused.borrow().clone();
        let next = match direction {
            FocusDirection::Next => {
                let index = current
                    .as_ref()
                    .and_then(|c| order.iter().position(|e| Rc::ptr_eq(e, c)));
                match index {
                    Some(i) => &order[(i + 1) % order.len()],
                    None => &order[0],
                }
            }
            FocusDirection::Previous => {
                let index = current
                    .as_ref()
                    .and_then(|c| order.iter().position(|e| Rc::ptr_eq(e, c)));
                match index {
                    Some(i) => &order[(i + order.len() - 1) % order.len()],
                    None => order.last().expect("checked non-empty above"),
                }
            }
            FocusDirection::Up | FocusDirection::Down | FocusDirection::Left
            | FocusDirection::Right => return false,
        };
        self.set_focus(next, FocusState::Keyboard)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{LayoutExt, VerticalLayout};

    fn tab_stop() -> Rc<VerticalLayout> {
        let node = VerticalLayout::new();
        node.set_tab_stop(true);
        node
    }

    #[test]
    fn set_focus_then_focused_round_trips() {
        let tracker = FocusTracker::new();
        let target: Rc<dyn UIElementExt> = tab_stop();
        assert!(tracker.set_focus(&target, FocusState::Programmatic));
        let focused = tracker.focused().expect("should be focused");
        assert!(Rc::ptr_eq(&focused, &target));
        assert_eq!(target.focus_state(), FocusState::Programmatic);
    }

    #[test]
    fn set_focus_rejects_non_tab_stop() {
        let tracker = FocusTracker::new();
        let target: Rc<dyn UIElementExt> = VerticalLayout::new();
        assert!(!tracker.set_focus(&target, FocusState::Programmatic));
        assert!(tracker.focused().is_none());
    }

    #[test]
    fn move_focus_next_cycles_and_wraps() {
        let root = VerticalLayout::new();
        let a = tab_stop();
        let b = tab_stop();
        root.children().add(a.clone());
        root.children().add(b.clone());
        let root: Rc<dyn UIElementExt> = root;

        let tracker = FocusTracker::new();
        assert!(tracker.move_focus(&root, FocusDirection::Next));
        let a_dyn: Rc<dyn UIElementExt> = a.clone();
        assert!(Rc::ptr_eq(&tracker.focused().unwrap(), &a_dyn));

        assert!(tracker.move_focus(&root, FocusDirection::Next));
        let b_dyn: Rc<dyn UIElementExt> = b.clone();
        assert!(Rc::ptr_eq(&tracker.focused().unwrap(), &b_dyn));

        assert!(tracker.move_focus(&root, FocusDirection::Next));
        assert!(Rc::ptr_eq(&tracker.focused().unwrap(), &a_dyn));
    }
}
