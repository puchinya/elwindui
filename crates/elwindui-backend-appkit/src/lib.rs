//! AppKit implementation of the widget surface `elwindui-codegen` targets for the `notepad`
//! example. See docs/elwindui_spec.md 付録A, 付録C, docs/elwindui_gui_framework_design.md §3.
//!
//! `elwindui-core`'s `Element`/`LayoutNode`/etc. traits aren't implemented against yet (see
//! docs/elwindui_gui_framework_design.md §3 for that integration, deferred); this crate exposes a
//! small standalone widget API (`Window`/`Column`/`Row`/`TextArea`/`Button`/`Text`) that the
//! generated code calls directly.

#![cfg(target_os = "macos")]

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, sel, AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSButton, NSScrollView,
    NSStackView, NSTextDelegate, NSTextField, NSTextView, NSTextViewDelegate,
    NSUserInterfaceLayoutOrientation, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSNotification, NSObjectProtocol, NSRect, NSString};
use std::cell::RefCell;
use std::rc::Rc;

fn mtm() -> MainThreadMarker {
    MainThreadMarker::new().expect("elwindui-backend-appkit must run on the main thread")
}

/// Everything the generated code can pass as a `Column`/`Row`/`Window` child.
#[derive(Clone)]
pub enum AnyView {
    Column(Column),
    Row(Row),
    TextArea(TextArea),
    Button(Button),
    Text(Text),
}

impl AnyView {
    fn as_nsview(&self) -> Retained<NSView> {
        match self {
            AnyView::Column(v) => Retained::into_super(v.ns.clone()),
            AnyView::Row(v) => Retained::into_super(v.ns.clone()),
            AnyView::TextArea(v) => Retained::into_super(v.scroll.clone()),
            AnyView::Button(v) => {
                let control: Retained<objc2_app_kit::NSControl> = Retained::into_super(v.ns.clone());
                let view: Retained<NSView> = Retained::into_super(control);
                view
            }
            AnyView::Text(v) => {
                let control: Retained<objc2_app_kit::NSControl> = Retained::into_super(v.ns.clone());
                let view: Retained<NSView> = Retained::into_super(control);
                view
            }
        }
    }
}

impl From<Column> for AnyView {
    fn from(v: Column) -> Self {
        AnyView::Column(v)
    }
}
impl From<Row> for AnyView {
    fn from(v: Row) -> Self {
        AnyView::Row(v)
    }
}
impl From<TextArea> for AnyView {
    fn from(v: TextArea) -> Self {
        AnyView::TextArea(v)
    }
}
impl From<Button> for AnyView {
    fn from(v: Button) -> Self {
        AnyView::Button(v)
    }
}
impl From<Text> for AnyView {
    fn from(v: Text) -> Self {
        AnyView::Text(v)
    }
}

#[derive(Clone)]
pub struct Window {
    ns: Retained<NSWindow>,
}

impl Window {
    pub fn new(title: &str) -> Self {
        let mtm = mtm();
        let content_rect = NSRect::new(objc2_foundation::NSPoint::new(0.0, 0.0), objc2_foundation::NSSize::new(480.0, 360.0));
        let style = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable
            | NSWindowStyleMask::Resizable;
        let ns = unsafe {
            let alloc = mtm.alloc::<NSWindow>();
            NSWindow::initWithContentRect_styleMask_backing_defer(
                alloc,
                content_rect,
                style,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        ns.setTitle(&NSString::from_str(title));
        Self { ns }
    }

    pub fn set_content(&self, view: AnyView) {
        self.ns.setContentView(Some(&view.as_nsview()));
    }

    pub fn set_title(&self, title: &str) {
        self.ns.setTitle(&NSString::from_str(title));
    }

    /// Blocking: activates the app and runs the AppKit main loop.
    pub fn show_and_run(&self) {
        let mtm = mtm();
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
        self.ns.makeKeyAndOrderFront(None);
        app.activate();
        app.run();
    }
}

#[derive(Clone)]
pub struct Column {
    ns: Retained<NSStackView>,
}

impl Column {
    pub fn new(children: Vec<AnyView>) -> Self {
        Self { ns: new_stack(children, NSUserInterfaceLayoutOrientation::Vertical) }
    }
}

#[derive(Clone)]
pub struct Row {
    ns: Retained<NSStackView>,
}

impl Row {
    pub fn new(children: Vec<AnyView>) -> Self {
        Self { ns: new_stack(children, NSUserInterfaceLayoutOrientation::Horizontal) }
    }
}

fn new_stack(children: Vec<AnyView>, orientation: NSUserInterfaceLayoutOrientation) -> Retained<NSStackView> {
    let m = mtm();
    let views: Vec<Retained<NSView>> = children.iter().map(AnyView::as_nsview).collect();
    let ns = NSStackView::stackViewWithViews(&objc2_foundation::NSArray::from_retained_slice(&views), m);
    ns.setOrientation(orientation);
    ns
}

#[derive(Clone)]
pub struct TextArea {
    scroll: Retained<NSScrollView>,
    text_view: Retained<NSTextView>,
    delegate_storage: Rc<RefCell<Option<Retained<TextViewDelegate>>>>,
}

impl TextArea {
    pub fn new(initial_text: &str) -> Self {
        let m = mtm();
        let scroll = NSTextView::scrollableTextView(m);
        let text_view = scroll
            .documentView()
            .expect("scrollableTextView always has a document view")
            .downcast::<NSTextView>()
            .expect("scrollableTextView's document view is an NSTextView");
        text_view.setString(&NSString::from_str(initial_text));
        Self { scroll, text_view, delegate_storage: Rc::new(RefCell::new(None)) }
    }

    pub fn set_text(&self, text: &str) {
        self.text_view.setString(&NSString::from_str(text));
    }

    pub fn set_on_change(&self, callback: Box<dyn Fn(String)>) {
        let m = mtm();
        let ivars = TextDelegateIvars { text_view: self.text_view.clone(), callback };
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

#[derive(Clone)]
pub struct Button {
    ns: Retained<NSButton>,
    target_storage: Rc<RefCell<Option<Retained<ButtonTarget>>>>,
}

impl Button {
    pub fn new(title: &str) -> Self {
        let m = mtm();
        let ns = unsafe {
            NSButton::buttonWithTitle_target_action(&NSString::from_str(title), None, None, m)
        };
        Self { ns, target_storage: Rc::new(RefCell::new(None)) }
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.ns.setEnabled(enabled);
    }

    pub fn set_on_click(&self, callback: Box<dyn Fn()>) {
        let target = ButtonTarget::new(ButtonTargetIvars { callback });
        unsafe {
            self.ns.setTarget(Some(&target));
            self.ns.setAction(Some(sel!(perform:)));
        }
        *self.target_storage.borrow_mut() = Some(target);
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

#[derive(Clone)]
pub struct Text {
    ns: Retained<NSTextField>,
}

impl Text {
    pub fn new(text: &str) -> Self {
        let m = mtm();
        let ns = NSTextField::labelWithString(&NSString::from_str(text), m);
        Self { ns }
    }

    pub fn set_text(&self, text: &str) {
        self.ns.setStringValue(&NSString::from_str(text));
    }
}

/// See docs/elwindui_spec.md 付録T.2. Modal file panels (`runModal`) are themselves synchronous
/// (they block until the user closes the panel), so these `async fn`s never actually suspend —
/// they resolve on the first poll. That's enough for `#[command(async)]` bodies that just need to
/// `.await` a dialog result; it is not a general-purpose async executor (nothing here can yield
/// across a real I/O wait), which is what `elwindui-core`'s planned `Dispatcher`/`spawn`
/// (docs/elwindui_gui_framework_design.md §7.3) is for.
pub mod platform {
    pub mod file_dialog {
        use crate::mtm;
        use objc2_app_kit::{NSModalResponseOK, NSOpenPanel, NSSavePanel};
        use std::path::PathBuf;

        pub async fn open() -> Option<PathBuf> {
            let panel = NSOpenPanel::openPanel(mtm());
            if panel.runModal() != NSModalResponseOK {
                return None;
            }
            panel.URL().and_then(|url| url.path()).map(|p| PathBuf::from(p.to_string()))
        }

        pub async fn save() -> Option<PathBuf> {
            let panel = NSSavePanel::savePanel(mtm());
            if panel.runModal() != NSModalResponseOK {
                return None;
            }
            panel.URL().and_then(|url| url.path()).map(|p| PathBuf::from(p.to_string()))
        }
    }
}
