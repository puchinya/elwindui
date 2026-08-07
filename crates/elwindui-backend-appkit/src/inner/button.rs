//! `NSButton` plus the target/action trampoline that turns a click into an `on_click` dispatch.

use crate::ffi::{AnyView, mtm};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{
    AnyThread, DefinedClass, define_class, msg_send, sel,
};
use elwindui_core::ui::ButtonRole;
use objc2_app_kit::{NSButton, NSColor};
use objc2_foundation::{NSObjectProtocol, NSString};
use std::cell::RefCell;
use std::rc::Rc;

/// Raw `NSButton` + click target — composed by `native_ui::Button` (and used directly, not through
/// `native_ui::Button`, by `TabChipImpl`/`TabStripImpl` below for their own internal chip/strip
/// buttons — see those types' own doc comments).
pub(crate) struct InnerButton {
    pub(crate) handle: AnyView,
    ns: Retained<NSButton>,
    target_storage: Rc<RefCell<Option<Retained<ButtonTarget>>>>,
}

impl InnerButton {
    pub(crate) fn new() -> Self {
        let m = mtm();
        let ns = unsafe {
            NSButton::buttonWithTitle_target_action(&NSString::from_str(""), None, None, m)
        };
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

    pub(crate) fn set_on_click(&self, callback: Box<dyn Fn()>) {
        let target = ButtonTarget::new(ButtonTargetIvars { callback });
        unsafe {
            self.ns.setTarget(Some(&target));
            self.ns.setAction(Some(sel!(perform:)));
        }
        *self.target_storage.borrow_mut() = Some(target);
    }

    /// Used by `TabChipImpl` to rename a tab's title button when its document's file name changes.
    pub(crate) fn set_text(&self, text: &str) {
        self.ns.setTitle(&NSString::from_str(text));
    }

    /// Applies a `ButtonRole`'s native emphasis, always resetting both knobs so switching roles at
    /// runtime can't leave a previous role's treatment behind.
    ///
    /// Both emphasized roles work through `bezelColor` — the filled-button look — differing only
    /// in which system colour fills it. That is deliberate, and the two obvious-looking
    /// alternatives were both tried and rejected against a real window:
    ///
    /// - `contentTintColor` does nothing to a standard bordered push button's title (it applies to
    ///   template images and a few other control types), so a red tint alone left `Destructive`
    ///   indistinguishable from `Normal`.
    /// - An attributed title with a red foreground is actively unworkable here: `AppKitHandle::
    ///   apply_text_style` (`ffi.rs`) calls `setTitle` on every layout pass, which discards any
    ///   attributed title a moment after this sets one.
    ///
    /// `hasDestructiveAction` is still set on macOS 11+, but for the semantic signal it carries to
    /// AppKit and assistive technology — on its own it produces no visible change on an unfilled
    /// button, so it cannot be what makes the role legible.
    ///
    /// `Primary` uses `bezelColor` rather than `keyEquivalent`, even though a default button is
    /// also accent-filled on macOS: `keyEquivalent` is what [`Self::set_is_default`] owns, and the
    /// two properties are orthogonal — a `Primary` button need not be the Return target, and a
    /// `Destructive` one often must not be.
    pub(crate) fn set_role(&self, role: ButtonRole) {
        let (bezel, destructive) = match role {
            ButtonRole::Normal => (None, false),
            ButtonRole::Primary => (Some(NSColor::controlAccentColor()), false),
            ButtonRole::Destructive => (Some(NSColor::systemRedColor()), true),
        };
        self.ns.setBezelColor(bezel.as_deref());
        set_has_destructive_action(&self.ns, destructive);
    }

    /// Makes this the window's default button, so Return activates it. AppKit expresses that as
    /// the carriage-return key equivalent, and draws the accent fill itself as a consequence.
    pub(crate) fn set_is_default(&self, is_default: bool) {
        self.ns
            .setKeyEquivalent(&NSString::from_str(if is_default { "\r" } else { "" }));
    }

    /// AppKit-only helper (no `elwindui_core::ui::Button` trait member — WinUI3's real `TabView`
    /// highlights its selected tab for free, no borderless-button trick needed there): used by
    /// `create_tab_chip` so `TabChipImpl::set_selected`'s translucent background tint shows through
    /// instead of being hidden behind the button's own opaque default bezel.
    pub(crate) fn set_bordered(&self, bordered: bool) {
        self.ns.setBordered(bordered);
    }
}

/// `NSButton.hasDestructiveAction` is macOS 11+, and objc2 generates its binding unconditionally —
/// calling it on 10.x would raise `unrecognized selector`. No `#[cfg]` can express a *runtime* OS
/// check, so probe the selector instead.
///
/// This is the first version-gated AppKit call in this crate; follow this shape for the next one
/// rather than reaching for a deployment-target `#[cfg]`, which would pin the whole binary to the
/// newer OS instead of degrading on the older one.
fn set_has_destructive_action(button: &NSButton, value: bool) {
    if button.respondsToSelector(sel!(setHasDestructiveAction:)) {
        button.setHasDestructiveAction(value);
    }
}

struct ButtonTargetIvars {
    callback: Box<dyn Fn()>,
}

define_class!(
    #[unsafe(super(objc2_foundation::NSObject))]
    #[ivars = ButtonTargetIvars]
    struct ButtonTarget;

    unsafe impl NSObjectProtocol for ButtonTarget {}

    impl ButtonTarget {
        #[unsafe(method(perform:))]
        fn perform(&self, _sender: &AnyObject) {
            (self.ivars().callback)();
        }
    }
);

impl ButtonTarget {
    fn new(ivars: ButtonTargetIvars) -> Retained<Self> {
        let this = Self::alloc().set_ivars(ivars);
        unsafe { msg_send![super(this), init] }
    }
}
