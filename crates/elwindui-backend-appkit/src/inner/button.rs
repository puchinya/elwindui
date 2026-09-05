//! `NSButton` plus the target/action trampoline that turns a click into an `on_click` dispatch.

use crate::ffi::{AnyView, mtm};
use elwindui_core::ui::ButtonRole;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AnyThread, ClassType, DefinedClass, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAccessibility, NSButton, NSCellImagePosition, NSColor, NSControlSize, NSImage,
};
use objc2_foundation::{NSAttributedString, NSObjectProtocol, NSString};
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
        let target = ButtonTarget::attach(&self.ns, callback);
        *self.target_storage.borrow_mut() = Some(target);
    }

    /// Used by `TabChipImpl` to rename a tab's title button when its document's file name changes.
    pub(crate) fn set_text(&self, text: &str) {
        let title = NSString::from_str(text);
        self.ns.setTitle(&title);
        // AppKit keeps `attributedTitle` separately from `title`. Reset it to the same plain
        // string before NativeControl reapplies the cascaded style; otherwise a later `setTitle`
        // during a containing layout reconciliation can leave the button with a stale/empty
        // attributed payload even though Accessibility reports the new plain title correctly.
        let plain = unsafe {
            NSAttributedString::initWithString_attributes(NSAttributedString::alloc(), &title, None)
        };
        self.ns.setAttributedTitle(&plain);
        self.handle.as_nsview().setNeedsDisplay(true);
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

    /// Exists only so `TabChipViewIvars` (`inner/tab_view.rs`) can retain the actual close
    /// `NSButton` alongside this `InnerButton` wrapper — never exposed through `elwindui-core`.
    pub(crate) fn native_button(&self) -> Retained<NSButton> {
        self.ns.clone()
    }

    pub(crate) fn set_control_size(&self, size: NSControlSize) {
        self.ns.setControlSize(size);
    }

    /// Shows an SF Symbol when the running OS supports the class-side symbol-image constructor,
    /// otherwise falls back to plain text. Same runtime `respondsToSelector`-probing shape as
    /// [`set_has_destructive_action`] below — probed on `NSImage`'s class object since the
    /// constructor being probed is a class method, not an instance method.
    pub(crate) fn set_system_symbol_or_text(
        &self,
        symbol_name: &str,
        fallback_text: &str,
        accessibility_description: &str,
    ) {
        let symbol_image = if NSImage::class().responds_to(sel!(
            imageWithSystemSymbolName:accessibilityDescription:
        )) {
            NSImage::imageWithSystemSymbolName_accessibilityDescription(
                &NSString::from_str(symbol_name),
                Some(&NSString::from_str(accessibility_description)),
            )
        } else {
            None
        };
        match symbol_image {
            Some(image) => {
                self.ns.setImage(Some(&image));
                self.ns.setTitle(&NSString::from_str(""));
                self.ns.setImagePosition(NSCellImagePosition::ImageOnly);
            }
            None => {
                self.ns.setImage(None);
                self.ns.setTitle(&NSString::from_str(fallback_text));
            }
        }
        self.ns
            .setAccessibilityLabel(Some(&NSString::from_str(accessibility_description)));
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

pub(crate) struct ButtonTargetIvars {
    callback: Box<dyn Fn()>,
}

define_class!(
    #[unsafe(super(objc2_foundation::NSObject))]
    #[ivars = ButtonTargetIvars]
    pub(crate) struct ButtonTarget;

    unsafe impl NSObjectProtocol for ButtonTarget {}

    impl ButtonTarget {
        #[unsafe(method(perform:))]
        fn perform(&self, _sender: &AnyObject) {
            (self.ivars().callback)();
        }
    }
);

impl ButtonTarget {
    /// Shared by `InnerButton` and — since `CheckBox`/`RadioButton` are the same `NSButton`
    /// widget in a different `NSButtonType`, not a different class — `InnerCheckBox`/
    /// `InnerRadioButton`, which wire it directly rather than duplicating this trampoline.
    pub(crate) fn new(callback: Box<dyn Fn()>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(ButtonTargetIvars { callback });
        unsafe { msg_send![super(this), init] }
    }

    /// Wires `target`/`action` on any `NSButton`-family control and returns the target, which the
    /// caller must retain for as long as the click should keep firing (an `NSButton` does not
    /// retain its own `target`).
    pub(crate) fn attach(button: &NSButton, callback: Box<dyn Fn()>) -> Retained<Self> {
        let target = Self::new(callback);
        unsafe {
            button.setTarget(Some(&target));
            button.setAction(Some(sel!(perform:)));
        }
        target
    }
}
