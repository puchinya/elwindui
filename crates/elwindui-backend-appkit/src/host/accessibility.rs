//! Synthetic AppKit accessibility projection for a `TreeHostView`.
//!
//! The objects in this module are deliberately thin. They retain only a stable Core
//! `AccessibilityId` and a weak host route; all public semantics, geometry, children, and action
//! decisions come from the host's immutable `AccessibilitySnapshot`.

use super::TreeHostView;
use elwindui_core::accessibility::{
    AccessibilityAction, AccessibilityHost, AccessibilityId, AccessibilityRole,
    AccessibilitySnapshot, AccessibilitySnapshotNode,
};
use elwindui_core::base::{Point, Rect};
use objc2::rc::{Retained, Weak};
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSAccessibilityElement, NSAccessibilityRole, NSScreen};
use objc2_foundation::{
    NSArray, NSNumber, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSValue,
};

pub(crate) struct AppKitAccessibilityHost(pub(crate) Weak<TreeHostView>);

impl AccessibilityHost for AppKitAccessibilityHost {
    fn request_accessibility_update(&self) {
        if let Some(host) = self.0.load() {
            host.rebuild_accessibility();
        }
    }
}

pub(crate) struct SyntheticAccessibilityElementIvars {
    pub(crate) id: AccessibilityId,
    pub(crate) host: Weak<TreeHostView>,
}

define_class!(
    #[unsafe(super(NSAccessibilityElement))]
    #[thread_kind = objc2::MainThreadOnly]
    #[ivars = SyntheticAccessibilityElementIvars]
    pub(crate) struct SyntheticAccessibilityElement;

    unsafe impl NSObjectProtocol for SyntheticAccessibilityElement {}

    impl SyntheticAccessibilityElement {
        #[unsafe(method_id(accessibilityChildren))]
        fn accessibility_children(&self) -> Option<Retained<NSArray>> {
            self.ivars()
                .host
                .load()
                .and_then(|host| children_for_host(&host, Some(self.ivars().id)))
        }

        #[unsafe(method_id(accessibilityVisibleChildren))]
        fn accessibility_visible_children(&self) -> Option<Retained<NSArray>> {
            self.ivars()
                .host
                .load()
                .and_then(|host| children_for_host(&host, Some(self.ivars().id)))
        }

        #[unsafe(method(accessibilityFrame))]
        fn accessibility_frame(&self) -> NSRect {
            self.ivars()
                .host
                .load()
                .map(|host| host.accessibility_frame_for(self.ivars().id))
                .unwrap_or_default()
        }

        #[unsafe(method_id(accessibilityParent))]
        fn accessibility_parent(&self) -> Option<Retained<AnyObject>> {
            self.ivars()
                .host
                .load()
                .and_then(|host| host.accessibility_parent_for(self.ivars().id))
        }

        #[unsafe(method(isAccessibilityFocused))]
        fn is_accessibility_focused(&self) -> bool {
            current_node(self)
                .is_some_and(|node| node.semantics.state.focused)
        }

        #[unsafe(method_id(accessibilityIdentifier))]
        fn accessibility_identifier(&self) -> Retained<NSString> {
            current_node(self)
                .and_then(|node| node.semantics.identifier.clone())
                .map(|value| NSString::from_str(&value))
                .unwrap_or_else(|| NSString::from_str(&self.ivars().id.raw().to_string()))
        }

        #[unsafe(method_id(accessibilityRole))]
        fn accessibility_role(&self) -> Option<Retained<NSAccessibilityRole>> {
            match current_node(self) {
                Some(node) => Some(NSString::from_str(role_name(node.semantics.role))),
                None => None,
            }
        }

        #[unsafe(method_id(accessibilityLabel))]
        fn accessibility_label(&self) -> Option<Retained<NSString>> {
            current_node(self).and_then(|node| node.semantics.label.clone().map(|value| NSString::from_str(&value)))
        }

        #[unsafe(method_id(accessibilityHelp))]
        fn accessibility_help(&self) -> Option<Retained<NSString>> {
            current_node(self).and_then(|node| node.semantics.hint.clone().map(|value| NSString::from_str(&value)))
        }

        #[unsafe(method_id(accessibilityValue))]
        fn accessibility_value(&self) -> Option<Retained<AnyObject>> {
            match current_node(self) {
                Some(node) => {
                    if let Some(range) = node.semantics.state.value_range {
                        Some(number_as_any(NSNumber::new_f64(range.value)))
                    } else if let Some(checked) = node.semantics.state.checked {
                        let value = match checked {
                            elwindui_core::accessibility::AccessibilityCheckState::Off => 0.0,
                            elwindui_core::accessibility::AccessibilityCheckState::On => 1.0,
                            elwindui_core::accessibility::AccessibilityCheckState::Mixed => 2.0,
                        };
                        Some(number_as_any(NSNumber::new_f64(value)))
                    } else {
                        node.semantics.value.clone().map(string_as_any)
                    }
                }
                None => None,
            }
        }

        #[unsafe(method_id(accessibilityMinValue))]
        fn accessibility_min_value(&self) -> Option<Retained<AnyObject>> {
            current_node(self)
                .and_then(|node| node.semantics.state.value_range)
                .map(|range| number_as_any(NSNumber::new_f64(range.min)))
        }

        #[unsafe(method_id(accessibilityMaxValue))]
        fn accessibility_max_value(&self) -> Option<Retained<AnyObject>> {
            current_node(self)
                .and_then(|node| node.semantics.state.value_range)
                .map(|range| number_as_any(NSNumber::new_f64(range.max)))
        }

        #[unsafe(method_id(accessibilityValueIncrement))]
        fn accessibility_value_increment(&self) -> Option<Retained<AnyObject>> {
            current_node(self)
                .and_then(|node| node.semantics.state.value_range)
                .and_then(|range| range.step)
                .map(|step| number_as_any(NSNumber::new_f64(step)))
        }

        #[unsafe(method(isAccessibilityEnabled))]
        fn is_accessibility_enabled(&self) -> bool {
            current_node(self).is_some_and(|node| !node.semantics.state.disabled)
        }

        #[unsafe(method(setAccessibilityFocused:))]
        fn set_accessibility_focused(&self, focused: bool) {
            if focused {
                if let Some(host) = self.ivars().host.load() {
                    host.ivars()
                        .accessibility_runtime
                        .dispatch_action(self.ivars().id, AccessibilityAction::Focus);
                }
            }
        }

        #[unsafe(method(accessibilityPerformPress))]
        fn accessibility_perform_press(&self) -> bool {
            self.dispatch(AccessibilityAction::Activate)
        }

        #[unsafe(method(accessibilityPerformIncrement))]
        fn accessibility_perform_increment(&self) -> bool {
            self.dispatch(AccessibilityAction::Increment)
        }

        #[unsafe(method(accessibilityPerformDecrement))]
        fn accessibility_perform_decrement(&self) -> bool {
            self.dispatch(AccessibilityAction::Decrement)
        }

        #[unsafe(method(setAccessibilityValue:))]
        fn set_accessibility_value(&self, value: Option<&AnyObject>) {
            let Some(host) = self.ivars().host.load() else { return };
            let Some(value) = value else { return };
            if let Some(number) = value.downcast_ref::<NSNumber>() {
                host.ivars().accessibility_runtime.dispatch_action(
                    self.ivars().id,
                    AccessibilityAction::SetValue(number.as_f64()),
                );
            } else if let Some(string) = value.downcast_ref::<NSString>() {
                host.ivars().accessibility_runtime.dispatch_action(
                    self.ivars().id,
                    AccessibilityAction::SetText(string.to_string()),
                );
            }
        }
    }
);

impl SyntheticAccessibilityElement {
    pub(crate) fn new(id: AccessibilityId, host: Weak<TreeHostView>) -> Retained<Self> {
        let this = <Self as MainThreadOnly>::alloc(super::mtm())
            .set_ivars(SyntheticAccessibilityElementIvars { id, host });
        unsafe { msg_send![super(this), init] }
    }

    fn dispatch(&self, action: AccessibilityAction) -> bool {
        self.ivars().host.load().is_some_and(|host| {
            host.ivars()
                .accessibility_runtime
                .dispatch_action(self.ivars().id, action)
        })
    }
}

fn number_as_any(number: Retained<NSNumber>) -> Retained<AnyObject> {
    let number: Retained<NSValue> = Retained::into_super(number);
    let number: Retained<NSObject> = Retained::into_super(number);
    Retained::into_super(number)
}

fn string_as_any(value: String) -> Retained<AnyObject> {
    let value: Retained<NSString> = NSString::from_str(&value);
    let value: Retained<NSObject> = Retained::into_super(value);
    Retained::into_super(value)
}

fn element_as_any(element: Retained<SyntheticAccessibilityElement>) -> Retained<AnyObject> {
    let element: Retained<NSAccessibilityElement> = Retained::into_super(element);
    let element: Retained<NSObject> = Retained::into_super(element);
    Retained::into_super(element)
}

fn current_node(element: &SyntheticAccessibilityElement) -> Option<AccessibilitySnapshotNode> {
    let host = element.ivars().host.load()?;
    let snapshot = host.ivars().accessibility_runtime.snapshot();
    find_node(&snapshot, element.ivars().id).cloned()
}

fn role_name(role: AccessibilityRole) -> &'static str {
    match role {
        AccessibilityRole::Button => "AXButton",
        AccessibilityRole::StaticText => "AXStaticText",
        AccessibilityRole::TextInput | AccessibilityRole::SecureTextInput => "AXTextField",
        AccessibilityRole::CheckBox => "AXCheckBox",
        AccessibilityRole::RadioButton => "AXRadioButton",
        AccessibilityRole::Switch => "AXSwitch",
        AccessibilityRole::Slider => "AXSlider",
        AccessibilityRole::ComboBox => "AXComboBox",
        AccessibilityRole::Group => "AXGroup",
    }
}

fn find_node<'a>(
    snapshot: &'a AccessibilitySnapshot,
    id: AccessibilityId,
) -> Option<&'a AccessibilitySnapshotNode> {
    snapshot
        .roots
        .iter()
        .find_map(|node| find_node_in(node, id))
}

fn find_node_in<'a>(
    node: &'a AccessibilitySnapshotNode,
    id: AccessibilityId,
) -> Option<&'a AccessibilitySnapshotNode> {
    if node.id == id {
        return Some(node);
    }
    node.children
        .iter()
        .find_map(|child| find_node_in(child, id))
}

fn child_ids(
    snapshot: &AccessibilitySnapshot,
    parent: Option<AccessibilityId>,
) -> Vec<AccessibilityId> {
    match parent {
        None => snapshot.roots.iter().map(|node| node.id).collect(),
        Some(id) => find_node(snapshot, id)
            .map(|node| node.children.iter().map(|child| child.id).collect())
            .unwrap_or_default(),
    }
}

pub(crate) fn children_for_host(
    host: &TreeHostView,
    parent: Option<AccessibilityId>,
) -> Option<Retained<NSArray>> {
    let snapshot = host.ivars().accessibility_runtime.snapshot();
    let children = child_ids(&snapshot, parent)
        .into_iter()
        .map(|id| host.accessibility_element_for(id))
        .map(element_as_any)
        .collect::<Vec<Retained<AnyObject>>>();
    Some(NSArray::from_retained_slice(&children))
}

pub(crate) fn hit_test_host(host: &TreeHostView, point: NSPoint) -> Option<Retained<AnyObject>> {
    let primary_height = NSScreen::screens(super::mtm())
        .firstObject()
        .or_else(|| NSScreen::mainScreen(super::mtm()))
        .map(|screen| screen.frame().size.height)?;
    let root_point =
        host.screen_to_root_point(super::appkit_screen_to_core(point, primary_height))?;
    let snapshot = host.ivars().accessibility_runtime.snapshot();
    let id = hit_test_snapshot(&snapshot, root_point)?;
    Some(element_as_any(host.accessibility_element_for(id)))
}

fn hit_test_snapshot(snapshot: &AccessibilitySnapshot, point: Point) -> Option<AccessibilityId> {
    fn visit(node: &AccessibilitySnapshotNode, point: Point) -> Option<AccessibilityId> {
        if !contains(node.bounds_in_root, point) {
            return None;
        }
        node.children
            .iter()
            .rev()
            .find_map(|child| visit(child, point))
            .or(Some(node.id))
    }
    snapshot
        .roots
        .iter()
        .rev()
        .find_map(|node| visit(node, point))
}

fn contains(rect: Rect, point: Point) -> bool {
    point.x >= rect.x
        && point.x <= rect.x + rect.width
        && point.y >= rect.y
        && point.y <= rect.y + rect.height
}

#[cfg(test)]
mod tests {
    use super::*;
    use elwindui_core::accessibility::{AccessibilitySemantics, AccessibilityState};

    fn node(id: u64, role: AccessibilityRole, bounds: Rect) -> AccessibilitySnapshotNode {
        AccessibilitySnapshotNode {
            id: AccessibilityId::from_raw(id),
            semantics: AccessibilitySemantics {
                role,
                label: None,
                value: None,
                hint: None,
                identifier: None,
                state: AccessibilityState::default(),
                child_behavior: elwindui_core::accessibility::AccessibilityChildBehavior::Automatic,
                actions: Vec::new(),
            },
            bounds_in_root: bounds,
            children: Vec::new(),
        }
    }

    #[test]
    fn role_mapping_uses_synthetic_ax_roles() {
        assert_eq!(role_name(AccessibilityRole::Button), "AXButton");
        assert_eq!(role_name(AccessibilityRole::SecureTextInput), "AXTextField");
        assert_eq!(role_name(AccessibilityRole::Slider), "AXSlider");
    }

    #[test]
    fn hit_testing_prefers_deepest_semantic_child_in_navigation_order() {
        let mut parent = node(
            1,
            AccessibilityRole::Group,
            Rect {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
            },
        );
        parent.children.push(node(
            2,
            AccessibilityRole::Button,
            Rect {
                x: 10.0,
                y: 10.0,
                width: 20.0,
                height: 20.0,
            },
        ));
        let snapshot = AccessibilitySnapshot {
            roots: vec![parent],
        };
        assert_eq!(
            hit_test_snapshot(&snapshot, Point { x: 15.0, y: 15.0 }),
            Some(AccessibilityId::from_raw(2))
        );
        assert_eq!(
            hit_test_snapshot(&snapshot, Point { x: 80.0, y: 80.0 }),
            Some(AccessibilityId::from_raw(1))
        );
    }
}

impl TreeHostView {
    pub(crate) fn accessibility_element_for(
        &self,
        id: AccessibilityId,
    ) -> Retained<SyntheticAccessibilityElement> {
        if let Some(element) = self.ivars().accessibility_elements.borrow().get(&id) {
            return element.clone();
        }
        let element =
            SyntheticAccessibilityElement::new(id, self.ivars().weak_self.borrow().clone());
        self.ivars()
            .accessibility_elements
            .borrow_mut()
            .insert(id, element.clone());
        element
    }

    pub(crate) fn accessibility_parent_for(
        &self,
        id: AccessibilityId,
    ) -> Option<Retained<AnyObject>> {
        let snapshot = self.ivars().accessibility_runtime.snapshot();
        if snapshot.roots.iter().any(|node| node.id == id) {
            let host_view = self.ivars().weak_self.borrow().load()?;
            let host_view: Retained<objc2_app_kit::NSView> = Retained::into_super(host_view);
            let host_view: Retained<objc2_app_kit::NSResponder> = Retained::into_super(host_view);
            let host_view: Retained<NSObject> = Retained::into_super(host_view);
            return Some(Retained::into_super(host_view));
        }
        let parent = find_parent(&snapshot, id)?;
        Some(element_as_any(self.accessibility_element_for(parent)))
    }

    pub(crate) fn accessibility_frame_for(&self, id: AccessibilityId) -> NSRect {
        let snapshot = self.ivars().accessibility_runtime.snapshot();
        let Some(node) = find_node(&snapshot, id) else {
            return NSRect::default();
        };
        let Some(primary_height) = NSScreen::screens(super::mtm())
            .firstObject()
            .or_else(|| NSScreen::mainScreen(super::mtm()))
            .map(|screen| screen.frame().size.height)
        else {
            return NSRect::default();
        };
        let bounds = node.bounds_in_root;
        let corners = [
            Point {
                x: bounds.x,
                y: bounds.y,
            },
            Point {
                x: bounds.x + bounds.width,
                y: bounds.y,
            },
            Point {
                x: bounds.x,
                y: bounds.y + bounds.height,
            },
            Point {
                x: bounds.x + bounds.width,
                y: bounds.y + bounds.height,
            },
        ];
        let points = corners
            .into_iter()
            .filter_map(|point| self.root_to_screen_point(point))
            .map(|point| super::core_screen_to_appkit(point, primary_height))
            .collect::<Vec<_>>();
        if points.len() != 4 {
            return NSRect::default();
        }
        let min_x = points
            .iter()
            .map(|point| point.x)
            .fold(f64::INFINITY, f64::min);
        let max_x = points
            .iter()
            .map(|point| point.x)
            .fold(f64::NEG_INFINITY, f64::max);
        let min_y = points
            .iter()
            .map(|point| point.y)
            .fold(f64::INFINITY, f64::min);
        let max_y = points
            .iter()
            .map(|point| point.y)
            .fold(f64::NEG_INFINITY, f64::max);
        NSRect::new(
            NSPoint::new(min_x, min_y),
            NSSize::new(max_x - min_x, max_y - min_y),
        )
    }
}

fn find_parent(snapshot: &AccessibilitySnapshot, id: AccessibilityId) -> Option<AccessibilityId> {
    fn visit(node: &AccessibilitySnapshotNode, id: AccessibilityId) -> Option<AccessibilityId> {
        if node.children.iter().any(|child| child.id == id) {
            return Some(node.id);
        }
        node.children.iter().find_map(|child| visit(child, id))
    }
    snapshot.roots.iter().find_map(|node| visit(node, id))
}
