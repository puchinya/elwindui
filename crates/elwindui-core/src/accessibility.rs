/// See docs/elwindui_spec.md 付録H.4.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessibilityRole {
    ButtonExt,
    TextInput,
    CheckBox,
    Slider,
    StaticText,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ValueRange {
    pub value: f64,
    pub min: f64,
    pub max: f64,
}

/// Modeled on WinUI3's AutomationPeer, which exposes different UIA providers (IToggleProvider,
/// IRangeValueProvider, IExpandCollapseProvider, ...) depending on control kind: unsupported
/// providers stay `None` rather than every role implementing every field.
#[derive(Debug, Clone, PartialEq)]
pub struct AccessibilityState {
    pub disabled: bool,
    pub focused: bool,
    pub toggled: Option<bool>,
    pub expanded: Option<bool>,
    pub selected: Option<bool>,
    pub value_range: Option<ValueRange>,
}

impl Default for AccessibilityState {
    fn default() -> Self {
        AccessibilityState {
            disabled: false,
            focused: false,
            toggled: None,
            expanded: None,
            selected: None,
            value_range: None,
        }
    }
}

/// See docs/elwindui_spec.md 付録H.4.
pub trait AccessibilityNode {
    fn role(&self) -> AccessibilityRole;
    fn label(&self) -> String;
    fn state(&self) -> AccessibilityState;
    fn children(&self) -> Vec<&dyn AccessibilityNode>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state_has_no_providers() {
        let state = AccessibilityState::default();
        assert!(!state.disabled);
        assert_eq!(state.toggled, None);
        assert_eq!(state.value_range, None);
    }
}
