//! `NSSlider` — a continuous-value control. `NSSlider` inherits `NSControl` directly (not
//! `NSButton`), so it needs its own target/action trampoline rather than reusing `ButtonTarget` —
//! same reasoning as `inner/toggle_switch.rs`'s own `ToggleSwitchTarget` (`NSSwitch` is likewise
//! not an `NSButton`), whose exact shape this mirrors.

use crate::ffi::{AnyView, mtm};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AnyThread, DefinedClass, define_class, msg_send, sel};
use objc2_app_kit::NSSlider;
use objc2_foundation::NSObjectProtocol;
use std::cell::RefCell;
use std::rc::Rc;

pub(crate) struct InnerSlider {
    pub(crate) handle: AnyView,
    ns: Retained<NSSlider>,
    target_storage: Rc<RefCell<Option<Retained<SliderTarget>>>>,
}

impl InnerSlider {
    pub(crate) fn new() -> Self {
        let ns = NSSlider::new(mtm());
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

    pub(crate) fn set_value(&self, value: f32) {
        self.ns.setDoubleValue(value as f64);
    }

    pub(crate) fn set_min(&self, min: f32) {
        self.ns.setMinValue(min as f64);
    }

    pub(crate) fn set_max(&self, max: f32) {
        self.ns.setMaxValue(max as f64);
    }

    /// `NSControl.isContinuous` defaults to `YES` for `NSSlider`, so this fires repeatedly while
    /// dragging — matching a slider's usual UX, and needing no explicit opt-in (`Slider`'s own doc
    /// comment, elwindui-core).
    pub(crate) fn set_on_change(&self, callback: Box<dyn Fn(f32)>) {
        let ns = self.ns.clone();
        let target = SliderTarget::new(SliderTargetIvars {
            callback: Box::new(move || callback(ns.doubleValue() as f32)),
        });
        unsafe {
            self.ns.setTarget(Some(&target));
            self.ns.setAction(Some(sel!(perform:)));
        }
        *self.target_storage.borrow_mut() = Some(target);
    }
}

struct SliderTargetIvars {
    callback: Box<dyn Fn()>,
}

define_class!(
    #[unsafe(super(objc2_foundation::NSObject))]
    #[ivars = SliderTargetIvars]
    struct SliderTarget;

    unsafe impl NSObjectProtocol for SliderTarget {}

    impl SliderTarget {
        #[unsafe(method(perform:))]
        fn perform(&self, _sender: &AnyObject) {
            (self.ivars().callback)();
        }
    }
);

impl SliderTarget {
    fn new(ivars: SliderTargetIvars) -> Retained<Self> {
        let this = Self::alloc().set_ivars(ivars);
        unsafe { msg_send![super(this), init] }
    }
}
