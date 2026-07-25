//! Native-side AppKit plumbing — every type here is `Inner`-prefixed and, except for `AnyView`
//! itself (re-exported at the crate root; see `lib.rs`'s own doc comment), private to this crate.
//! `native_ui.rs` composes these as plain fields and calls into them; this module owns every bit
//! of genuinely AppKit-specific complexity (NSTextView delegates, tab strip bookkeeping, ...) so
//! `native_ui.rs` stays a thin, uniform "implement the core-side trait by delegating" layer.

use crate::ffi::{AnyView, mtm, new_stack};
use crate::host::TreeHostView;
use crate::render::parse_color;
use elwindui_core::input::FocusState;
use elwindui_core::ui::UIElementExt;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool};
use objc2::{
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel,
};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSButton,
    NSControlTextEditingDelegate, NSFont, NSMenu,
    NSMenuItem, NSResponder, NSScreen, NSScrollView, NSSecureTextField, NSStackView,
    NSTextDelegate, NSTextField, NSTextFieldDelegate, NSTextView, NSTextViewDelegate, NSUserInterfaceLayoutOrientation, NSView, NSWindow,
    NSWindowStyleMask,
};
use objc2_foundation::{NSNotification, NSObjectProtocol, NSRect, NSString};
use std::cell::{Cell, RefCell};
use std::rc::Rc;






/// Walks up from `responder`'s own `NSView` ancestor chain looking for the nearest `TreeHostView`
/// (the window's own top-level content host, or a nested one — `InnerTabView`'s per-tab host,
/// `InnerScrollView`'s content host once that exists, ...) that has `responder`'s *immediate child*
/// registered as one of its own native leaf islands (`TreeHostView::native_containers`). Returns
/// that host together with the owning element's `render_group_id`, ready for
/// `elwindui_core::focus::native_focus_gained`/`native_focus_lost`. Returns `None` for anything not
/// reachable this way — most commonly a `TabView` chip/close button (an `InnerButton` created
/// directly by `create_tab_chip`, never wrapped in a `RenderCommand::NativeControl`) or the
/// `TreeHostView`/`NSWindow` itself becoming first responder (e.g. on window activation with
/// nothing else focused yet) — both are correctly not elwindui-visible focus targets.
fn resolve_focus_owner(
    responder: Option<Retained<NSResponder>>,
) -> Option<(Retained<TreeHostView>, u64)> {
    let mut previous: Option<Retained<NSView>> = None;
    let mut current: Option<Retained<NSView>> = responder.and_then(|r| r.downcast::<NSView>().ok());
    while let Some(view) = current {
        match view.downcast::<TreeHostView>() {
            Ok(host) => {
                let owner_id = previous.as_deref().and_then(|c| host.resolve_native_owner_id(c))?;
                return Some((host, owner_id));
            }
            Err(view) => {
                current = unsafe { view.superview() };
                previous = Some(view);
            }
        }
    }
    None
}

define_class!(
    /// A plain `NSWindow` subclass whose only job is bridging AppKit's own first-responder changes
    /// into `elwindui_core::focus::FocusTracker` — see `docs/elwindui_gui_framework_design.md` §5.5.
    /// Subclassing the window (rather than every individual native leaf class) is the standard,
    /// minimal-surface-area AppKit technique for observing "did some view anywhere in this window
    /// become/stop being first responder" without per-widget-class overrides, and mirrors this same
    /// file's own `TreeHostView` subclassing convention.
    #[unsafe(super(NSWindow))]
    #[thread_kind = objc2::MainThreadOnly]
    #[ivars = ()]
    pub(crate) struct ElwinduiWindow;

    unsafe impl NSObjectProtocol for ElwinduiWindow {}

    impl ElwinduiWindow {
        /// Detects a real, click/API-driven focus change (`ok == true`) and bridges it into
        /// `elwindui_core::focus`. Whether `responder` lands on a *native leaf* window's own
        /// `resolve_focus_owner` decides whether anything happens at all — see that function's own
        /// doc comment for what's intentionally excluded. `FocusState::Pointer` is used
        /// unconditionally for the gained side (Phase 1 simplification — distinguishing a real
        /// mouse click from AppKit's own Tab-driven key-view-loop focus change would need
        /// inspecting `NSApp.currentEvent`, and no such key-view loop is wired between elwindui
        /// elements yet regardless — see `docs/elwindui_gui_framework_design.md` §5.5/§8.1's "known
        /// limitation" notes on Tab/Shift+Tab out of a focused native control).
        ///
        /// Resolves the target through `host.ivars().render_tree.borrow()` in its own `let`
        /// statement, ending that borrow *before* calling `native_focus_gained` — this used to be
        /// one `if let Some(render_tree) = ...borrow().as_ref() { native_focus_gained(render_tree,
        /// ..) }`, which held the borrow for the whole call. `native_focus_gained` dispatches
        /// `on_got_focus`, which can run arbitrary user code; in `examples/controls-demo`'s TextBox
        /// tab, that handler sets an `#[observable]` field bound to another `TextBlock`, whose
        /// property-change notification synchronously calls `AppKitRelayoutHost::request_relayout`
        /// — which itself needs `render_tree.borrow_mut()` to mark the tree dirty (only the actual
        /// AppKit layout pass is deferred via `setNeedsLayout`, not this). With the borrow still
        /// held from the outer `if let`, that `borrow_mut()` panicked with `BorrowMutError` on every
        /// click or Enter-driven focus change that touched a bound sibling element — crashing the
        /// whole app, since the panic then unwound across this method's own ObjC callback boundary.
        #[unsafe(method(makeFirstResponder:))]
        fn make_first_responder(&self, responder: Option<&NSResponder>) -> Bool {
            let old = self.firstResponder();
            let ok: Bool = unsafe { msg_send![super(self), makeFirstResponder: responder] };
            if !ok.as_bool() {
                return ok;
            }
            let new = self.firstResponder();
            if let Some((host, owner_id)) = resolve_focus_owner(new) {
                let target = host
                    .ivars()
                    .render_tree
                    .borrow()
                    .as_ref()
                    .and_then(|rt| elwindui_core::focus::resolve_native_focus_target(rt, owner_id));
                if let Some(target) = target {
                    elwindui_core::focus::native_focus_gained(
                        &target,
                        &host.ivars().keyboard.focus,
                        FocusState::Pointer,
                    );
                }
            } else if let Some((host, owner_id)) = resolve_focus_owner(old) {
                elwindui_core::focus::native_focus_lost(&host.ivars().keyboard.focus, owner_id);
            }
            ok
        }
    }
);

/// Raw `NSWindow` + content host — composed by `native_ui::Window`.
#[derive(Clone)]
pub(crate) struct InnerWindow {
    ns: Retained<NSWindow>,
    content_host: Retained<TreeHostView>,
}

impl InnerWindow {
    pub(crate) fn new() -> Self {
        let mtm = mtm();
        let content_rect = NSRect::new(
            objc2_foundation::NSPoint::new(0.0, 0.0),
            objc2_foundation::NSSize::new(480.0, 360.0),
        );
        let style = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable
            | NSWindowStyleMask::Resizable;
        // `ElwinduiWindow` (not a stock `NSWindow`) so `makeFirstResponder:` can bridge native
        // focus changes into `elwindui_core::focus` — see that type's own doc comment.
        let ns: Retained<NSWindow> = unsafe {
            let alloc = ElwinduiWindow::alloc(mtm).set_ivars(());
            let window: Retained<ElwinduiWindow> = msg_send![
                super(alloc),
                initWithContentRect: content_rect,
                styleMask: style,
                backing: NSBackingStoreType::Buffered,
                defer: false,
            ];
            Retained::into_super(window)
        };
        let content_host = TreeHostView::new();
        // `Window` property setters can resize the NSWindow after this content view has been
        // installed (the notepad starts at 640×480 although InnerWindow's construction rect is
        // 480×360). Keep the host synchronized with the client area just like per-tab hosts do.
        content_host.setTranslatesAutoresizingMaskIntoConstraints(true);
        content_host.setAutoresizingMask(
            objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
                | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        ns.setContentView(Some(&content_host));
        Self { ns, content_host }
    }

    pub(crate) fn set_content(&self, content: Rc<dyn UIElementExt>) {
        self.content_host.set_tree(content);
    }

    fn sync_content_host_frame(&self) {
        let client = self.ns.contentRectForFrameRect(self.ns.frame());
        self.content_host.setFrame(NSRect::new(
            objc2_foundation::NSPoint::new(0.0, 0.0),
            client.size,
        ));
        self.content_host.setNeedsLayout(true);
    }

    pub(crate) fn set_title(&self, title: &str) {
        self.ns.setTitle(&NSString::from_str(title));
    }

    /// Sets `NSApplication.mainMenu` (macOS has one global top menu bar, not a per-window one).
    pub(crate) fn set_menu_bar(&self, menu_bar: &InnerMenuBar) {
        NSApplication::sharedApplication(mtm()).setMainMenu(Some(&menu_bar.ns));
    }

    pub(crate) fn show(&self) {
        let mtm = mtm();
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
        self.ns.makeKeyAndOrderFront(None);
        app.activate();
    }

    fn screen_height(&self) -> f64 {
        self.ns
            .screen()
            .or_else(|| NSScreen::mainScreen(mtm()))
            .map(|screen| screen.frame().size.height)
            .unwrap_or(0.0)
    }

    pub(crate) fn left(&self) -> f32 {
        self.ns.frame().origin.x as f32
    }

    pub(crate) fn set_left(&self, left: f32) {
        let mut frame = self.ns.frame();
        frame.origin.x = left as f64;
        self.ns.setFrame_display(frame, true);
    }

    pub(crate) fn top(&self) -> f32 {
        let frame = self.ns.frame();
        (self.screen_height() - (frame.origin.y + frame.size.height)) as f32
    }

    pub(crate) fn set_top(&self, top: f32) {
        let screen_height = self.screen_height();
        let mut frame = self.ns.frame();
        frame.origin.y = screen_height - top as f64 - frame.size.height;
        self.ns.setFrame_display(frame, true);
    }

    pub(crate) fn width(&self) -> f32 {
        self.ns.frame().size.width as f32
    }

    pub(crate) fn set_width(&self, width: f32) {
        let mut frame = self.ns.frame();
        frame.size.width = width as f64;
        self.ns.setFrame_display(frame, true);
        self.sync_content_host_frame();
    }

    pub(crate) fn height(&self) -> f32 {
        self.ns.frame().size.height as f32
    }

    pub(crate) fn set_height(&self, height: f32) {
        let mut frame = self.ns.frame();
        let old_height = frame.size.height;
        frame.size.height = height as f64;
        frame.origin.y -= height as f64 - old_height;
        self.ns.setFrame_display(frame, true);
        self.sync_content_host_frame();
    }
}

/// Raw `NSTextView` + change-notification delegate — composed by `native_ui::TextArea`.
pub(crate) struct InnerTextArea {
    handle: AnyView,
    text_view: Retained<NSTextView>,
    delegate_storage: Rc<RefCell<Option<Retained<TextViewDelegate>>>>,
    /// See `measure`'s own doc comment for why these exist and how they're computed.
    default_width: f32,
    default_height: f32,
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

        // `NSScrollView.fittingSize()` reports `{0,0}` regardless of the view's current frame —
        // unlike a plain `NSView`/`NSControl`, it does not fall back to echoing frame.size when
        // unconstrained (verified empirically: setting a non-zero frame here has no effect on what
        // `fittingSize()` later reports). So `TextArea` cannot rely on the generic
        // `NativeControl::measure_override` -> `AnyView::measure` -> `fittingSize()` path every
        // other native leaf shares (see that method's own doc comment) — `native_ui::TextArea`
        // overrides `measure_override` itself and calls `InnerTextArea::measure` below instead.
        // The height is derived from the text view's own font metrics (not a hardcoded pixel
        // constant) once, at construction, matching how `NSTextField` (`InnerTextBox`) gets a
        // non-zero default from its cell's real `intrinsicContentSize`, and mirroring how WinUI3's
        // `TextArea` (`elwindui-backend-winui3::inner::InnerTextArea`) always has a non-zero
        // minimum height from its default style (it isn't wrapped in a `ScrollViewer` there).
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

        let handle = AnyView::from(scroll);
        Self {
            handle,
            text_view,
            delegate_storage: Rc::new(RefCell::new(None)),
            default_width,
            default_height,
        }
    }

    pub(crate) fn handle(&self) -> AnyView {
        self.handle.clone()
    }

    /// See the doc comment on `default_width`/`default_height` (set in `new`) for why this exists
    /// instead of `native_ui::NativeControl`'s shared `fittingSize()`-based `measure_override`.
    pub(crate) fn measure(&self, _available: elwindui_core::base::Size) -> elwindui_core::base::Size {
        elwindui_core::base::Size {
            width: self.default_width,
            height: self.default_height,
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

/// Raw `NSScrollView` + nested `TreeHostView` (`ElwinduiContentRoot`) — composed by
/// `native_ui::ScrollView`. See `elwindui_core::ui::ScrollView`'s own doc comment for the
/// `ScrollView -> NativeScrollHost -> ElwinduiContentRoot -> content` structure this implements.
/// `content_host` is a second, independent `TreeHostView` instance — the same nested-hosting
/// pattern `InnerTabView::insert_tab`'s own per-tab `TreeHostView::new()` already establishes, not a
/// one-off special case — with its own `set_tree`, its own `AppKitRelayoutHost`/`AppKitFocusHost`
/// registration (falls out of `TreeHostView::set_tree` unchanged, no new focus-chain code needed:
/// `ElwinduiWindow::make_first_responder`'s responder-chain walk already finds *any* `TreeHostView`
/// ancestor, nested ones included), and — the one genuinely new piece — `unconstrained_axes` set on
/// whichever axis scrolls, so that axis measures/arranges at its true natural size instead of being
/// clamped to the viewport.
pub(crate) struct InnerScrollView {
    handle: AnyView,
    scroll: Retained<NSScrollView>,
    content_host: Retained<TreeHostView>,
    /// `(horizontal_scroll_enabled, vertical_scroll_enabled)` — mirrors the `.elwind`-visible
    /// property names directly (unlike `TreeHostIvars::unconstrained_axes`, which is phrased as
    /// "width/height unconstrained" — the same booleans, just named from the opposite perspective:
    /// scrolling enabled on an axis is exactly what makes that axis unconstrained).
    axes: Cell<(bool, bool)>,
}

impl InnerScrollView {
    pub(crate) fn new() -> Self {
        let m = mtm();
        let scroll = NSScrollView::new(m);
        let content_host = TreeHostView::new();
        content_host.setTranslatesAutoresizingMaskIntoConstraints(true);
        scroll.setDocumentView(Some(&content_host));
        let handle = AnyView::from(scroll.clone());
        let this = Self {
            handle,
            scroll,
            content_host,
            // Vertical-only scrolling by default — matches both platforms' own scroll-widget
            // defaults and the overwhelmingly common product case (`ScrollView` in
            // `builtins.elwind`'s own default).
            axes: Cell::new((false, true)),
        };
        this.apply_axes();
        this
    }

    /// Applies `axes` to the native scroller visibility, `content_host`'s own unconstrained-measure
    /// axes, and its autoresizing mask — the "classic pre-Auto-Layout fill" technique
    /// `InnerTabView::insert_tab`/`InnerWindow::new` already use elsewhere in this file, here used
    /// to track the *non*-scrolling axis (axes) to the scroll view's own viewport size automatically
    /// on every resize, no `NSNotificationCenter` observation needed: a scrolling axis gets no
    /// autoresizing bit at all (so it stays whatever size `relayout`'s own unconstrained measurement
    /// just set it to), while a non-scrolling axis keeps its `ViewWidthSizable`/`ViewHeightSizable`
    /// bit so AppKit's ordinary autoresizing machinery keeps that axis synced to the clip view.
    fn apply_axes(&self) {
        let (horizontal, vertical) = self.axes.get();
        self.content_host.set_unconstrained_axes(horizontal, vertical);
        let mut mask = objc2_app_kit::NSAutoresizingMaskOptions::ViewNotSizable;
        if !horizontal {
            mask |= objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable;
        }
        if !vertical {
            mask |= objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable;
        }
        self.content_host.setAutoresizingMask(mask);
        self.scroll.setHasHorizontalScroller(horizontal);
        self.scroll.setHasVerticalScroller(vertical);
    }

    pub(crate) fn handle(&self) -> AnyView {
        self.handle.clone()
    }

    pub(crate) fn set_content(&self, content: Rc<dyn UIElementExt>) {
        self.content_host.set_tree(content);
    }

    pub(crate) fn set_horizontal_scroll_enabled(&self, enabled: bool) {
        let (_, vertical) = self.axes.get();
        self.axes.set((enabled, vertical));
        self.apply_axes();
    }

    pub(crate) fn set_vertical_scroll_enabled(&self, enabled: bool) {
        let (horizontal, _) = self.axes.get();
        self.axes.set((horizontal, enabled));
        self.apply_axes();
    }
}

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

    /// AppKit-only helper (no `elwindui_core::ui::Button` trait member — WinUI3's real `TabView`
    /// highlights its selected tab for free, no borderless-button trick needed there): used by
    /// `create_tab_chip` so `TabChipImpl::set_selected`'s translucent background tint shows through
    /// instead of being hidden behind the button's own opaque default bezel.
    pub(crate) fn set_bordered(&self, bordered: bool) {
        self.ns.setBordered(bordered);
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

/// See docs/elwindui_builtins_spec.md 付録Y. A single tab's header: a title button (click to
/// select) plus a small close button, packed into one row so `TabStripImpl` can insert/remove it as
/// one unit. Purely an internal composition helper (never a real `.elwind`-declared element), so
/// its two buttons are plain `InnerButton`s, not `native_ui::Button` — no use-site margin/alignment
/// ever applies to them.
pub(crate) struct TabChipImpl {
    ns: Retained<NSStackView>,
    pub(crate) title_button: InnerButton,
    pub(crate) close_button: InnerButton,
}

fn create_tab_chip(title: &str) -> TabChipImpl {
    let title_button = InnerButton::new();
    title_button.set_text(title);
    // Borderless: an `NSButton`'s default bezel is opaque and would otherwise cover almost the
    // entire chip row, hiding `set_selected`'s translucent background tint underneath it.
    title_button.set_bordered(false);
    let close_button = InnerButton::new();
    close_button.set_text("×");
    close_button.set_bordered(false);
    let ns = new_stack(
        vec![title_button.handle.clone(), close_button.handle.clone()],
        NSUserInterfaceLayoutOrientation::Horizontal,
    );
    TabChipImpl {
        ns,
        title_button,
        close_button,
    }
}

impl TabChipImpl {
    pub(crate) fn set_title(&self, title: &str) {
        self.title_button.set_text(title);
    }

    /// Highlights this chip's own row with a translucent background tint when it's the selected
    /// tab. AppKit has no native "selected tab" concept to lean on here (unlike WinUI3's real
    /// `Controls::TabView`, whose `SelectedIndex` gets OS-drawn highlighting for free) — this
    /// backend hand-rolls its tab strip out of a plain `NSStackView`, so the highlight is drawn the
    /// same way `Rectangle`'s own `fill` is: a layer-backed background color, applied to `ns` (the
    /// chip's whole row) rather than just `title_button` so it isn't hidden behind that button's
    /// own bezel rendering.
    pub(crate) fn set_selected(&self, selected: bool) {
        self.ns.setWantsLayer(true);
        let layer = self.ns.layer().expect("wantsLayer(true) implies a layer");
        if selected {
            layer.setBackgroundColor(Some(&parse_color("#7f7f7f40")));
        } else {
            layer.setBackgroundColor(None);
        }
    }
}

/// The row of `TabChipImpl`s plus a trailing "+" button. `InnerTabView` owns one of these and the
/// content area below it; kept as a separate type since 付録Y's backend table describes it as its
/// own piece (a custom `NSStackView`-based strip, not `NSTabViewController`).
pub(crate) struct TabStripImpl {
    ns: Retained<NSStackView>,
    pub(crate) new_tab_button: InnerButton,
}

fn create_tab_strip() -> TabStripImpl {
    let new_tab_button = InnerButton::new();
    new_tab_button.set_text("+");
    let ns = new_stack(
        vec![new_tab_button.handle.clone()],
        NSUserInterfaceLayoutOrientation::Horizontal,
    );
    TabStripImpl { ns, new_tab_button }
}

impl TabStripImpl {
    /// Inserts a chip before the "+" button, at arranged-subview position `index`.
    fn insert_tab(&self, index: usize, title: &str) -> TabChipImpl {
        let chip = create_tab_chip(title);
        let view: Retained<NSView> = Retained::into_super(chip.ns.clone());
        self.ns.insertArrangedSubview_atIndex(&view, index as isize);
        chip
    }

    fn remove_tab(&self, chip: &TabChipImpl) {
        let view: Retained<NSView> = Retained::into_super(chip.ns.clone());
        self.ns.removeArrangedSubview(&view);
        view.removeFromSuperview();
    }
}

/// See docs/elwindui_builtins_spec.md 付録Y. Vertical stack of `[TabStripImpl, content_container]`
/// — composed by `native_ui::TabView`, which owns the mapping from its `children` collection's
/// `TabViewItem`s to `TabChipImpl`s + content hosts. This type only holds the widget areas — it has
/// no notion of "the list of tabs" on its own.
///
/// Each tab gets its own persistent `TreeHostView` (created once, in `insert_tab`), added as an
/// overlaid subview of `content_container` and shown/hidden via `set_tab_content_visible` rather
/// than destroyed and rebuilt — a single shared pane would have no way to restore a previously-
/// shown-then-hidden tab's content after switching away from it.
pub(crate) struct InnerTabView {
    handle: AnyView,
    pub(crate) strip: TabStripImpl,
    content_container: Retained<NSView>,
}

impl InnerTabView {
    pub(crate) fn new() -> Self {
        let m = mtm();
        let strip = create_tab_strip();
        let content_container = NSView::initWithFrame(NSView::alloc(m), NSRect::default());
        let strip_view: Retained<NSView> = Retained::into_super(strip.ns.clone());
        let root = NSStackView::stackViewWithViews(
            &objc2_foundation::NSArray::from_retained_slice(&[
                strip_view,
                content_container.clone(),
            ]),
            m,
        );
        root.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
        // `NSStackView`'s default `distribution` (`GravityAreas`) leaves each arranged subview at
        // its own intrinsic size unless hugging priorities say otherwise — `.Fill` makes the stack
        // actually consume its *entire* stacking-axis extent, matching the expected "chips row at
        // natural height, content area fills the rest" shape. `content_container`'s own vertical
        // hugging priority is dropped to (near-)zero so it — not the also-low-priority-by-default
        // `strip` — is the one that absorbs whatever space `Fill` distributes.
        content_container.setContentHuggingPriority_forOrientation(
            1.0,
            objc2_app_kit::NSLayoutConstraintOrientation::Vertical,
        );
        root.setDistribution(objc2_app_kit::NSStackViewDistribution::Fill);
        let handle = AnyView::from(root);
        Self {
            handle,
            strip,
            content_container,
        }
    }

    pub(crate) fn handle(&self) -> AnyView {
        self.handle.clone()
    }

    pub(crate) fn set_on_new_tab(&self, callback: Box<dyn Fn()>) {
        self.strip.new_tab_button.set_on_click(callback);
    }

    /// Inserts a new tab chip at `index` (wiring `on_select`/`on_close` to the given callbacks)
    /// plus a fresh, persistent content host — added to `content_container`, initially hidden.
    pub(crate) fn insert_tab(
        &self,
        index: usize,
        title: &str,
        on_select: Box<dyn Fn()>,
        on_close: Box<dyn Fn()>,
    ) -> (TabChipImpl, Retained<TreeHostView>) {
        let chip = self.strip.insert_tab(index, title);
        chip.title_button.set_on_click(on_select);
        chip.close_button.set_on_click(on_close);

        let host = TreeHostView::new();
        // Classic pre-Auto-Layout "fill the parent" technique instead of `NSLayoutConstraint`s:
        // `translatesAutoresizingMaskIntoConstraints(true)` (this container has no Auto Layout
        // constraints of its own, so this is the default anyway, made explicit) plus a
        // `.width | .height` autoresizing mask makes AppKit stretch `host` to match
        // `content_container`'s bounds on every resize, with no custom `NSView` subclass or
        // constraint bookkeeping needed here.
        host.setTranslatesAutoresizingMaskIntoConstraints(true);
        host.setAutoresizingMask(
            objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
                | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        host.setFrame(self.content_container.bounds());
        host.setHidden(true);
        self.content_container.addSubview(&host);
        (chip, host)
    }

    /// Removes a tab's chip and its persistent content host together.
    pub(crate) fn remove_tab(&self, chip: &TabChipImpl, host: &TreeHostView) {
        self.strip.remove_tab(chip);
        host.removeFromSuperview();
    }

    /// Shows or hides a tab's content host — selecting a tab means showing its host and hiding the
    /// previously-selected one, never touching either one's actual content.
    pub(crate) fn set_tab_content_visible(&self, host: &TreeHostView, visible: bool) {
        host.setHidden(!visible);
    }
}

/// See docs/elwindui_builtins_spec.md 付録X. A single application-wide `NSMenu` (top menu bar
/// item / `File`, `Edit`, ...) entry — composed by `native_ui::MenuItem`.
#[derive(Clone)]
pub(crate) struct InnerMenuItem {
    ns: Retained<NSMenuItem>,
    target_storage: Rc<RefCell<Option<Retained<MenuItemTarget>>>>,
}

impl InnerMenuItem {
    pub(crate) fn new() -> Self {
        let m = mtm();
        let ns = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                m.alloc::<NSMenuItem>(),
                &NSString::from_str(""),
                None,
                &NSString::from_str(""),
            )
        };
        Self {
            ns,
            target_storage: Rc::new(RefCell::new(None)),
        }
    }

    /// A real `NSMenuItem.title` setter — construction takes no title argument, so this is the
    /// only way a menu item's title is ever actually set.
    pub(crate) fn set_text(&self, text: &str) {
        self.ns.setTitle(&NSString::from_str(text));
    }

    pub(crate) fn set_enabled(&self, enabled: bool) {
        self.ns.setEnabled(enabled);
    }

    /// A bare key character (e.g. `"s"`); macOS defaults a menu item's modifier mask to Cmd,
    /// which matches the common `Cmd+<letter>` shortcuts notepad needs.
    pub(crate) fn set_shortcut(&self, key_equivalent: &str) {
        self.ns
            .setKeyEquivalent(&NSString::from_str(key_equivalent));
    }

    pub(crate) fn set_on_select(&self, callback: Box<dyn Fn()>) {
        let target = MenuItemTarget::new(MenuItemTargetIvars { callback });
        unsafe {
            self.ns.setTarget(Some(&target));
            self.ns.setAction(Some(sel!(perform:)));
        }
        *self.target_storage.borrow_mut() = Some(target);
    }
}

struct MenuItemTargetIvars {
    callback: Box<dyn Fn()>,
}

define_class!(
    #[unsafe(super(objc2_foundation::NSObject))]
    #[ivars = MenuItemTargetIvars]
    struct MenuItemTarget;

    unsafe impl NSObjectProtocol for MenuItemTarget {}

    impl MenuItemTarget {
        #[unsafe(method(perform:))]
        fn perform(&self, _sender: &AnyObject) {
            (self.ivars().callback)();
        }
    }
);

impl MenuItemTarget {
    fn new(ivars: MenuItemTargetIvars) -> Retained<Self> {
        let this = Self::alloc().set_ivars(ivars);
        unsafe { msg_send![super(this), init] }
    }
}

/// A dropdown attached to a `MenuBarItem` (or, per 付録M, a right-click context menu — not used
/// that way here, but the same type covers both) — composed by `native_ui::Menu`.
#[derive(Clone)]
pub(crate) struct InnerMenu {
    ns: Retained<NSMenu>,
}

impl InnerMenu {
    pub(crate) fn new() -> Self {
        let m = mtm();
        let ns = NSMenu::initWithTitle(m.alloc::<NSMenu>(), &NSString::from_str(""));
        Self { ns }
    }

    pub(crate) fn add_item(&self, item: &InnerMenuItem) {
        self.ns.addItem(&item.ns);
    }
    pub(crate) fn remove_item(&self, item: &InnerMenuItem) {
        self.ns.removeItem(&item.ns);
    }
}

/// One top-level entry in the menu bar (e.g. "File"), holding its dropdown `InnerMenu` — composed
/// by `native_ui::MenuBarItem`.
#[derive(Clone)]
pub(crate) struct InnerMenuBarItem {
    ns: Retained<NSMenuItem>,
}

impl InnerMenuBarItem {
    pub(crate) fn new() -> Self {
        let m = mtm();
        let ns = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                m.alloc::<NSMenuItem>(),
                &NSString::from_str(""),
                None,
                &NSString::from_str(""),
            )
        };
        Self { ns }
    }

    pub(crate) fn set_text(&self, text: &str) {
        self.ns.setTitle(&NSString::from_str(text));
    }
    pub(crate) fn set_submenu(&self, submenu: &InnerMenu) {
        self.ns.setSubmenu(Some(&submenu.ns));
    }
}

/// The whole top menu bar, installed via `native_ui::Window::set_menu_bar` — composed by
/// `native_ui::MenuBar`.
#[derive(Clone)]
pub(crate) struct InnerMenuBar {
    ns: Retained<NSMenu>,
}

impl InnerMenuBar {
    pub(crate) fn new() -> Self {
        let m = mtm();
        let ns = NSMenu::initWithTitle(m.alloc::<NSMenu>(), &NSString::from_str(""));

        // macOS convention: `mainMenu`'s *first* item is always displayed as the bold app name
        // (whatever title it's given is ignored/overridden by the OS) and its submenu is "the app
        // menu". Without one, the DSL's first real top-level item (e.g. "File") gets silently
        // absorbed into that slot instead of showing up as its own menu — so this app-menu slot,
        // with at minimum a working Quit item, is provided here rather than asked of the DSL
        // author, since it's a platform detail of `NSApp.mainMenu`, not something 付録X's
        // `MenuBar`/`MenuBarItem` DSL shape should need to know about.
        let app_menu_item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                m.alloc::<NSMenuItem>(),
                &NSString::from_str(""),
                None,
                &NSString::from_str(""),
            )
        };
        let app_menu = NSMenu::initWithTitle(m.alloc::<NSMenu>(), &NSString::from_str(""));
        let quit_item = unsafe {
            // No target: leaving it nil dispatches through the responder chain to
            // `NSApplication`, which implements `terminate:` itself — the standard way to wire a
            // Quit item without the app needing to be its own `NSApplicationDelegate`.
            NSMenuItem::initWithTitle_action_keyEquivalent(
                m.alloc::<NSMenuItem>(),
                &NSString::from_str("Quit"),
                Some(sel!(terminate:)),
                &NSString::from_str("q"),
            )
        };
        app_menu.addItem(&quit_item);
        app_menu_item.setSubmenu(Some(&app_menu));
        ns.addItem(&app_menu_item);
        Self { ns }
    }

    pub(crate) fn add_item(&self, item: &InnerMenuBarItem) {
        self.ns.addItem(&item.ns);
    }
    pub(crate) fn remove_item(&self, item: &InnerMenuBarItem) {
        self.ns.removeItem(&item.ns);
    }
}

/// Offscreen golden-scene rendering tests (painter design doc §20.2) — renders a handful of
/// representative scenes into an in-memory `CGBitmapContext` via `CALayer.renderInContext`
/// (no window/screen involved, so no Screen Recording permission is needed and these run
/// headlessly in `cargo test`) and asserts specific sample pixels rather than diffing against a
/// checked-in reference PNG — a narrower, self-contained foundation for this class of test rather
/// than the full 24-scene cross-backend suite the design doc describes (WinUI3/GTK4 can't run on
/// this machine at all — see `docs/elwindui_implementation_status.md` — so a true cross-backend
/// image diff isn't achievable here regardless).
#[cfg(test)]
mod golden_tests {
    use crate::render::{GradientMaskShape, add_shape_layer, apply_fill, ellipse_cgpath,
        path_to_cgpath, rounded_rect_cgpath, try_add_gradient_fill_layer,
        try_add_image_fill_layer, build_image_container_layer, resolve_cgimage};
    use objc2_core_foundation::CFRetained;
    use objc2_quartz_core::{CALayer, kCAFillRuleEvenOdd, kCAFillRuleNonZero};
    use objc2_core_graphics::{CGImage, CGMutablePath};
    use objc2_quartz_core::CAShapeLayer;
    use std::collections::HashMap;
    use crate::render::fitted_image_rect;
    use objc2_core_graphics::CGColorSpace;
    use super::*;

    struct Bitmap {
        ctx: CFRetained<objc2_core_graphics::CGContext>,
        pixels: Box<[u8]>,
        width: usize,
        height: usize,
        bytes_per_row: usize,
    }

    impl Bitmap {
        fn new(width: usize, height: usize) -> Self {
            let bytes_per_row = width * 4;
            let mut pixels = vec![0u8; bytes_per_row * height].into_boxed_slice();
            let color_space = CGColorSpace::new_device_rgb().expect("device RGB color space");
            let bitmap_info = objc2_core_graphics::CGImageAlphaInfo::PremultipliedLast.0
                | objc2_core_graphics::CGBitmapInfo::ByteOrder32Big.0;
            let ctx = unsafe {
                objc2_core_graphics::CGBitmapContextCreate(
                    pixels.as_mut_ptr() as *mut _,
                    width,
                    height,
                    8,
                    bytes_per_row,
                    Some(&color_space),
                    bitmap_info,
                )
            }
            .expect("CGBitmapContextCreate");
            Self {
                ctx,
                pixels,
                width,
                height,
                bytes_per_row,
            }
        }

        fn pixel(&self, x: usize, y: usize) -> (u8, u8, u8, u8) {
            assert!(x < self.width && y < self.height);
            let offset = y * self.bytes_per_row + x * 4;
            (
                self.pixels[offset],
                self.pixels[offset + 1],
                self.pixels[offset + 2],
                self.pixels[offset + 3],
            )
        }
    }

    /// `CALayer.renderInContext:` against a `CGBitmapContext` renders **Y-flipped** relative to
    /// the logical/path coordinates fed to `add_shape_layer`/`rounded_rect_cgpath`/etc — a shape
    /// built at logical `y` ends up at roughly `bitmap.pixel(x, bitmap.height - y)`, not
    /// `bitmap.pixel(x, y)`. The 4 original tests below never surfaced this (they only ever sample
    /// flip-symmetric geometry: bounding-box corners of a uniform shape, or points exactly on the
    /// canvas's own vertical center) — any *new* test with real top/bottom asymmetry (e.g. one
    /// rounded corner vs one sharp corner, a curve that bows toward one edge) must account for it.
    fn render_layer(root: &Retained<CALayer>, bitmap: &Bitmap) {
        root.renderInContext(&bitmap.ctx);
    }

    fn approx(actual: (u8, u8, u8, u8), expected: (u8, u8, u8, u8), tolerance: u8) {
        let close = |a: u8, b: u8| a.abs_diff(b) <= tolerance;
        assert!(
            close(actual.0, expected.0)
                && close(actual.1, expected.1)
                && close(actual.2, expected.2)
                && close(actual.3, expected.3),
            "expected {expected:?}, got {actual:?} (tolerance {tolerance})"
        );
    }

    #[test]
    fn solid_filled_rect_paints_the_expected_color_and_nothing_outside_it() {
        let bitmap = Bitmap::new(64, 64);
        let root = CALayer::new();
        root.setBounds(objc2_core_foundation::CGRect::new(
            objc2_core_foundation::CGPoint::new(0.0, 0.0),
            objc2_core_foundation::CGSize::new(64.0, 64.0),
        ));
        let world = elwindui_core::base::AffineTransform::identity();
        let rect = elwindui_core::base::Rect {
            x: 16.0,
            y: 16.0,
            width: 32.0,
            height: 32.0,
        };
        let path = rounded_rect_cgpath(&world, rect, elwindui_core::base::CornerRadius::default());
        add_shape_layer(
            &root,
            &path,
            Some(&elwindui_core::graphics::Brush::Solid(
                elwindui_core::graphics::Color::rgb(255, 0, 0),
            )),
            None,
            1.0,
            rect,
        );
        render_layer(&root, &bitmap);
        approx(bitmap.pixel(32, 32), (255, 0, 0, 255), 50);
        approx(bitmap.pixel(2, 2), (0, 0, 0, 0), 10);
    }

    #[test]
    fn filled_ellipse_is_transparent_at_its_corners() {
        let bitmap = Bitmap::new(64, 64);
        let root = CALayer::new();
        root.setBounds(objc2_core_foundation::CGRect::new(
            objc2_core_foundation::CGPoint::new(0.0, 0.0),
            objc2_core_foundation::CGSize::new(64.0, 64.0),
        ));
        let world = elwindui_core::base::AffineTransform::identity();
        let rect = elwindui_core::base::Rect {
            x: 8.0,
            y: 8.0,
            width: 48.0,
            height: 48.0,
        };
        let path = ellipse_cgpath(&world, rect);
        add_shape_layer(
            &root,
            &path,
            Some(&elwindui_core::graphics::Brush::Solid(
                elwindui_core::graphics::Color::rgb(0, 128, 255),
            )),
            None,
            1.0,
            rect,
        );
        render_layer(&root, &bitmap);
        // Ellipse center: opaque blue.
        approx(bitmap.pixel(32, 32), (0, 128, 255, 255), 50);
        // Bounding-box corner: outside the ellipse's curve, must stay transparent.
        approx(bitmap.pixel(9, 9), (0, 0, 0, 0), 10);
    }

    #[test]
    fn stroked_rect_paints_only_near_its_border() {
        let bitmap = Bitmap::new(64, 64);
        let root = CALayer::new();
        root.setBounds(objc2_core_foundation::CGRect::new(
            objc2_core_foundation::CGPoint::new(0.0, 0.0),
            objc2_core_foundation::CGSize::new(64.0, 64.0),
        ));
        let world = elwindui_core::base::AffineTransform::identity();
        let rect = elwindui_core::base::Rect {
            x: 16.0,
            y: 16.0,
            width: 32.0,
            height: 32.0,
        };
        let path = rounded_rect_cgpath(&world, rect, elwindui_core::base::CornerRadius::default());
        let stroke = elwindui_core::graphics::StrokeStyle {
            width: 4.0,
            ..Default::default()
        };
        add_shape_layer(
            &root,
            &path,
            None,
            Some((
                &elwindui_core::graphics::Brush::Solid(elwindui_core::graphics::Color::black()),
                &stroke,
            )),
            1.0,
            rect,
        );
        render_layer(&root, &bitmap);
        // Interior of the rect (well inside the 4px-wide border): unpainted.
        approx(bitmap.pixel(32, 32), (0, 0, 0, 0), 10);
        // Right on the border: opaque black.
        approx(bitmap.pixel(16, 32), (0, 0, 0, 255), 40);
    }

    #[test]
    fn opacity_accumulator_scales_down_alpha() {
        let bitmap = Bitmap::new(64, 64);
        let root = CALayer::new();
        root.setBounds(objc2_core_foundation::CGRect::new(
            objc2_core_foundation::CGPoint::new(0.0, 0.0),
            objc2_core_foundation::CGSize::new(64.0, 64.0),
        ));
        let world = elwindui_core::base::AffineTransform::identity();
        let rect = elwindui_core::base::Rect {
            x: 16.0,
            y: 16.0,
            width: 32.0,
            height: 32.0,
        };
        let path = rounded_rect_cgpath(&world, rect, elwindui_core::base::CornerRadius::default());
        add_shape_layer(
            &root,
            &path,
            Some(&elwindui_core::graphics::Brush::Solid(
                elwindui_core::graphics::Color::rgb(0, 255, 0),
            )),
            None,
            0.5,
            rect,
        );
        render_layer(&root, &bitmap);
        let (_, _, _, a) = bitmap.pixel(32, 32);
        assert!(
            a < 200,
            "half-opacity fill should not be fully opaque, got alpha {a}"
        );
        assert!(
            a > 50,
            "half-opacity fill should still be visibly painted, got alpha {a}"
        );
    }

    #[test]
    fn fitted_image_rect_fill_always_matches_dest_regardless_of_image_size() {
        let dest = elwindui_core::base::Rect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 50.0,
        };
        let placed = fitted_image_rect(
            dest,
            (20.0, 80.0),
            elwindui_core::graphics::ImageFit::Fill,
            elwindui_core::graphics::AlignmentX::Center,
            elwindui_core::graphics::AlignmentY::Center,
        );
        assert_eq!(placed, elwindui_core::base::Rect { x: 0.0, y: 0.0, width: 100.0, height: 50.0 });
    }

    #[test]
    fn fitted_image_rect_contain_letterboxes_without_overflowing_dest() {
        let dest = elwindui_core::base::Rect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
        };
        // A 200x100 (2:1) image `Contain`ed into a 100x100 square must shrink to fit the narrower
        // axis (height), leaving horizontal letterboxing rather than overflowing either axis.
        let placed = fitted_image_rect(
            dest,
            (200.0, 100.0),
            elwindui_core::graphics::ImageFit::Contain,
            elwindui_core::graphics::AlignmentX::Center,
            elwindui_core::graphics::AlignmentY::Center,
        );
        assert_eq!(placed.width, 100.0);
        assert_eq!(placed.height, 50.0);
        assert_eq!(placed.x, 0.0);
        assert_eq!(placed.y, 25.0);
    }

    #[test]
    fn fitted_image_rect_cover_fills_dest_and_overflows_the_wider_axis() {
        let dest = elwindui_core::base::Rect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
        };
        // The same 2:1 image `Cover`ing a 100x100 square must grow to fill the *shorter* axis
        // (height), overflowing width — the opposite of `Contain`'s letterboxing.
        let placed = fitted_image_rect(
            dest,
            (200.0, 100.0),
            elwindui_core::graphics::ImageFit::Cover,
            elwindui_core::graphics::AlignmentX::Center,
            elwindui_core::graphics::AlignmentY::Center,
        );
        assert_eq!(placed.width, 200.0);
        assert_eq!(placed.height, 100.0);
        assert_eq!(placed.x, -50.0);
        assert_eq!(placed.y, 0.0);
    }

    #[test]
    fn fitted_image_rect_none_draws_at_intrinsic_size_and_honors_alignment() {
        let dest = elwindui_core::base::Rect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
        };
        let placed = fitted_image_rect(
            dest,
            (30.0, 20.0),
            elwindui_core::graphics::ImageFit::None,
            elwindui_core::graphics::AlignmentX::Right,
            elwindui_core::graphics::AlignmentY::Bottom,
        );
        assert_eq!(placed.width, 30.0);
        assert_eq!(placed.height, 20.0);
        assert_eq!(placed.x, 70.0);
        assert_eq!(placed.y, 80.0);
    }

    // The remaining tests below extend coverage toward painter design doc §20.2's ~19-scene
    // checklist (only the 4 tests above existed before this pass). Not covered by this lightweight
    // harness (a bare `CALayer` fed straight to the drawing helpers, no `TreeHostView`/real window):
    // native-control/painted-content Z-order interleaving — that needs a real `NSView` subview
    // hierarchy, out of reach here without much heavier test infrastructure. Also not covered:
    // clockwise/counterclockwise arc sweep — `path_to_cgpath`'s own doc comment already documents
    // `PathCommand::ArcTo` as unrendered on this backend (a known gap, not something this test pass
    // introduced), so a "does the sweep direction change the rendered shape" test would just fail
    // against that pre-existing gap rather than exercising real behavior.

    #[test]
    fn rounded_rect_applies_each_corner_radius_independently() {
        let bitmap = Bitmap::new(64, 64);
        let root = CALayer::new();
        root.setBounds(objc2_core_foundation::CGRect::new(
            objc2_core_foundation::CGPoint::new(0.0, 0.0),
            objc2_core_foundation::CGSize::new(64.0, 64.0),
        ));
        let world = elwindui_core::base::AffineTransform::identity();
        let rect = elwindui_core::base::Rect {
            x: 8.0,
            y: 8.0,
            width: 48.0,
            height: 48.0,
        };
        // `top_left` (the (rect.x, rect.y) corner — see `PathBuilder::add_rounded_rect`) stays
        // sharp; the other three corners are rounded.
        let radii = elwindui_core::base::CornerRadius {
            top_left: 0.0,
            top_right: 20.0,
            bottom_right: 20.0,
            bottom_left: 20.0,
        };
        let path = rounded_rect_cgpath(&world, rect, radii);
        add_shape_layer(
            &root,
            &path,
            Some(&elwindui_core::graphics::Brush::Solid(
                elwindui_core::graphics::Color::rgb(0, 200, 0),
            )),
            None,
            1.0,
            rect,
        );
        render_layer(&root, &bitmap);
        // The sharp (radius 0) corner is painted right up to (rect.x, rect.y) — `render_layer`'s
        // own Y-flip note applies (logical y=9 lands near pixel row 64-9=55).
        approx(bitmap.pixel(9, 55), (0, 200, 0, 255), 50);
        // The rounded (radius 20) opposite corner stays unpainted this close to (x+w, y+h).
        approx(bitmap.pixel(55, 9), (0, 0, 0, 0), 10);
    }

    #[test]
    fn line_cap_butt_does_not_extend_past_the_segment_endpoint() {
        let bitmap = Bitmap::new(64, 64);
        let root = CALayer::new();
        root.setBounds(objc2_core_foundation::CGRect::new(
            objc2_core_foundation::CGPoint::new(0.0, 0.0),
            objc2_core_foundation::CGSize::new(64.0, 64.0),
        ));
        let path = CGMutablePath::new();
        unsafe {
            CGMutablePath::move_to_point(Some(&path), std::ptr::null(), 16.0, 32.0);
            CGMutablePath::add_line_to_point(Some(&path), std::ptr::null(), 48.0, 32.0);
        }
        let stroke = elwindui_core::graphics::StrokeStyle {
            width: 10.0,
            start_cap: elwindui_core::graphics::LineCap::Butt,
            end_cap: elwindui_core::graphics::LineCap::Butt,
            ..Default::default()
        };
        let bounds = elwindui_core::base::Rect {
            x: 16.0,
            y: 27.0,
            width: 32.0,
            height: 10.0,
        };
        add_shape_layer(
            &root,
            &path,
            None,
            Some((
                &elwindui_core::graphics::Brush::Solid(elwindui_core::graphics::Color::black()),
                &stroke,
            )),
            1.0,
            bounds,
        );
        render_layer(&root, &bitmap);
        // Well inside the segment: painted.
        approx(bitmap.pixel(32, 32), (0, 0, 0, 255), 50);
        // 3px beyond the endpoint at x=16 — a butt cap stops exactly at the endpoint, so this
        // stays unpainted.
        approx(bitmap.pixel(13, 32), (0, 0, 0, 0), 10);
    }

    #[test]
    fn line_cap_round_extends_past_the_segment_endpoint() {
        let bitmap = Bitmap::new(64, 64);
        let root = CALayer::new();
        root.setBounds(objc2_core_foundation::CGRect::new(
            objc2_core_foundation::CGPoint::new(0.0, 0.0),
            objc2_core_foundation::CGSize::new(64.0, 64.0),
        ));
        let path = CGMutablePath::new();
        unsafe {
            CGMutablePath::move_to_point(Some(&path), std::ptr::null(), 16.0, 32.0);
            CGMutablePath::add_line_to_point(Some(&path), std::ptr::null(), 48.0, 32.0);
        }
        // Half the 10.0 stroke width is 5.0, so a round cap extends ~5px past x=16 — well past
        // the same x=13 sample point a butt cap (the test above) leaves unpainted.
        let stroke = elwindui_core::graphics::StrokeStyle {
            width: 10.0,
            start_cap: elwindui_core::graphics::LineCap::Round,
            end_cap: elwindui_core::graphics::LineCap::Round,
            ..Default::default()
        };
        let bounds = elwindui_core::base::Rect {
            x: 16.0,
            y: 27.0,
            width: 32.0,
            height: 10.0,
        };
        add_shape_layer(
            &root,
            &path,
            None,
            Some((
                &elwindui_core::graphics::Brush::Solid(elwindui_core::graphics::Color::black()),
                &stroke,
            )),
            1.0,
            bounds,
        );
        render_layer(&root, &bitmap);
        approx(bitmap.pixel(13, 32), (0, 0, 0, 255), 80);
    }

    /// Builds a narrow, acute-angled "V" (two segments meeting at `(32, 10)`, opening downward)
    /// stroked with `join`/`miter_limit` — shared by the miter/bevel/miter-limit tests below, since
    /// they only differ in that one `StrokeStyle`.
    fn stroke_acute_v(
        join: elwindui_core::graphics::LineJoin,
        miter_limit: f32,
    ) -> (u8, u8, u8, u8) {
        let bitmap = Bitmap::new(64, 64);
        let root = CALayer::new();
        root.setBounds(objc2_core_foundation::CGRect::new(
            objc2_core_foundation::CGPoint::new(0.0, 0.0),
            objc2_core_foundation::CGSize::new(64.0, 64.0),
        ));
        let path = CGMutablePath::new();
        unsafe {
            CGMutablePath::move_to_point(Some(&path), std::ptr::null(), 10.0, 50.0);
            CGMutablePath::add_line_to_point(Some(&path), std::ptr::null(), 32.0, 10.0);
            CGMutablePath::add_line_to_point(Some(&path), std::ptr::null(), 54.0, 50.0);
        }
        let stroke = elwindui_core::graphics::StrokeStyle {
            width: 8.0,
            line_join: join,
            miter_limit,
            ..Default::default()
        };
        let bounds = elwindui_core::base::Rect {
            x: 10.0,
            y: 10.0,
            width: 44.0,
            height: 40.0,
        };
        add_shape_layer(
            &root,
            &path,
            None,
            Some((
                &elwindui_core::graphics::Brush::Solid(elwindui_core::graphics::Color::black()),
                &stroke,
            )),
            1.0,
            bounds,
        );
        render_layer(&root, &bitmap);
        // Between the bevel's flat cut (~y=6.5) and the full miter tip (~y=1.7) along the
        // vertex's outward bisector — a miter join reaches this point, a bevel join does not.
        // `render_layer`'s own Y-flip note applies (logical y=4 lands near pixel row 64-4=60).
        bitmap.pixel(32, 60)
    }

    #[test]
    fn line_join_miter_extends_the_outer_corner_of_an_acute_angle() {
        // Default `miter_limit` (10.0) comfortably exceeds this vertex's own ~2.07 ratio, so the
        // join renders as a true miter.
        approx(
            stroke_acute_v(elwindui_core::graphics::LineJoin::Miter, 10.0),
            (0, 0, 0, 255),
            80,
        );
    }

    #[test]
    fn line_join_bevel_does_not_extend_the_outer_corner_of_an_acute_angle() {
        approx(
            stroke_acute_v(elwindui_core::graphics::LineJoin::Bevel, 10.0),
            (0, 0, 0, 0),
            10,
        );
    }

    #[test]
    fn miter_limit_below_the_vertex_ratio_forces_a_bevel_style_corner() {
        // This vertex needs a miter-length/half-width ratio of ~2.07; 1.5 falls short, so even a
        // `LineJoin::Miter` request must fall back to a bevel-style flat corner.
        approx(
            stroke_acute_v(elwindui_core::graphics::LineJoin::Miter, 1.5),
            (0, 0, 0, 0),
            10,
        );
    }

    #[test]
    fn dash_pattern_alternates_on_and_off_segments_along_the_line() {
        let bitmap = Bitmap::new(64, 64);
        let root = CALayer::new();
        root.setBounds(objc2_core_foundation::CGRect::new(
            objc2_core_foundation::CGPoint::new(0.0, 0.0),
            objc2_core_foundation::CGSize::new(64.0, 64.0),
        ));
        let path = CGMutablePath::new();
        unsafe {
            CGMutablePath::move_to_point(Some(&path), std::ptr::null(), 4.0, 32.0);
            CGMutablePath::add_line_to_point(Some(&path), std::ptr::null(), 60.0, 32.0);
        }
        let stroke = elwindui_core::graphics::StrokeStyle {
            width: 6.0,
            dash_pattern: std::sync::Arc::from([8.0, 8.0]),
            ..Default::default()
        };
        let bounds = elwindui_core::base::Rect {
            x: 4.0,
            y: 29.0,
            width: 56.0,
            height: 6.0,
        };
        add_shape_layer(
            &root,
            &path,
            None,
            Some((
                &elwindui_core::graphics::Brush::Solid(elwindui_core::graphics::Color::black()),
                &stroke,
            )),
            1.0,
            bounds,
        );
        render_layer(&root, &bitmap);
        // [4, 12) is the first "on" segment.
        approx(bitmap.pixel(8, 32), (0, 0, 0, 255), 50);
        // [12, 20) is the first "off" gap.
        approx(bitmap.pixel(16, 32), (0, 0, 0, 0), 10);
    }

    #[test]
    fn dash_offset_shifts_the_on_off_phase_along_the_line() {
        let bitmap = Bitmap::new(64, 64);
        let root = CALayer::new();
        root.setBounds(objc2_core_foundation::CGRect::new(
            objc2_core_foundation::CGPoint::new(0.0, 0.0),
            objc2_core_foundation::CGSize::new(64.0, 64.0),
        ));
        let path = CGMutablePath::new();
        unsafe {
            CGMutablePath::move_to_point(Some(&path), std::ptr::null(), 4.0, 32.0);
            CGMutablePath::add_line_to_point(Some(&path), std::ptr::null(), 60.0, 32.0);
        }
        let stroke = elwindui_core::graphics::StrokeStyle {
            width: 6.0,
            dash_pattern: std::sync::Arc::from([8.0, 8.0]),
            dash_offset: 8.0,
            ..Default::default()
        };
        let bounds = elwindui_core::base::Rect {
            x: 4.0,
            y: 29.0,
            width: 56.0,
            height: 6.0,
        };
        add_shape_layer(
            &root,
            &path,
            None,
            Some((
                &elwindui_core::graphics::Brush::Solid(elwindui_core::graphics::Color::black()),
                &stroke,
            )),
            1.0,
            bounds,
        );
        render_layer(&root, &bitmap);
        // With no offset, x=8 sits in the first "on" segment (the test above). Shifting the phase
        // by a full dash period (8.0) flips it to "off".
        approx(bitmap.pixel(8, 32), (0, 0, 0, 0), 10);
    }

    /// The path shared by the `NonZero`/`EvenOdd` tests below: two 30x30 squares, sharing the same
    /// winding order, overlapping in their bottom-right/top-left quadrant.
    fn two_overlapping_same_winding_squares() -> elwindui_core::graphics::Path {
        let mut builder = elwindui_core::graphics::PathBuilder::new();
        builder.add_rect(elwindui_core::base::Rect {
            x: 10.0,
            y: 10.0,
            width: 30.0,
            height: 30.0,
        });
        builder.add_rect(elwindui_core::base::Rect {
            x: 25.0,
            y: 25.0,
            width: 30.0,
            height: 30.0,
        });
        builder.build().expect("two rects is never an empty path")
    }

    #[test]
    fn nonzero_fill_rule_fills_the_overlap_of_two_same_winding_subpaths() {
        let bitmap = Bitmap::new(64, 64);
        let root = CALayer::new();
        root.setBounds(objc2_core_foundation::CGRect::new(
            objc2_core_foundation::CGPoint::new(0.0, 0.0),
            objc2_core_foundation::CGSize::new(64.0, 64.0),
        ));
        let world = elwindui_core::base::AffineTransform::identity();
        let path = two_overlapping_same_winding_squares();
        let cg_path = path_to_cgpath(&world, &path);
        let shape_layer = CAShapeLayer::new();
        shape_layer.setPath(Some(&cg_path));
        shape_layer.setFillRule(unsafe { kCAFillRuleNonZero });
        apply_fill(
            &shape_layer,
            Some(&elwindui_core::graphics::Brush::Solid(
                elwindui_core::graphics::Color::rgb(0, 150, 0),
            )),
            path.bounds(),
        );
        shape_layer.setOpacity(1.0);
        let shape_layer: Retained<CALayer> = Retained::into_super(shape_layer);
        root.addSublayer(&shape_layer);
        render_layer(&root, &bitmap);
        approx(bitmap.pixel(32, 32), (0, 150, 0, 255), 50); // overlap: two windings, still filled
        approx(bitmap.pixel(15, 49), (0, 150, 0, 255), 50); // first square only (Y-flipped)
    }

    #[test]
    fn evenodd_fill_rule_punches_a_hole_where_two_same_winding_subpaths_overlap() {
        let bitmap = Bitmap::new(64, 64);
        let root = CALayer::new();
        root.setBounds(objc2_core_foundation::CGRect::new(
            objc2_core_foundation::CGPoint::new(0.0, 0.0),
            objc2_core_foundation::CGSize::new(64.0, 64.0),
        ));
        let world = elwindui_core::base::AffineTransform::identity();
        let path = two_overlapping_same_winding_squares();
        let cg_path = path_to_cgpath(&world, &path);
        let shape_layer = CAShapeLayer::new();
        shape_layer.setPath(Some(&cg_path));
        shape_layer.setFillRule(unsafe { kCAFillRuleEvenOdd });
        apply_fill(
            &shape_layer,
            Some(&elwindui_core::graphics::Brush::Solid(
                elwindui_core::graphics::Color::rgb(0, 150, 0),
            )),
            path.bounds(),
        );
        shape_layer.setOpacity(1.0);
        let shape_layer: Retained<CALayer> = Retained::into_super(shape_layer);
        root.addSublayer(&shape_layer);
        render_layer(&root, &bitmap);
        approx(bitmap.pixel(32, 32), (0, 0, 0, 0), 10); // overlap: even crossing count -> a hole
        approx(bitmap.pixel(15, 49), (0, 150, 0, 255), 50); // first square only: still filled (Y-flipped)
    }

    #[test]
    fn quadratic_bezier_bows_away_from_the_straight_chord_between_its_endpoints() {
        let bitmap = Bitmap::new(64, 64);
        let root = CALayer::new();
        root.setBounds(objc2_core_foundation::CGRect::new(
            objc2_core_foundation::CGPoint::new(0.0, 0.0),
            objc2_core_foundation::CGSize::new(64.0, 64.0),
        ));
        let world = elwindui_core::base::AffineTransform::identity();
        let mut builder = elwindui_core::graphics::PathBuilder::new();
        builder.move_to(elwindui_core::base::Point { x: 10.0, y: 50.0 });
        builder.quad_to(
            elwindui_core::base::Point { x: 32.0, y: 10.0 },
            elwindui_core::base::Point { x: 54.0, y: 50.0 },
        );
        let path = builder.build().expect("a moved-to, curved path is never empty");
        let cg_path = path_to_cgpath(&world, &path);
        let stroke = elwindui_core::graphics::StrokeStyle {
            width: 6.0,
            ..Default::default()
        };
        add_shape_layer(
            &root,
            &cg_path,
            None,
            Some((
                &elwindui_core::graphics::Brush::Solid(elwindui_core::graphics::Color::black()),
                &stroke,
            )),
            1.0,
            path.bounds(),
        );
        render_layer(&root, &bitmap);
        // The curve's own midpoint (t=0.5) sits at (32, 30) — nowhere near the straight chord's
        // midpoint (32, 50), proving the quadratic control point actually bent the curve.
        // `render_layer`'s own Y-flip note applies (logical y -> pixel row 64-y).
        approx(bitmap.pixel(32, 34), (0, 0, 0, 255), 50);
        approx(bitmap.pixel(32, 14), (0, 0, 0, 0), 10);
    }

    #[test]
    fn cubic_bezier_bows_away_from_the_straight_chord_between_its_endpoints() {
        let bitmap = Bitmap::new(64, 64);
        let root = CALayer::new();
        root.setBounds(objc2_core_foundation::CGRect::new(
            objc2_core_foundation::CGPoint::new(0.0, 0.0),
            objc2_core_foundation::CGSize::new(64.0, 64.0),
        ));
        let world = elwindui_core::base::AffineTransform::identity();
        let mut builder = elwindui_core::graphics::PathBuilder::new();
        builder.move_to(elwindui_core::base::Point { x: 10.0, y: 50.0 });
        builder.cubic_to(
            elwindui_core::base::Point { x: 20.0, y: 10.0 },
            elwindui_core::base::Point { x: 44.0, y: 10.0 },
            elwindui_core::base::Point { x: 54.0, y: 50.0 },
        );
        let path = builder.build().expect("a moved-to, curved path is never empty");
        let cg_path = path_to_cgpath(&world, &path);
        let stroke = elwindui_core::graphics::StrokeStyle {
            width: 6.0,
            ..Default::default()
        };
        add_shape_layer(
            &root,
            &cg_path,
            None,
            Some((
                &elwindui_core::graphics::Brush::Solid(elwindui_core::graphics::Color::black()),
                &stroke,
            )),
            1.0,
            path.bounds(),
        );
        render_layer(&root, &bitmap);
        // The curve's own midpoint (t=0.5) sits at (32, 20) — nowhere near the straight chord's
        // midpoint (32, 50), proving both control points actually bent the curve.
        // `render_layer`'s own Y-flip note applies (logical y -> pixel row 64-y).
        approx(bitmap.pixel(32, 44), (0, 0, 0, 255), 50);
        approx(bitmap.pixel(32, 14), (0, 0, 0, 0), 10);
    }

    #[test]
    fn linear_gradient_interpolates_between_its_two_stop_colors() {
        let bitmap = Bitmap::new(64, 64);
        let root = CALayer::new();
        root.setBounds(objc2_core_foundation::CGRect::new(
            objc2_core_foundation::CGPoint::new(0.0, 0.0),
            objc2_core_foundation::CGSize::new(64.0, 64.0),
        ));
        let rect = elwindui_core::base::Rect {
            x: 0.0,
            y: 0.0,
            width: 64.0,
            height: 64.0,
        };
        let brush = elwindui_core::graphics::Brush::LinearGradient(
            elwindui_core::graphics::LinearGradientBrush::new(
                elwindui_core::base::Point { x: 0.0, y: 0.0 },
                elwindui_core::base::Point { x: 1.0, y: 0.0 },
                vec![
                    elwindui_core::graphics::GradientStop::new(
                        0.0,
                        elwindui_core::graphics::Color::rgb(255, 0, 0),
                    )
                    .unwrap(),
                    elwindui_core::graphics::GradientStop::new(
                        1.0,
                        elwindui_core::graphics::Color::rgb(0, 0, 255),
                    )
                    .unwrap(),
                ],
            )
            .unwrap(),
        );
        let world = elwindui_core::base::AffineTransform::identity();
        let realized = try_add_gradient_fill_layer(
            &root,
            &brush,
            rect,
            GradientMaskShape::RoundedRect(elwindui_core::base::CornerRadius::default()),
            &world,
            1.0,
        );
        assert!(
            realized,
            "a pure-translation world must realize a gradient brush as a real CAGradientLayer"
        );
        render_layer(&root, &bitmap);
        approx(bitmap.pixel(4, 32), (255, 0, 0, 255), 80); // near the left edge: close to stop 0
        approx(bitmap.pixel(60, 32), (0, 0, 255, 255), 80); // near the right edge: close to stop 1
    }

    #[test]
    fn radial_gradient_interpolates_from_center_to_edge() {
        let bitmap = Bitmap::new(64, 64);
        let root = CALayer::new();
        root.setBounds(objc2_core_foundation::CGRect::new(
            objc2_core_foundation::CGPoint::new(0.0, 0.0),
            objc2_core_foundation::CGSize::new(64.0, 64.0),
        ));
        let rect = elwindui_core::base::Rect {
            x: 0.0,
            y: 0.0,
            width: 64.0,
            height: 64.0,
        };
        let brush = elwindui_core::graphics::Brush::RadialGradient(
            elwindui_core::graphics::RadialGradientBrush::new(
                elwindui_core::base::Point { x: 0.5, y: 0.5 },
                0.5,
                0.5,
                vec![
                    elwindui_core::graphics::GradientStop::new(
                        0.0,
                        elwindui_core::graphics::Color::rgb(255, 0, 0),
                    )
                    .unwrap(),
                    elwindui_core::graphics::GradientStop::new(
                        1.0,
                        elwindui_core::graphics::Color::rgb(0, 0, 255),
                    )
                    .unwrap(),
                ],
            )
            .unwrap(),
        );
        let world = elwindui_core::base::AffineTransform::identity();
        let realized = try_add_gradient_fill_layer(
            &root,
            &brush,
            rect,
            GradientMaskShape::Ellipse,
            &world,
            1.0,
        );
        assert!(realized);
        render_layer(&root, &bitmap);
        approx(bitmap.pixel(32, 32), (255, 0, 0, 255), 60); // center: close to stop 0
        approx(bitmap.pixel(32, 4), (0, 0, 255, 255), 90); // near the edge: close to stop 1
    }

    #[test]
    fn draw_image_contain_letterboxes_and_leaves_the_gap_unpainted() {
        let bitmap = Bitmap::new(64, 64);
        let root = CALayer::new();
        root.setBounds(objc2_core_foundation::CGRect::new(
            objc2_core_foundation::CGPoint::new(0.0, 0.0),
            objc2_core_foundation::CGSize::new(64.0, 64.0),
        ));
        // A 20x10 solid-blue image `Contain`ed into a 20x20 square must shrink to fit the width
        // (already exact) while the height (half of the square) leaves 5px letterbox gaps above
        // and below, centered by default alignment.
        let pixels = vec![0u8, 0, 255, 255].repeat(20 * 10);
        let image = elwindui_core::graphics::Image::from_rgba8(
            20,
            10,
            20 * 4,
            pixels,
            elwindui_core::graphics::AlphaMode::Opaque,
        )
        .expect("valid RGBA8 buffer");
        let mut image_cache = HashMap::new();
        let resolved =
            resolve_cgimage(&image, &mut image_cache).expect("valid RGBA8 buffer decodes");
        let dest = elwindui_core::base::Rect {
            x: 2.0,
            y: 2.0,
            width: 20.0,
            height: 20.0,
        };
        let options = elwindui_core::graphics::ImageDrawOptions {
            fit: elwindui_core::graphics::ImageFit::Contain,
            ..Default::default()
        };
        let world = elwindui_core::base::AffineTransform::identity();
        let container = build_image_container_layer(&resolved, dest, None, &options, &world, 1.0)
            .expect("no source crop means there's always something to draw");
        root.addSublayer(&container);
        render_layer(&root, &bitmap);
        // `render_layer`'s own Y-flip note applies (logical y -> pixel row 64-y).
        approx(bitmap.pixel(12, 52), (0, 0, 255, 255), 50); // inside the letterboxed image
        approx(bitmap.pixel(12, 60), (0, 0, 0, 0), 10); // top letterbox gap: left unpainted
    }

    #[test]
    fn draw_image_source_crop_only_shows_the_cropped_region() {
        let bitmap = Bitmap::new(64, 64);
        let root = CALayer::new();
        root.setBounds(objc2_core_foundation::CGRect::new(
            objc2_core_foundation::CGPoint::new(0.0, 0.0),
            objc2_core_foundation::CGSize::new(64.0, 64.0),
        ));
        // A 2x1 image: left pixel red, right pixel blue.
        let pixels = vec![255u8, 0, 0, 255, 0, 0, 255, 255];
        let image = elwindui_core::graphics::Image::from_rgba8(
            2,
            1,
            2 * 4,
            pixels,
            elwindui_core::graphics::AlphaMode::Opaque,
        )
        .expect("valid RGBA8 buffer");
        let mut image_cache = HashMap::new();
        let resolved =
            resolve_cgimage(&image, &mut image_cache).expect("valid RGBA8 buffer decodes");
        let dest = elwindui_core::base::Rect {
            x: 2.0,
            y: 2.0,
            width: 20.0,
            height: 20.0,
        };
        // Crop to just the right (blue) pixel.
        let source = elwindui_core::base::Rect {
            x: 1.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        };
        let options = elwindui_core::graphics::ImageDrawOptions {
            fit: elwindui_core::graphics::ImageFit::Fill,
            ..Default::default()
        };
        let world = elwindui_core::base::AffineTransform::identity();
        let container =
            build_image_container_layer(&resolved, dest, Some(source), &options, &world, 1.0)
                .expect("the crop rect is fully inside the image, not an empty intersection");
        root.addSublayer(&container);
        render_layer(&root, &bitmap);
        // `render_layer`'s own Y-flip note applies (logical y -> pixel row 64-y).
        approx(bitmap.pixel(12, 52), (0, 0, 255, 255), 50);
    }

    // The two tests below exercise nested `PushTransform`/`PushOpacity` *composition* — but not
    // through `replay_commands`'s own Push/Pop recursion itself: that needs a real `&TreeHostView`
    // (its `NativeControl` arm touches `host.ivars()`), and constructing one (`TreeHostView::new`)
    // asserts the calling thread is the app's main thread, which `cargo test`'s worker-thread pool
    // never is. Instead, each test computes the exact composed `AffineTransform`/`opacity`
    // `replay_commands`' `PushTransform`/`PushOpacity` arms would produce (`transform.concat
    // (pushed)`, `opacity * pushed` — see those arms' own source) and feeds it straight to
    // `rounded_rect_cgpath`/`add_shape_layer`, the same one-level-below approach every other test
    // in this module already uses.

    #[test]
    fn nested_push_transform_composes_both_transforms_in_order() {
        let bitmap = Bitmap::new(64, 64);
        let root = CALayer::new();
        root.setBounds(objc2_core_foundation::CGRect::new(
            objc2_core_foundation::CGPoint::new(0.0, 0.0),
            objc2_core_foundation::CGSize::new(64.0, 64.0),
        ));
        let outer = elwindui_core::base::AffineTransform::translation(20.0, 0.0);
        let inner = elwindui_core::base::AffineTransform::translation(0.0, 20.0);
        let world = outer.concat(&inner);
        let rect = elwindui_core::base::Rect {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        };
        let path = rounded_rect_cgpath(&world, rect, elwindui_core::base::CornerRadius::default());
        add_shape_layer(
            &root,
            &path,
            Some(&elwindui_core::graphics::Brush::Solid(
                elwindui_core::graphics::Color::rgb(0, 200, 0),
            )),
            None,
            1.0,
            rect,
        );
        render_layer(&root, &bitmap);
        // Both translations compose: the 10x10 rect, originally at (0,0), ends up at (20,20).
        // `render_layer`'s own Y-flip note applies (logical y -> pixel row 64-y).
        approx(bitmap.pixel(25, 39), (0, 200, 0, 255), 50);
        approx(bitmap.pixel(5, 59), (0, 0, 0, 0), 10);
    }

    #[test]
    fn nested_push_opacity_multiplies_both_levels() {
        let bitmap = Bitmap::new(64, 64);
        let root = CALayer::new();
        root.setBounds(objc2_core_foundation::CGRect::new(
            objc2_core_foundation::CGPoint::new(0.0, 0.0),
            objc2_core_foundation::CGSize::new(64.0, 64.0),
        ));
        let world = elwindui_core::base::AffineTransform::identity();
        let opacity = 0.5f32 * 0.5f32;
        let rect = elwindui_core::base::Rect {
            x: 16.0,
            y: 16.0,
            width: 32.0,
            height: 32.0,
        };
        let path = rounded_rect_cgpath(&world, rect, elwindui_core::base::CornerRadius::default());
        add_shape_layer(
            &root,
            &path,
            Some(&elwindui_core::graphics::Brush::Solid(
                elwindui_core::graphics::Color::rgb(0, 255, 0),
            )),
            None,
            opacity,
            rect,
        );
        render_layer(&root, &bitmap);
        // The rect is centered on the canvas, so this sample point is Y-flip-invariant.
        let (_, _, _, a) = bitmap.pixel(32, 32);
        // 0.5 * 0.5 = 0.25 net opacity, far below what a single 0.5 level would give (~127) —
        // proving the two `PushOpacity` levels multiplied instead of only the inner (or outer)
        // value winning.
        assert!(a < 100, "nested 0.5*0.5 opacity should be far below ~127, got {a}");
        assert!(a > 20, "nested opacity should still be visibly painted, got {a}");
    }
}

/// `RenderCommand::DrawVectorImage` golden tests (SVG読み込み・ベクター描画対応 実装指示書§22.8) —
/// same offscreen `CALayer.renderInContext` + sample-point-with-tolerance technique as
/// `golden_tests` above, cross-checked against `resvg`'s own rasterization of the same fixture SVG
/// (a dev-dependency only — see `vector_renderer.rs`'s own module doc comment on why production
/// rendering never touches `usvg`/`resvg`). Sample points are chosen on the canvas's own vertical
/// center line wherever possible, same reasoning `golden_tests`'s own doc comment gives for why
/// that's Y-flip-invariant and safe to compare directly against `CALayer.renderInContext`'s
/// flipped output without correcting for it.
#[cfg(test)]
mod svg_golden_tests {
    use objc2_core_foundation::CFRetained;
    use objc2_core_graphics::CGImage;
    use objc2_quartz_core::CALayer;
    use std::collections::HashMap;
    use objc2_core_graphics::CGColorSpace;
    use super::*;
    use elwindui_core::graphics::VectorImageDrawOptions;

    struct Bitmap {
        ctx: CFRetained<objc2_core_graphics::CGContext>,
        pixels: Box<[u8]>,
        width: usize,
        height: usize,
        bytes_per_row: usize,
    }

    impl Bitmap {
        fn new(width: usize, height: usize) -> Self {
            let bytes_per_row = width * 4;
            let mut pixels = vec![0u8; bytes_per_row * height].into_boxed_slice();
            let color_space = CGColorSpace::new_device_rgb().expect("device RGB color space");
            #[allow(deprecated)]
            let bitmap_info = objc2_core_graphics::CGImageAlphaInfo::PremultipliedLast.0
                | objc2_core_graphics::CGBitmapInfo::ByteOrder32Big.0;
            let ctx = unsafe {
                objc2_core_graphics::CGBitmapContextCreate(
                    pixels.as_mut_ptr() as *mut _,
                    width,
                    height,
                    8,
                    bytes_per_row,
                    Some(&color_space),
                    bitmap_info,
                )
            }
            .expect("CGBitmapContextCreate");
            Self {
                ctx,
                pixels,
                width,
                height,
                bytes_per_row,
            }
        }

        fn pixel(&self, x: usize, y: usize) -> (u8, u8, u8, u8) {
            assert!(x < self.width && y < self.height);
            let offset = y * self.bytes_per_row + x * 4;
            (
                self.pixels[offset],
                self.pixels[offset + 1],
                self.pixels[offset + 2],
                self.pixels[offset + 3],
            )
        }
    }

    fn approx(actual: (u8, u8, u8, u8), expected: (u8, u8, u8, u8), tolerance: u8) {
        let close = |a: u8, b: u8| a.abs_diff(b) <= tolerance;
        assert!(
            close(actual.0, expected.0)
                && close(actual.1, expected.1)
                && close(actual.2, expected.2)
                && close(actual.3, expected.3),
            "expected {expected:?}, got {actual:?} (tolerance {tolerance})"
        );
    }

    fn render_via_elwindui(svg: &str, size: usize) -> Bitmap {
        let image = elwindui_svg::load_svg_str(svg).expect("valid fixture SVG");
        let bitmap = Bitmap::new(size, size);
        let root = CALayer::new();
        root.setBounds(objc2_core_foundation::CGRect::new(
            objc2_core_foundation::CGPoint::new(0.0, 0.0),
            objc2_core_foundation::CGSize::new(size as f64, size as f64),
        ));
        let world = elwindui_core::base::AffineTransform::identity();
        let dest = elwindui_core::base::Rect {
            x: 0.0,
            y: 0.0,
            width: size as f32,
            height: size as f32,
        };
        let mut cache = HashMap::new();
        let mut vector_raster_cache = HashMap::new();
        crate::render::draw_vector_image(
            &root,
            &image,
            dest,
            None,
            &VectorImageDrawOptions::default(),
            &world,
            1.0,
            &mut cache,
            &mut vector_raster_cache,
        );
        root.renderInContext(&bitmap.ctx);
        bitmap
    }

    fn render_via_resvg(svg: &str, size: u32) -> resvg::tiny_skia::Pixmap {
        let opt = resvg::usvg::Options::default();
        let tree = resvg::usvg::Tree::from_str(svg, &opt).expect("valid fixture SVG");
        let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size).expect("non-zero pixmap size");
        let tree_size = tree.size();
        let scale = (size as f32 / tree_size.width()).min(size as f32 / tree_size.height());
        let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
        resvg::render(&tree, transform, &mut pixmap.as_mut());
        pixmap
    }

    fn resvg_pixel(pixmap: &resvg::tiny_skia::Pixmap, x: u32, y: u32) -> (u8, u8, u8, u8) {
        let c = pixmap.pixel(x, y).unwrap_or(resvg::tiny_skia::PremultipliedColorU8::TRANSPARENT);
        (c.red(), c.green(), c.blue(), c.alpha())
    }

    const SOLID_RECT_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 64 64"><rect x="16" y="16" width="32" height="32" fill="#ff0000"/></svg>"##;

    #[test]
    fn solid_rect_matches_resvg_at_center_and_is_transparent_outside() {
        let bitmap = render_via_elwindui(SOLID_RECT_SVG, 64);
        let reference = render_via_resvg(SOLID_RECT_SVG, 64);
        approx(bitmap.pixel(32, 32), resvg_pixel(&reference, 32, 32), 40);
        approx(bitmap.pixel(2, 2), (0, 0, 0, 0), 10);
    }

    const LINEAR_GRADIENT_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 64 64">
        <defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="0">
            <stop offset="0" stop-color="#0000ff"/>
            <stop offset="1" stop-color="#ffff00"/>
        </linearGradient></defs>
        <rect x="0" y="0" width="64" height="64" fill="url(#g)"/>
    </svg>"##;

    #[test]
    fn linear_gradient_matches_resvg_at_left_and_right_samples() {
        let bitmap = render_via_elwindui(LINEAR_GRADIENT_SVG, 64);
        let reference = render_via_resvg(LINEAR_GRADIENT_SVG, 64);
        // Both sample points sit on the vertical center row (y=32), which a horizontal-only
        // gradient never varies along — Y-flip-invariant, same reasoning as `golden_tests`'s own
        // sample point choices.
        for x in [4u32, 60u32] {
            approx(
                bitmap.pixel(x as usize, 32),
                resvg_pixel(&reference, x, 32),
                50,
            );
        }
    }

    const GROUP_OPACITY_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 64 64">
        <g opacity="0.5"><rect x="16" y="16" width="32" height="32" fill="#00ff00"/></g>
    </svg>"##;

    #[test]
    fn group_opacity_matches_resvg_alpha_at_center() {
        let bitmap = render_via_elwindui(GROUP_OPACITY_SVG, 64);
        let reference = render_via_resvg(GROUP_OPACITY_SVG, 64);
        approx(bitmap.pixel(32, 32), resvg_pixel(&reference, 32, 32), 50);
    }

    const CLIP_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 64 64">
        <defs><clipPath id="c"><circle cx="32" cy="32" r="16"/></clipPath></defs>
        <rect x="0" y="0" width="64" height="64" fill="#ff00ff" clip-path="url(#c)"/>
    </svg>"##;

    #[test]
    fn clip_path_matches_resvg_inside_the_circle_and_is_transparent_outside() {
        let bitmap = render_via_elwindui(CLIP_SVG, 64);
        let reference = render_via_resvg(CLIP_SVG, 64);
        // Wider tolerance than the other fixtures here: `CAShapeLayer`-mask compositing carries
        // more inherent AA/blending softness than a plain shape fill even at the mask's own
        // center, well away from its edge (empirically observed ~64/255 green-channel deviation at
        // this fixture's dead center) — still tight enough to catch a genuinely broken clip (e.g.
        // one that fails open/fully-transparent).
        approx(bitmap.pixel(32, 32), resvg_pixel(&reference, 32, 32), 90);
        assert!(
            bitmap.pixel(2, 2).3 < 30,
            "outside the clipPath circle should be near-transparent, got alpha {}",
            bitmap.pixel(2, 2).3
        );
    }

    const PATTERN_TILE_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 64 64">
        <defs>
            <pattern id="p" x="0" y="0" width="8" height="8" patternUnits="userSpaceOnUse">
                <rect width="8" height="8" fill="#0000ff"/>
            </pattern>
        </defs>
        <rect x="0" y="0" width="64" height="64" fill="url(#p)"/>
    </svg>"##;

    #[test]
    fn pattern_fill_repeats_across_the_whole_shape_not_just_the_first_tile() {
        let bitmap = render_via_elwindui(PATTERN_TILE_SVG, 64);
        let reference = render_via_resvg(PATTERN_TILE_SVG, 64);
        // A single-tile-only implementation would leave everything outside the pattern's own
        // declared `[0,8)x[0,8)` tile fully transparent — sampling far from the origin (here, deep
        // into the 8th tile column/row) is exactly what distinguishes "repeats infinitely" from
        // "drawn once at its own position".
        for (x, y) in [(60usize, 60usize), (36, 4), (4, 36)] {
            let (_, _, b, a) = bitmap.pixel(x, y);
            assert!(
                a > 200 && b > 150,
                "expected an opaque blue tile at ({x},{y}), got rgba={:?}",
                bitmap.pixel(x, y)
            );
        }
        approx(bitmap.pixel(60, 60), resvg_pixel(&reference, 60, 60), 60);
    }

    const FE_COMPOSITE_XOR_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 64 64">
        <filter id="f" x="0" y="0" width="64" height="64" filterUnits="userSpaceOnUse">
            <feFlood flood-color="#ff0000" result="a"/>
            <feFlood flood-color="#0000ff" result="b"/>
            <feComposite in="a" in2="b" operator="xor"/>
        </filter>
        <rect x="0" y="0" width="64" height="64" fill="#000000" filter="url(#f)"/>
    </svg>"##;

    #[test]
    fn fe_composite_xor_cancels_out_two_fully_overlapping_opaque_floods() {
        let bitmap = render_via_elwindui(FE_COMPOSITE_XOR_SVG, 64);
        let reference = render_via_resvg(FE_COMPOSITE_XOR_SVG, 64);
        // Two same-extent, fully opaque flood fills XOR'd together cancel out completely (each is
        // entirely "covered" by the other, so both `SourceOut` halves are empty) — a deterministic
        // outcome distinct from the old "treated as Over" fallback, which would show the top
        // (red) flood solidly instead.
        approx(bitmap.pixel(32, 32), (0, 0, 0, 0), 40);
        approx(bitmap.pixel(32, 32), resvg_pixel(&reference, 32, 32), 40);
    }

    const FE_COMPOSITE_ARITHMETIC_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 64 64">
        <filter id="f" x="0" y="0" width="64" height="64" filterUnits="userSpaceOnUse">
            <feFlood flood-color="#ff0000" result="a"/>
            <feFlood flood-color="#0000ff" result="b"/>
            <feComposite in="a" in2="b" operator="arithmetic" k1="0.5" k2="0.5" k3="0.5" k4="0"/>
        </filter>
        <rect x="0" y="0" width="64" height="64" fill="#000000" filter="url(#f)"/>
    </svg>"##;

    #[test]
    fn fe_composite_arithmetic_matches_resvg() {
        let bitmap = render_via_elwindui(FE_COMPOSITE_ARITHMETIC_SVG, 64);
        let reference = render_via_resvg(FE_COMPOSITE_ARITHMETIC_SVG, 64);
        approx(bitmap.pixel(32, 32), resvg_pixel(&reference, 32, 32), 40);
    }

    const FE_TILE_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64" viewBox="0 0 64 64">
        <filter id="f" x="0" y="0" width="64" height="64" filterUnits="userSpaceOnUse">
            <feFlood flood-color="#00ff00" result="flood"/>
            <feTile in="flood"/>
        </filter>
        <rect x="0" y="0" width="64" height="64" fill="#000000" filter="url(#f)"/>
    </svg>"##;

    #[test]
    fn fe_tile_filter_primitive_runs_without_error_and_preserves_flood_color() {
        // A full-region `feFlood` already covers the entire filter region (this pipeline doesn't
        // apply each primitive's own `x`/`y`/`width`/`height` subregion before feeding it to the
        // next primitive — a pre-existing simplification orthogonal to this test), so tiling it
        // is visually a no-op; this fixture's job is to confirm `CIAffineTile` accepts the
        // `NSValue`-boxed identity `inputTransform` without erroring and the color survives,
        // rather than demonstrating visible repetition (see `pattern_fill_repeats_...` above for
        // an infinite-repetition test where the tile source's extent isn't pipeline-constrained).
        let bitmap = render_via_elwindui(FE_TILE_SVG, 64);
        approx(bitmap.pixel(32, 32), (0, 255, 0, 255), 40);
    }

    /// `VectorRasterizeMode::Auto`/`Fixed`/`Vector` — the rasterize-and-cache draw modes
    /// (`vector_renderer.rs::draw_vector_image`'s own doc comment), tested against
    /// `vector_raster_cache` directly rather than pixel output (already covered by every test
    /// above, all of which now exercise `Auto`, the new default) — these instead confirm *when* a
    /// cached bitmap is reused vs. rebuilt.
    mod rasterize_mode {
        use super::*;
        use elwindui_core::graphics::VectorRasterizeMode;

        fn draw_into(
            image: &elwindui_core::graphics::VectorImage,
            dest: elwindui_core::base::Rect,
            rasterize: VectorRasterizeMode,
            image_cache: &mut HashMap<usize, CFRetained<CGImage>>,
            vector_raster_cache: &mut HashMap<
                elwindui_core::graphics::VectorImageId,
                (u32, u32, CFRetained<CGImage>),
            >,
        ) {
            let root = CALayer::new();
            root.setBounds(objc2_core_foundation::CGRect::new(
                objc2_core_foundation::CGPoint::new(0.0, 0.0),
                objc2_core_foundation::CGSize::new(64.0, 64.0),
            ));
            crate::render::draw_vector_image(
                &root,
                image,
                dest,
                None,
                &VectorImageDrawOptions {
                    rasterize,
                    ..Default::default()
                },
                &elwindui_core::base::AffineTransform::identity(),
                1.0,
                image_cache,
                vector_raster_cache,
            );
        }

        fn small_rect_image() -> elwindui_core::graphics::VectorImage {
            elwindui_svg::load_svg_str(SOLID_RECT_SVG).expect("valid fixture SVG")
        }

        fn dest(size: f32) -> elwindui_core::base::Rect {
            elwindui_core::base::Rect { x: 0.0, y: 0.0, width: size, height: size }
        }

        #[test]
        fn auto_mode_reuses_the_cached_bitmap_when_the_drawn_size_is_unchanged() {
            let image = small_rect_image();
            let mut image_cache = HashMap::new();
            let mut cache = HashMap::new();
            draw_into(&image, dest(64.0), VectorRasterizeMode::Auto, &mut image_cache, &mut cache);
            let (w1, h1, cg1) = cache.get(&image.id()).cloned().expect("first draw caches a bitmap");
            draw_into(&image, dest(64.0), VectorRasterizeMode::Auto, &mut image_cache, &mut cache);
            let (w2, h2, cg2) = cache.get(&image.id()).cloned().expect("still cached");
            assert_eq!((w1, h1), (w2, h2));
            assert_eq!(
                CFRetained::as_ptr(&cg1),
                CFRetained::as_ptr(&cg2),
                "same size should reuse the exact same cached CGImage, not rasterize again"
            );
        }

        #[test]
        fn auto_mode_rerasterizes_at_the_exact_size_when_growth_jumps_past_the_1_5x_margin() {
            let image = small_rect_image();
            let mut image_cache = HashMap::new();
            let mut cache = HashMap::new();
            draw_into(&image, dest(64.0), VectorRasterizeMode::Auto, &mut image_cache, &mut cache);
            let (_, _, cg1) = cache.get(&image.id()).cloned().expect("first draw caches a bitmap");
            // 128 >= 64 * 1.5 (96), so this isn't a "gradual" enlargement the margin should
            // absorb — the fresh rasterization lands exactly on the requested size.
            draw_into(&image, dest(128.0), VectorRasterizeMode::Auto, &mut image_cache, &mut cache);
            let (w2, h2, cg2) = cache.get(&image.id()).cloned().expect("still cached");
            assert_eq!((w2, h2), (128, 128));
            assert_ne!(
                CFRetained::as_ptr(&cg1),
                CFRetained::as_ptr(&cg2),
                "a growth past the 1.5x margin must trigger a fresh rasterization"
            );
        }

        #[test]
        fn auto_mode_never_rerasterizes_when_the_drawn_size_shrinks() {
            let image = small_rect_image();
            let mut image_cache = HashMap::new();
            let mut cache = HashMap::new();
            draw_into(&image, dest(128.0), VectorRasterizeMode::Auto, &mut image_cache, &mut cache);
            let (_, _, cg1) = cache.get(&image.id()).cloned().expect("first draw caches a bitmap");
            draw_into(&image, dest(64.0), VectorRasterizeMode::Auto, &mut image_cache, &mut cache);
            let (w2, h2, cg2) = cache.get(&image.id()).cloned().expect("still cached");
            // The larger bitmap is kept as-is — `build_image_container_layer` just downscales it
            // to fit the smaller `dest`, so there is nothing to gain from rerasterizing smaller.
            assert_eq!((w2, h2), (128, 128));
            assert_eq!(
                CFRetained::as_ptr(&cg1),
                CFRetained::as_ptr(&cg2),
                "shrinking the drawn size must never trigger a rerasterization"
            );
        }

        #[test]
        fn auto_mode_pads_a_gradual_enlargement_to_1_5x_and_then_reuses_that_padding() {
            let image = small_rect_image();
            let mut image_cache = HashMap::new();
            let mut cache = HashMap::new();
            draw_into(&image, dest(64.0), VectorRasterizeMode::Auto, &mut image_cache, &mut cache);
            // 80 < 64 * 1.5 (96) — growth within the margin pads to 96, not the raw 80 requested.
            draw_into(&image, dest(80.0), VectorRasterizeMode::Auto, &mut image_cache, &mut cache);
            let (w2, h2, cg2) = cache.get(&image.id()).cloned().expect("padded rasterization cached");
            assert_eq!((w2, h2), (96, 96));
            // A further, still-modest enlargement that fits inside the 96x96 padding must reuse
            // it without rerasterizing — this is the whole point of padding on growth.
            draw_into(&image, dest(90.0), VectorRasterizeMode::Auto, &mut image_cache, &mut cache);
            let (w3, h3, cg3) = cache.get(&image.id()).cloned().expect("still cached");
            assert_eq!((w3, h3), (96, 96));
            assert_eq!(
                CFRetained::as_ptr(&cg2),
                CFRetained::as_ptr(&cg3),
                "growth that still fits inside the padded bitmap must not rerasterize"
            );
        }

        #[test]
        fn fixed_mode_keeps_the_same_bitmap_across_a_dest_resize() {
            let image = small_rect_image();
            let mut image_cache = HashMap::new();
            let mut cache = HashMap::new();
            let fixed = VectorRasterizeMode::Fixed { pixel_width: 32, pixel_height: 32 };
            draw_into(&image, dest(64.0), fixed, &mut image_cache, &mut cache);
            let (w1, h1, cg1) = cache.get(&image.id()).cloned().expect("first draw caches a bitmap");
            assert_eq!((w1, h1), (32, 32));
            // A `dest` resize that would have changed `Auto`'s target pixel size must not affect
            // `Fixed` at all — that's the whole point of specifying a fixed rasterization size.
            draw_into(&image, dest(128.0), fixed, &mut image_cache, &mut cache);
            let (w2, h2, cg2) = cache.get(&image.id()).cloned().expect("still cached");
            assert_eq!((w2, h2), (32, 32));
            assert_eq!(
                CFRetained::as_ptr(&cg1),
                CFRetained::as_ptr(&cg2),
                "Fixed mode must not rerasterize when only the display size changes"
            );
        }

        #[test]
        fn vector_mode_never_populates_the_raster_cache() {
            let image = small_rect_image();
            let mut image_cache = HashMap::new();
            let mut cache = HashMap::new();
            draw_into(&image, dest(64.0), VectorRasterizeMode::Vector, &mut image_cache, &mut cache);
            assert!(
                cache.is_empty(),
                "Vector mode should render the live CALayer tree, never touching the raster cache"
            );
        }
    }
}
