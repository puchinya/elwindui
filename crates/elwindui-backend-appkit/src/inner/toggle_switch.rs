//! `NSSwitch` — a plain on/off control, not a differently-configured `NSButton` like `CheckBox`/
//! `RadioButton` above, so it needs its own click trampoline rather than reusing `ButtonTarget`
//! (which is typed to `&NSButton`). `NSSwitch` inherits `NSControl` directly, so `setTarget`/
//! `setAction` are available the same way.

use crate::ffi::{AnyView, mtm};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AnyThread, DefinedClass, define_class, msg_send, sel};
use objc2_app_kit::{NSControlStateValueOff, NSControlStateValueOn, NSSwitch};
use objc2_foundation::NSObjectProtocol;
use std::cell::RefCell;
use std::rc::Rc;

pub(crate) struct InnerToggleSwitch {
    pub(crate) handle: AnyView,
    ns: Retained<NSSwitch>,
    target_storage: Rc<RefCell<Option<Retained<ToggleSwitchTarget>>>>,
}

impl InnerToggleSwitch {
    pub(crate) fn new() -> Self {
        let ns = NSSwitch::new(mtm());
        let handle = AnyView::from(ns.clone());
        Self {
            handle,
            ns,
            target_storage: Rc::new(RefCell::new(None)),
        }
    }

    pub(crate) fn handle(&self) -> AnyView {
        self.handle.clone()
    }

    pub(crate) fn set_enabled(&self, enabled: bool) {
        self.ns.setEnabled(enabled);
    }

    pub(crate) fn set_is_on(&self, is_on: bool) {
        self.ns.setState(if is_on {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        });
    }

    pub(crate) fn set_on_change(&self, callback: Box<dyn Fn(bool)>) {
        let ns = self.ns.clone();
        let target = ToggleSwitchTarget::new(ToggleSwitchTargetIvars {
            callback: Box::new(move || callback(ns.state() == NSControlStateValueOn)),
        });
        unsafe {
            self.ns.setTarget(Some(&target));
            self.ns.setAction(Some(sel!(perform:)));
        }
        *self.target_storage.borrow_mut() = Some(target);
    }
}

struct ToggleSwitchTargetIvars {
    callback: Box<dyn Fn()>,
}

define_class!(
    #[unsafe(super(objc2_foundation::NSObject))]
    #[ivars = ToggleSwitchTargetIvars]
    struct ToggleSwitchTarget;

    unsafe impl NSObjectProtocol for ToggleSwitchTarget {}

    impl ToggleSwitchTarget {
        #[unsafe(method(perform:))]
        fn perform(&self, _sender: &AnyObject) {
            (self.ivars().callback)();
        }
    }
);

impl ToggleSwitchTarget {
    fn new(ivars: ToggleSwitchTargetIvars) -> Retained<Self> {
        let this = Self::alloc().set_ivars(ivars);
        unsafe { msg_send![super(this), init] }
    }
}
