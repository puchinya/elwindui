//! The three text controls: `NSTextView` (`TextArea`) and `NSTextField`/`NSSecureTextField`
//! (`TextBox`/`PasswordBox`), plus the delegates that turn native edits into `on_change`.

use crate::ffi::{AnyView, mtm};
use objc2::rc::Retained;
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel,
};
use objc2_app_kit::{
    NSControlTextEditingDelegate, NSFont, NSSecureTextField,
    NSTextDelegate, NSTextField, NSTextFieldDelegate, NSTextView, NSTextViewDelegate,
};
use objc2_foundation::{NSNotification, NSObjectProtocol, NSString};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Raw `NSTextView` + change-notification delegate — composed by `native_ui::TextArea`.
pub(crate) struct InnerTextArea {
    handle: AnyView,
    text_view: Retained<NSTextView>,
    delegate_storage: Rc<RefCell<Option<Retained<TextViewDelegate>>>>,
    /// See `measure`'s own doc comment for why these exist and how they're computed. `Cell`, not a
    /// plain field: `recompute_default_size` (called from `apply_text_style`, since a font change
    /// changes the text view's own line-height metrics) refreshes them through `&self`.
    default_width: Cell<f32>,
    default_height: Cell<f32>,
}

impl InnerTextArea {
    pub(crate) fn new() -> Self {
        let m = mtm();
        let scroll = NSTextView::scrollableTextView(m);
        let text_view = scroll
            .documentView()
            .expect("scrollableTextView always has a document view")
            .downcast::<NSTextView>()
            .expect("scrollableTextView's document view is an NSTextView");

        let (default_width, default_height) = Self::compute_default_size(&text_view);

        let handle = AnyView::from(scroll);
        Self {
            handle,
            text_view,
            delegate_storage: Rc::new(RefCell::new(None)),
            default_width: Cell::new(default_width),
            default_height: Cell::new(default_height),
        }
    }

    /// `NSScrollView.fittingSize()` reports `{0,0}` regardless of the view's current frame — unlike
    /// a plain `NSView`/`NSControl`, it does not fall back to echoing frame.size when unconstrained
    /// (verified empirically: setting a non-zero frame here has no effect on what `fittingSize()`
    /// later reports). So `TextArea` cannot rely on the generic `NativeControl::measure_override`
    /// -> `AnyView::measure` -> `fittingSize()` path every other native leaf shares (see that
    /// method's own doc comment) — `native_ui::TextArea` overrides `measure_override` itself and
    /// calls `InnerTextArea::measure` below instead. The height is derived from the text view's
    /// own current font metrics, matching how `NSTextField` (`InnerTextBox`) gets a non-zero
    /// default from its cell's real `intrinsicContentSize`, and mirroring how WinUI3's `TextArea`
    /// (`elwindui-backend-winui3::inner::InnerTextArea`) always has a non-zero minimum height from
    /// its default style (it isn't wrapped in a `ScrollViewer` there).
    ///
    /// Recomputed (not just computed once at construction) because a `font_size`/`font_family`
    /// change after construction would otherwise leave these — and therefore `measure`'s result —
    /// silently stale; see `apply_text_style`'s call site.
    fn compute_default_size(text_view: &NSTextView) -> (f32, f32) {
        let font = text_view
            .font()
            .unwrap_or_else(|| NSFont::systemFontOfSize(NSFont::systemFontSize()));
        let line_height = unsafe { text_view.layoutManager() }
            .map(|lm| lm.defaultLineHeightForFont(&font))
            .unwrap_or_else(|| font.pointSize());
        let inset = text_view.textContainerInset();
        let default_height = (line_height + inset.height * 2.0) as f32;
        // No single metric analogous to `defaultLineHeightForFont` exists for "typical text
        // width" short of measuring an actual reference string (which would pull in
        // `NSAttributedString`/`NSDictionary` bindings this crate doesn't otherwise need). Derive
        // a reasonably wide default from the same line-height metric instead of a bare pixel
        // constant — used by both `VerticalLayout` (whose cross-axis stretch — see
        // `crates/elwindui-core/src/layout.rs`'s stack-arrange doc comment — makes this value
        // largely moot there) and `HorizontalLayout`, whose *main* axis is width, so a `TextArea`
        // inside one has no such stretch to fall back on and needs a real measured value here.
        let default_width = default_height * 20.0;
        (default_width, default_height)
    }

    pub(crate) fn handle(&self) -> AnyView {
        self.handle.clone()
    }

    /// Refreshes `default_width`/`default_height` from the text view's *current* font metrics.
    /// `native_ui::TextArea::measure_override` calls this right after `sync_text_style()` — the
    /// generic `AppKitHandle::apply_text_style` impl on `Retained<NSScrollView>` (`ffi.rs`) already
    /// pushed any new font onto this same `text_view` by that point, but has no way to reach back
    /// into `InnerTextArea`'s own cached size fields; this closes that loop so a `font_size`/
    /// `font_family` change doesn't leave `measure`'s result silently stale.
    pub(crate) fn refresh_default_size(&self) {
        let (width, height) = Self::compute_default_size(&self.text_view);
        self.default_width.set(width);
        self.default_height.set(height);
    }

    /// See the doc comment on `default_width`/`default_height` for why this exists instead of
    /// `native_ui::NativeControl`'s shared `fittingSize()`-based `measure_override`.
    pub(crate) fn measure(&self, _available: elwindui_core::base::Size) -> elwindui_core::base::Size {
        elwindui_core::base::Size {
            width: self.default_width.get(),
            height: self.default_height.get(),
        }
    }

    /// `NSTextView.setString:` resets the caret/selection. In the normal two-way input path the
    /// native buffer has already changed before its delegate calls the model setter, so identical
    /// model→widget updates must be a no-op.
    pub(crate) fn set_text(&self, text: &str) {
        if self.text_view.string().to_string() == text {
            return;
        }
        self.text_view.setString(&NSString::from_str(text));
    }

    /// `NSTextView.delegate` is an unretained (weak) reference, so the delegate this creates is
    /// only kept alive by `self.delegate_storage`.
    pub(crate) fn set_on_change(&self, callback: Box<dyn Fn(String)>) {
        let m = mtm();
        let ivars = TextDelegateIvars {
            text_view: self.text_view.clone(),
            callback,
        };
        let delegate = TextViewDelegate::new(m, ivars);
        let protocol_obj: &objc2::runtime::ProtocolObject<dyn NSTextViewDelegate> =
            objc2::runtime::ProtocolObject::from_ref(&*delegate);
        self.text_view.setDelegate(Some(protocol_obj));
        *self.delegate_storage.borrow_mut() = Some(delegate);
    }
}

struct TextDelegateIvars {
    text_view: Retained<NSTextView>,
    callback: Box<dyn Fn(String)>,
}

define_class!(
    #[unsafe(super(objc2_foundation::NSObject))]
    #[thread_kind = objc2::MainThreadOnly]
    #[ivars = TextDelegateIvars]
    struct TextViewDelegate;

    unsafe impl NSObjectProtocol for TextViewDelegate {}

    unsafe impl NSTextDelegate for TextViewDelegate {
        #[unsafe(method(textDidChange:))]
        fn text_did_change(&self, _notification: &NSNotification) {
            let s = self.ivars().text_view.string();
            (self.ivars().callback)(s.to_string());
        }
    }

    unsafe impl NSTextViewDelegate for TextViewDelegate {}
);

impl TextViewDelegate {
    fn new(mtm: MainThreadMarker, ivars: TextDelegateIvars) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ivars);
        unsafe { msg_send![super(this), init] }
    }
}

/// Shared machinery for any `NSTextField`-family single-line native leaf: `TextBox`'s own
/// `NSTextField`, and `PasswordBox`'s `NSSecureTextField` (a direct `NSTextField` subclass — see
/// `AppKitHandle for Retained<NSTextField>` above and `InnerPasswordBox`, which upcasts its own
/// `Retained<NSSecureTextField>` via `Retained::into_super` before wrapping it here). Built once for
/// `InnerTextBox` and reused verbatim by `InnerPasswordBox` — both widgets need the exact same
/// value-compare-guarded `set_string_value`/delegate-wiring/`max_length`-truncation logic, and
/// writing it twice would just be two copies of the same bug surface (see
/// `docs/elwindui_nativecontrol_expansion_status.md` on the "generalize before duplicating" policy
/// this follows).
///
/// Unlike `InnerTextArea`'s `TextViewDelegate` (constructed fresh, once, inside `set_on_change`
/// alone), this widget family also needs an optional submit-on-Enter callback (`TextBox` only, see
/// `InnerTextBox::set_on_submit`) that a *second* setter call must be able to wire without
/// clobbering whichever of `set_on_change`/`set_on_submit` was set first — `NSControl.delegate` is a
/// single slot, so two separately-constructed delegate objects each calling `setDelegate:` would
/// silently drop whichever wired second. The delegate here is therefore built exactly once, in
/// `NativeTextFieldCommon::new`, and every setter only ever mutates that one delegate's own interior
/// `RefCell`/`Cell` state — never re-registers a new delegate object.
struct NativeTextFieldCommon {
    field: Retained<NSTextField>,
    delegate: Retained<NativeTextFieldDelegate>,
}

impl NativeTextFieldCommon {
    fn new(field: Retained<NSTextField>) -> Self {
        let ivars = NativeTextFieldDelegateIvars {
            field: field.clone(),
            max_length: Cell::new(None),
            on_change: RefCell::new(None),
            on_submit: RefCell::new(None),
        };
        let delegate = NativeTextFieldDelegate::new(mtm(), ivars);
        let protocol_obj: &objc2::runtime::ProtocolObject<dyn NSTextFieldDelegate> =
            objc2::runtime::ProtocolObject::from_ref(&*delegate);
        // `NSTextField.delegate` is an unretained (weak) reference, so this delegate is only kept
        // alive by `self.delegate` for as long as this `NativeTextFieldCommon` (and, transitively,
        // its owning `InnerTextBox`/`InnerPasswordBox`) lives.
        unsafe { field.setDelegate(Some(protocol_obj)) };
        Self { field, delegate }
    }

    /// Same value-compare-guard idiom as `InnerTextArea::set_text` — see that method's own doc
    /// comment for why: a `#[two_way]`-bound field re-syncs on every model change, including the one
    /// the native edit itself just caused, so an identical model→widget update must be a no-op.
    fn set_string_value(&self, text: &str) {
        if self.field.stringValue().to_string() == text {
            return;
        }
        self.field.setStringValue(&NSString::from_str(text));
    }

    fn set_max_length(&self, max_length: Option<u32>) {
        self.delegate.ivars().max_length.set(max_length);
    }

    fn set_on_change(&self, callback: Box<dyn Fn(String)>) {
        *self.delegate.ivars().on_change.borrow_mut() = Some(callback);
    }

    /// `TextBox`-only (see `InnerTextBox::set_on_submit`'s own doc comment) — harmless to expose
    /// here too rather than duplicating this setter, since `PasswordBox` simply never calls it.
    fn set_on_submit(&self, callback: Box<dyn Fn()>) {
        *self.delegate.ivars().on_submit.borrow_mut() = Some(callback);
    }
}

struct NativeTextFieldDelegateIvars {
    field: Retained<NSTextField>,
    max_length: Cell<Option<u32>>,
    on_change: RefCell<Option<Box<dyn Fn(String)>>>,
    on_submit: RefCell<Option<Box<dyn Fn()>>>,
}

define_class!(
    #[unsafe(super(objc2_foundation::NSObject))]
    #[thread_kind = objc2::MainThreadOnly]
    #[ivars = NativeTextFieldDelegateIvars]
    struct NativeTextFieldDelegate;

    unsafe impl NSObjectProtocol for NativeTextFieldDelegate {}

    unsafe impl NSControlTextEditingDelegate for NativeTextFieldDelegate {
        #[unsafe(method(controlTextDidChange:))]
        fn control_text_did_change(&self, _notification: &NSNotification) {
            let field = &self.ivars().field;
            // AppKit has no native max-length primitive on `NSTextField`, unlike WinUI3's
            // `TextBox.MaxLength` (`native_ui::TextBox`'s own doc comment on this asymmetry) — so a
            // limit is enforced here, after the fact, by truncating straight back. This can move
            // the caret; that's an accepted (and, without a native primitive, unavoidable) cost of
            // enforcing the limit at all, and only happens on the one edit that actually exceeds it.
            if let Some(max_length) = self.ivars().max_length.get() {
                let current = field.stringValue().to_string();
                if current.chars().count() > max_length as usize {
                    let truncated: String = current.chars().take(max_length as usize).collect();
                    field.setStringValue(&NSString::from_str(&truncated));
                }
            }
            let s = field.stringValue();
            if let Some(callback) = self.ivars().on_change.borrow().as_ref() {
                callback(s.to_string());
            }
        }

        /// `TextBox`-only submit-on-Enter (`InnerTextBox::set_on_submit`'s own doc comment on the
        /// narrow scope of this addition). `on_submit` stays `None` for `PasswordBox`, so this is a
        /// no-op there — the same single delegate serves both widgets regardless.
        #[unsafe(method(control:textView:doCommandBySelector:))]
        unsafe fn control_text_view_do_command_by_selector(
            &self,
            _control: &objc2_app_kit::NSControl,
            _text_view: &NSTextView,
            command_selector: objc2::runtime::Sel,
        ) -> bool {
            if command_selector == sel!(insertNewline:) {
                if let Some(callback) = self.ivars().on_submit.borrow().as_ref() {
                    callback();
                }
            }
            false
        }
    }

    unsafe impl NSTextFieldDelegate for NativeTextFieldDelegate {}
);

impl NativeTextFieldDelegate {
    fn new(mtm: MainThreadMarker, ivars: NativeTextFieldDelegateIvars) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ivars);
        unsafe { msg_send![super(this), init] }
    }
}

/// Raw `NSTextField` + change-notification delegate — composed by `native_ui::TextBox`. Contrast
/// with `InnerTextArea` (`NSTextView` wrapped in an `NSScrollView` via `scrollableTextView`): this
/// wraps a bare `NSTextField` directly, no scrolling concept at all, single line only.
pub(crate) struct InnerTextBox {
    handle: AnyView,
    field: Retained<NSTextField>,
    common: NativeTextFieldCommon,
}

impl InnerTextBox {
    pub(crate) fn new() -> Self {
        let m = mtm();
        let field = NSTextField::new(m);
        field.setBezeled(true);
        field.setEditable(true);
        let handle = AnyView::from(field.clone());
        let common = NativeTextFieldCommon::new(field.clone());
        Self {
            handle,
            field,
            common,
        }
    }

    pub(crate) fn handle(&self) -> AnyView {
        self.handle.clone()
    }

    pub(crate) fn set_text(&self, text: &str) {
        self.common.set_string_value(text);
    }

    pub(crate) fn set_placeholder(&self, text: &str) {
        self.field
            .setPlaceholderString(Some(&NSString::from_str(text)));
    }

    pub(crate) fn set_read_only(&self, read_only: bool) {
        self.field.setEditable(!read_only);
    }

    pub(crate) fn set_max_length(&self, max_length: Option<u32>) {
        self.common.set_max_length(max_length);
    }

    pub(crate) fn set_text_alignment(&self, alignment: elwindui_core::ui::TextAlignment) {
        use elwindui_core::ui::TextAlignment;
        use objc2_app_kit::NSTextAlignment;
        self.field.setAlignment(match alignment {
            TextAlignment::Left => NSTextAlignment::Left,
            TextAlignment::Center => NSTextAlignment::Center,
            TextAlignment::Right => NSTextAlignment::Right,
        });
    }

    pub(crate) fn set_on_change(&self, callback: Box<dyn Fn(String)>) {
        self.common.set_on_change(callback);
    }

    /// TextBox-specific, narrowly-scoped addition (not the general native-keyboard-forwarding
    /// problem `docs/elwindui_gui_framework_design.md` §5.5/§8.1 documents as a known limitation for
    /// Tab-out-of-a-focused-native-leaf): detects the Enter key via
    /// `NSControlTextEditingDelegate::control:textView:doCommandBySelector:` (`insertNewline:`) and
    /// forwards it through `callback`, letting `native_ui::TextBox::on_constructed` dispatch it as an
    /// ordinary `on_key_down` — see that method's own doc comment.
    pub(crate) fn set_on_submit(&self, callback: Box<dyn Fn()>) {
        self.common.set_on_submit(callback);
    }
}

/// Raw `NSSecureTextField` + change-notification delegate — composed by `native_ui::PasswordBox`.
/// `NSSecureTextField` is a direct `NSTextField` subclass (see `AppKitHandle for
/// Retained<NSTextField>` above), so it's upcast once at construction and handed to the exact same
/// `NativeTextFieldCommon` `InnerTextBox` uses — see that shared type's own doc comment for why this
/// reuse, rather than a second copy of the same delegate/value-guard/max-length machinery, is
/// deliberate.
pub(crate) struct InnerPasswordBox {
    handle: AnyView,
    field: Retained<NSSecureTextField>,
    common: NativeTextFieldCommon,
}

impl InnerPasswordBox {
    pub(crate) fn new() -> Self {
        let m = mtm();
        let field = NSSecureTextField::new(m);
        field.setBezeled(true);
        field.setEditable(true);
        // `AppKitHandle` is only implemented for `Retained<NSTextField>` (not
        // `Retained<NSSecureTextField>` — see that impl's own doc comment on why one impl per raw
        // widget type, not per class-hierarchy level), so `AnyView` wraps the upcast handle too,
        // not `field` itself.
        let upcast: Retained<NSTextField> = Retained::into_super(field.clone());
        let handle = AnyView::from(upcast.clone());
        let common = NativeTextFieldCommon::new(upcast);
        Self {
            handle,
            field,
            common,
        }
    }

    pub(crate) fn handle(&self) -> AnyView {
        self.handle.clone()
    }

    pub(crate) fn set_password(&self, password: &str) {
        self.common.set_string_value(password);
    }

    pub(crate) fn set_placeholder(&self, text: &str) {
        self.field
            .setPlaceholderString(Some(&NSString::from_str(text)));
    }

    pub(crate) fn set_max_length(&self, max_length: Option<u32>) {
        self.common.set_max_length(max_length);
    }

    pub(crate) fn set_on_change(&self, callback: Box<dyn Fn(String)>) {
        self.common.set_on_change(callback);
    }

    /// `NSSecureTextField` has no native "reveal password" toggle, unlike WinUI3's
    /// `PasswordRevealMode` (`native_ui::PasswordBox`'s own doc comment has the full asymmetry).
    /// A full implementation would compose a custom "eye" toggle button that swaps the live
    /// obscured field for a plain `NSTextField` showing the same string — real, but disproportionate
    /// scope for this control's Phase 1 first cut (see
    /// docs/elwindui_nativecontrol_expansion_status.md). `true` is therefore silently a no-op here;
    /// the setter stays wired so a future pass has a real place to land.
    pub(crate) fn set_reveal_enabled(&self, _enabled: bool) {}
}
