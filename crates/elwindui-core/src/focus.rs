/// Corresponds 1:1 with the string literal passed to `#[id("...")]`. See docs/elwindui_spec.md §13.
pub type ElementId = String;

/// See docs/elwindui_spec.md 付録H.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusDirection {
    Next,
    Previous,
    Up,
    Down,
    Left,
    Right,
}

/// See docs/elwindui_spec.md 付録H.3. Native backends mirror results into the OS focus chain
/// (WinUI3 `FocusManager`, AppKit `NSResponder` chain, GTK4 `gtk_widget_grab_focus`), with this
/// trait's implementation as the source of truth.
pub trait FocusManager {
    fn move_focus(&mut self, direction: FocusDirection) -> Option<ElementId>;
    fn set_focus(&mut self, id: ElementId);
    fn focused(&self) -> Option<ElementId>;
    fn trap_focus(&mut self, scope: ElementId);
}

#[cfg(test)]
mod tests {
    use super::*;

    struct SingleFocus {
        current: Option<ElementId>,
    }

    impl FocusManager for SingleFocus {
        fn move_focus(&mut self, _direction: FocusDirection) -> Option<ElementId> {
            self.current.clone()
        }

        fn set_focus(&mut self, id: ElementId) {
            self.current = Some(id);
        }

        fn focused(&self) -> Option<ElementId> {
            self.current.clone()
        }

        fn trap_focus(&mut self, scope: ElementId) {
            self.current = Some(scope);
        }
    }

    #[test]
    fn set_focus_then_focused_round_trips() {
        let mut manager = SingleFocus { current: None };
        manager.set_focus("username".to_string());
        assert_eq!(manager.focused(), Some("username".to_string()));
    }
}
