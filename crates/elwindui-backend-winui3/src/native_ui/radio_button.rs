//! `builtin::RadioButton` — the `RadioButtonExt` implementation, including elwindui's own
//! group-exclusivity bookkeeping (`elwindui_core::ui::RadioButton`'s own doc comment explains why
//! this is deliberately not delegated to native grouping — mirrors
//! `elwindui_backend_appkit::native_ui::RadioButton`'s own registry exactly).

use super::NativeControl;
use crate::AnyView;
use crate::inner::InnerRadioButton;
use elwindui_core::ui::UIElementExt;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

// Deliberately *not* `use elwindui_core::base::AsAny;` — see `native_ui/mod.rs`'s own top-level
// comment for why a direct import makes `.as_any()` resolve to the blanket impl on
// `Rc<dyn UIElementExt>` itself instead of on the trait object it points to.

#[elwindui_macros::class(struct_only = elwindui_core::ui::RadioButtonExt, inherits = crate::NativeControl)]
pub struct RadioButton {
    inner: InnerRadioButton,
    group: RefCell<String>,
    on_change: RefCell<Option<Box<dyn Fn(bool)>>>,
}

thread_local! {
    static GROUPS: RefCell<HashMap<String, Vec<Weak<dyn UIElementExt>>>> =
        RefCell::new(HashMap::new());
}

#[elwindui_macros::class]
impl RadioButton {
    #[inherent]
    pub fn into_any_view(&self) -> AnyView {
        self.inner.handle()
    }

    fn set_text(&self, text: &str) {
        self.inner.set_text(text);
    }
    fn set_checked(&self, checked: bool) {
        self.inner.set_checked(checked);
        if checked {
            self.uncheck_siblings();
        }
    }
    /// Registers the write-back callback used by a two-way `checked` binding.
    #[inherent]
    pub fn set_on_checked_change(&self, callback: Box<dyn Fn(bool)>) {
        *self.on_change.borrow_mut() = Some(callback);
    }
    fn set_group(&self, group: &str) {
        *self.group.borrow_mut() = group.to_string();
        if group.is_empty() {
            return;
        }
        let owner: Rc<dyn UIElementExt> = self
            .as_ui_element()
            .visual_collection
            .owner_rc()
            .expect("RadioButton::set_group: object must already be Rc-constructed");
        GROUPS.with(|groups| {
            let mut groups = groups.borrow_mut();
            let members = groups.entry(group.to_string()).or_default();
            members.retain(|member| member.strong_count() > 0);
            members.push(Rc::downgrade(&owner));
        });
    }
    fn set_enabled(&self, enabled: bool) {
        self.inner.set_enabled(enabled);
    }

    fn construct() -> Self {
        let inner = InnerRadioButton::new();
        let handle = inner.handle();
        Self {
            base: NativeControl::construct(handle),
            inner,
            group: RefCell::new(String::new()),
            on_change: RefCell::new(None),
        }
    }

    fn on_constructed(&self) {
        self.set_tab_stop(true);
        let node: Rc<dyn UIElementExt> = self
            .as_ui_element()
            .visual_collection
            .owner_rc()
            .expect("RadioButton::on_constructed: object must already be Rc-constructed");
        self.inner.set_on_click(Box::new(move || {
            let this = node
                .as_any()
                .downcast_ref::<RadioButton>()
                .expect("owner_rc of a RadioButton must downcast to RadioButton");
            this.inner.set_checked(true);
            this.uncheck_siblings();
            if let Some(callback) = this.on_change.borrow().as_ref() {
                callback(true);
            }
        }));
    }

    #[inherent]
    fn uncheck_siblings(&self) {
        let group = self.group.borrow();
        if group.is_empty() {
            return;
        }
        GROUPS.with(|groups| {
            let groups = groups.borrow();
            let Some(members) = groups.get(group.as_str()) else {
                return;
            };
            for member in members {
                let Some(member) = member.upgrade() else {
                    continue;
                };
                let Some(member) = member.as_any().downcast_ref::<RadioButton>() else {
                    continue;
                };
                if std::ptr::eq(member, self) {
                    continue;
                }
                member.inner.set_checked(false);
                if let Some(callback) = member.on_change.borrow().as_ref() {
                    callback(false);
                }
            }
        });
    }
}
