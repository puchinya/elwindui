//! `elwindui::ui::RadioButton` — a native mutually-exclusive selection button.

use super::*;
use crate::ui::native::{NativePeer, RadioButtonPeer, create_native_peer};
use std::cell::OnceCell;
use std::collections::HashMap;

/// `elwindui::ui::RadioButton` — a native mutually-exclusive selection button (AppKit: `NSButton` with
/// `NSButtonType::Radio`; WinUI3: `RadioButton`).
///
/// **Grouping is managed by elwindui, not by either native toolkit.** AppKit's own automatic
/// radio grouping only applies to buttons that share both a superview *and* an action selector —
/// a condition this framework's per-instance click trampoline never satisfies, so nothing needs
/// suppressing there. Instead, every `RadioButton` sharing the same non-empty `group` string is
/// tracked together (by Core, in one per-UI-thread registry), and
/// checking one un-checks every other member of its group. A `RadioButton` with no `group` (the
/// default) participates in no exclusivity at all — it behaves like a plain two-state toggle.
///
/// Declared once here as a Core-owned Component (Issue #313). Core owns the group registry, so
/// exclusivity is identical on every backend; the [`RadioButtonPeer`] only displays the state and
/// reports user activation.
#[elwindui_macros::component(inherits crate::ui::NativeControl, sealed)]
#[prop(text: String)]
#[prop(two_way, checked: bool)]
#[prop(group: Option<String>)]
#[prop(enabled: Option<bool>)]
pub struct RadioButton {
    text: RefCell<String>,
    checked: Cell<bool>,
    group: RefCell<String>,
    enabled: Cell<bool>,
    on_change: RefCell<Option<Rc<dyn Fn(bool)>>>,
    peer: OnceCell<Rc<dyn RadioButtonPeer>>,
}

thread_local! {
    /// Every grouped `RadioButton` on this UI thread, by group name. Entries are `Weak`, so the
    /// registry never keeps a button alive; dead entries are pruned on registration.
    static GROUPS: RefCell<HashMap<String, Vec<Weak<dyn UIElementExt>>>> =
        RefCell::new(HashMap::new());
}

#[elwindui_macros::component]
impl RadioButton {
    #[overrides]
    fn perform_accessibility_action(&self, action: AccessibilityAction) -> bool {
        match action {
            AccessibilityAction::Activate | AccessibilityAction::Select => {
                self.select(true);
                true
            }
            AccessibilityAction::Focus => self.focus(),
            _ => false,
        }
    }

    /// Realization phase 1: create only (see [`NativeControl`]).
    #[overrides]
    fn __create_native_peer(&self) -> Option<Rc<dyn NativePeer>> {
        let peer = create_native_peer("RadioButton", |factory| factory.create_radio_button());
        if self.peer.set(peer.clone()).is_err() {
            panic!("elwindui: a RadioButton's native peer was realized more than once");
        }
        Some(peer)
    }

    /// Realization phase 3: push the logical state, then install the click callback last.
    #[overrides]
    fn __initialize_native_peer(&self) {
        let peer = self.peer.get().expect("created in phase 1").clone();
        let text = self.text.borrow().clone();
        peer.set_text(&text);
        peer.set_checked(self.checked.get());
        peer.set_enabled(self.enabled.get());
        let on_click = weak_native_callback(self.as_ui_element(), |this: &RadioButton, ()| {
            this.select(false);
        });
        peer.set_on_click(Some(Box::new(move || on_click(()))));
    }

    fn set_text(&self, text: &str) {
        *self.text.borrow_mut() = text.to_string();
        if let Some(peer) = self.peer.get() {
            peer.set_text(text);
        }
        let latest = self.text.borrow().clone();
        update_intrinsic_semantics(self.as_ui_element(), |semantics| {
            semantics.label = Some(latest);
        });
        // Some toolkits drop the attributed title when the caption changes (AppKit `NSButton`),
        // and the caption changes the fitting size.
        self.base.reapply_text_style();
        self.base.invalidate_measure();
    }

    /// Programmatic: updates the display (and unchecks the rest of the group when `true`)
    /// without notifying this button's `on_change`.
    fn set_checked(&self, checked: bool) {
        self.apply_checked(checked);
        if checked {
            self.uncheck_siblings();
        }
    }

    fn set_on_change(&self, callback: Box<dyn Fn(bool)>) {
        *self.on_change.borrow_mut() = Some(Rc::from(callback));
    }

    /// Moves this button to `group` (or out of every group for `""`). The registration in the
    /// previous group is removed, so the old group no longer reaches this button, and a button
    /// is never registered twice. A checked button that joins a group keeps its check and
    /// unchecks the group's other checked member, exactly like `set_checked(true)`, so a group
    /// never shows two checked buttons.
    fn set_group(&self, group: &str) {
        let previous = self.group.replace(group.to_string());
        if previous == group {
            return;
        }
        let Some(owner) = self.as_ui_element().visual_collection.owner_rc() else {
            return;
        };
        let this = Rc::downgrade(&owner);
        GROUPS.with(|groups| {
            let mut groups = groups.borrow_mut();
            if let Some(members) = groups.get_mut(&previous) {
                retain_other_live_members(members, &this);
                if members.is_empty() {
                    groups.remove(&previous);
                }
            }
            if !group.is_empty() {
                let members = groups.entry(group.to_string()).or_default();
                retain_other_live_members(members, &this);
                members.push(this.clone());
            }
        });
        if self.checked.get() {
            self.uncheck_siblings();
        }
    }

    fn set_enabled(&self, enabled: bool) {
        self.enabled.set(enabled);
        if let Some(peer) = self.peer.get() {
            peer.set_enabled(enabled);
        }
        sync_native_enabled(self.as_ui_element(), self.enabled.get());
    }

    /// `codegen` calls exactly `set_on_{field}_change` for a `#[two_way]` prop.
    #[inherent]
    pub fn set_on_checked_change(&self, callback: Box<dyn Fn(bool)>) {
        self.set_on_change(callback);
    }

    fn construct() -> Self {
        Self {
            base: NativeControl::construct(None),
            text: RefCell::new(String::new()),
            checked: Cell::new(false),
            group: RefCell::new(String::new()),
            enabled: Cell::new(true),
            on_change: RefCell::new(None),
            peer: OnceCell::new(),
        }
    }

    fn on_constructed(&self) {
        let mut semantics = native_semantics(
            AccessibilityRole::RadioButton,
            &[
                AccessibilityActionKind::Activate,
                AccessibilityActionKind::Select,
                AccessibilityActionKind::Focus,
            ],
        );
        semantics.state.checked = Some(AccessibilityCheckState::Off);
        semantics.state.selected = Some(false);
        self.base.set_intrinsic_accessibility_semantics(semantics);
        self.set_tab_stop(true);
    }
}

/// Drops dead registrations and any registration of `this` from one group's member list.
fn retain_other_live_members(
    members: &mut Vec<Weak<dyn UIElementExt>>,
    this: &Weak<dyn UIElementExt>,
) {
    members.retain(|member: &Weak<dyn UIElementExt>| {
        Weak::<dyn UIElementExt>::strong_count(member) > 0
            && !Weak::<dyn UIElementExt>::ptr_eq(member, this)
    });
}

impl RadioButton {
    /// A user selection: from the native click (`push == false`, the widget already shows it)
    /// or an accessibility action (`push == true`). Checks this button, unchecks the rest of the
    /// group, and notifies every button whose state actually changed, once each.
    fn select(&self, push: bool) {
        let changed = !self.checked.get();
        if push {
            self.apply_checked(true);
        } else if changed {
            self.checked.set(true);
            self.sync_checked_semantics();
        }
        self.uncheck_siblings();
        if changed {
            notify_change(&self.on_change, true);
        }
    }

    fn apply_checked(&self, checked: bool) {
        self.checked.set(checked);
        if let Some(peer) = self.peer.get() {
            peer.set_checked(checked);
        }
        self.sync_checked_semantics();
    }

    /// Unchecks every other live member of this button's group, notifying each one that was
    /// checked. Members are collected first so no registry borrow is held during the calls.
    fn uncheck_siblings(&self) {
        let group = self.group.borrow().clone();
        if group.is_empty() {
            return;
        }
        let members: Vec<Rc<dyn UIElementExt>> = GROUPS.with(|groups| {
            groups
                .borrow()
                .get(&group)
                .into_iter()
                .flat_map(|members| members.iter().filter_map(Weak::upgrade))
                .collect()
        });
        for member in members {
            let Some(member) = member.as_any().downcast_ref::<RadioButton>() else {
                continue;
            };
            if std::ptr::eq(member, self) || !member.checked.get() {
                continue;
            }
            member.apply_checked(false);
            notify_change(&member.on_change, false);
        }
    }

    fn sync_checked_semantics(&self) {
        let checked = self.checked.get();
        update_intrinsic_semantics(self.as_ui_element(), |semantics| {
            semantics.state.checked = Some(if checked {
                AccessibilityCheckState::On
            } else {
                AccessibilityCheckState::Off
            });
            semantics.state.selected = Some(checked);
        });
    }
}
