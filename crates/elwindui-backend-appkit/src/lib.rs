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
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSBackingStoreType,
    NSButton, NSMenu, NSMenuItem, NSScrollView, NSStackView, NSTextDelegate, NSTextField,
    NSTextView, NSTextViewDelegate, NSUserInterfaceLayoutOrientation, NSView, NSWindow,
    NSWindowStyleMask,
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
    TabView(TabView),
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
            AnyView::TabView(v) => Retained::into_super(v.root.clone()),
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
impl From<TabView> for AnyView {
    fn from(v: TabView) -> Self {
        AnyView::TabView(v)
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

    /// Sets `NSApplication.mainMenu` (macOS has one global top menu bar, not a per-window one).
    pub fn set_menu_bar(&self, menu_bar: &MenuBar) {
        NSApplication::sharedApplication(mtm()).setMainMenu(Some(&menu_bar.ns));
    }

    /// Shows the window and activates the app. Does not block — call `application::run()`
    /// afterward to actually enter the platform event loop (see that module's doc comment for
    /// why the two are separate).
    pub fn show(&self) {
        let mtm = mtm();
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
        self.ns.makeKeyAndOrderFront(None);
        app.activate();
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

    /// `NSTextView.delegate` is an unretained (weak) reference, so the delegate this creates is
    /// only kept alive by `self.delegate_storage`. Since `TextArea` derives `Clone` by sharing
    /// that `Rc`, the delegate survives as long as *any* clone of this `TextArea` value does —
    /// but if every clone is dropped (e.g. a caller extracts just the raw `NSView` and discards
    /// the `TextArea` struct itself), the delegate is deallocated and `on_change` silently stops
    /// firing, even though typed characters keep appearing on screen (native `NSTextView`
    /// rendering needs no delegate). Found via `TabView`'s content pane, which used to do exactly
    /// that — see `TabView::set_content`'s doc comment.
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

    /// Used by `TabChip` to rename a tab's title button when its document's file name changes.
    pub fn set_text(&self, text: &str) {
        self.ns.setTitle(&NSString::from_str(text));
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

/// See docs/elwindui_builtins_spec.md 付録Y. A single tab's header: a title button (click to
/// select) plus a small close button, packed into one row so `TabStrip` can insert/remove it as
/// one unit.
#[derive(Clone)]
pub struct TabChip {
    ns: Retained<NSStackView>,
    pub title_button: Button,
    pub close_button: Button,
}

impl TabChip {
    fn new(title: &str) -> Self {
        let title_button = Button::new(title);
        let close_button = Button::new("×");
        let ns = new_stack(
            vec![AnyView::Button(title_button.clone()), AnyView::Button(close_button.clone())],
            NSUserInterfaceLayoutOrientation::Horizontal,
        );
        Self { ns, title_button, close_button }
    }

    pub fn set_title(&self, title: &str) {
        self.title_button.set_text(title);
    }
}

/// The row of `TabChip`s plus a trailing "+" button. `TabView` owns one of these and the content
/// area below it; kept as a separate type since 付録Y's backend table describes it as its own
/// piece (a custom `NSStackView`-based strip, not `NSTabViewController`).
#[derive(Clone)]
pub struct TabStrip {
    ns: Retained<NSStackView>,
    pub new_tab_button: Button,
}

impl TabStrip {
    fn new() -> Self {
        let new_tab_button = Button::new("+");
        let ns = new_stack(
            vec![AnyView::Button(new_tab_button.clone())],
            NSUserInterfaceLayoutOrientation::Horizontal,
        );
        Self { ns, new_tab_button }
    }

    /// Inserts a chip before the "+" button, at arranged-subview position `index`.
    fn insert_tab(&self, index: usize, title: &str) -> TabChip {
        let chip = TabChip::new(title);
        let view: Retained<NSView> = Retained::into_super(chip.ns.clone());
        self.ns.insertArrangedSubview_atIndex(&view, index as isize);
        chip
    }

    fn remove_tab(&self, chip: &TabChip) {
        let view: Retained<NSView> = Retained::into_super(chip.ns.clone());
        self.ns.removeArrangedSubview(&view);
        view.removeFromSuperview();
    }
}

/// See docs/elwindui_builtins_spec.md 付録Y. Vertical stack of `[TabStrip, content_area]`; the
/// generated code (see `elwindui-codegen`'s specialized `TabView` codegen path) owns the mapping
/// from the observable tab list to `TabChip`s and calls `set_content` whenever the active tab
/// changes. This type only holds the two widget areas — it has no notion of "the list of tabs" on
/// its own, matching how `Column`/`Row` don't know about the data their children came from either.
#[derive(Clone)]
pub struct TabView {
    root: Retained<NSStackView>,
    pub strip: TabStrip,
    content_area: Retained<NSStackView>,
    // Stores the whole `AnyView`, not just its extracted `Retained<NSView>` — a `TextArea`'s
    // change-notification delegate (`delegate_storage`) only stays alive as long as *some* clone
    // of that `TextArea` value does (see `TextArea::set_on_change`'s doc comment). Keeping only
    // the bare `NSView` here would drop the last such clone the moment `set_content` returns,
    // deallocating the delegate — typed characters would still render (native `NSTextView`
    // behavior needs no delegate) but `on_change` would silently never fire again, so edits would
    // never reach the model.
    current_content: Rc<RefCell<Option<AnyView>>>,
}

impl TabView {
    pub fn new() -> Self {
        let strip = TabStrip::new();
        let content_area = new_stack(vec![], NSUserInterfaceLayoutOrientation::Vertical);
        let strip_view: Retained<NSView> = Retained::into_super(strip.ns.clone());
        let content_view: Retained<NSView> = Retained::into_super(content_area.clone());
        let root = NSStackView::stackViewWithViews(
            &objc2_foundation::NSArray::from_retained_slice(&[strip_view, content_view]),
            mtm(),
        );
        root.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
        Self { root, strip, content_area, current_content: Rc::new(RefCell::new(None)) }
    }

    pub fn set_on_new_tab(&self, callback: Box<dyn Fn()>) {
        self.strip.new_tab_button.set_on_click(callback);
    }

    /// Inserts a new tab chip at `index`, wiring `on_select`/`on_close` to the given callbacks.
    pub fn insert_tab(
        &self,
        index: usize,
        title: &str,
        on_select: Box<dyn Fn()>,
        on_close: Box<dyn Fn()>,
    ) -> TabChip {
        let chip = self.strip.insert_tab(index, title);
        chip.title_button.set_on_click(on_select);
        chip.close_button.set_on_click(on_close);
        chip
    }

    pub fn remove_tab(&self, chip: &TabChip) {
        self.strip.remove_tab(chip);
    }

    /// Swaps the single visible document pane for the currently selected tab.
    pub fn set_content(&self, view: AnyView) {
        if let Some(old) = self.current_content.borrow_mut().take() {
            let old_view = old.as_nsview();
            self.content_area.removeArrangedSubview(&old_view);
            old_view.removeFromSuperview();
        }
        let new_view = view.as_nsview();
        self.content_area.addArrangedSubview(&new_view);
        *self.current_content.borrow_mut() = Some(view);
    }
}

/// See docs/elwindui_builtins_spec.md 付録X. A single application-wide `NSMenu` (top menu bar
/// item / `File`, `Edit`, ...), reusing `MenuItem` for its leaf entries.
#[derive(Clone)]
pub struct MenuItem {
    ns: Retained<NSMenuItem>,
    target_storage: Rc<RefCell<Option<Retained<MenuItemTarget>>>>,
}

impl MenuItem {
    pub fn new(title: &str) -> Self {
        let m = mtm();
        let ns = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                m.alloc::<NSMenuItem>(),
                &NSString::from_str(title),
                None,
                &NSString::from_str(""),
            )
        };
        Self { ns, target_storage: Rc::new(RefCell::new(None)) }
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.ns.setEnabled(enabled);
    }

    /// A bare key character (e.g. `"s"`); macOS defaults a menu item's modifier mask to Cmd,
    /// which matches the common `Cmd+<letter>` shortcuts notepad needs (付録K.2's platform
    /// conversion rule already reads "Ctrl" as "Cmd" on macOS at the DSL level).
    pub fn set_shortcut(&self, key_equivalent: &str) {
        self.ns.setKeyEquivalent(&NSString::from_str(key_equivalent));
    }

    pub fn set_on_select(&self, callback: Box<dyn Fn()>) {
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
/// that way here, but the same type covers both).
#[derive(Clone)]
pub struct Menu {
    ns: Retained<NSMenu>,
}

impl Menu {
    pub fn new(items: Vec<MenuItem>) -> Self {
        let m = mtm();
        let ns = NSMenu::initWithTitle(m.alloc::<NSMenu>(), &NSString::from_str(""));
        for item in &items {
            ns.addItem(&item.ns);
        }
        Self { ns }
    }
}

/// One top-level entry in the menu bar (e.g. "File"), holding its dropdown `Menu`.
#[derive(Clone)]
pub struct MenuBarItem {
    ns: Retained<NSMenuItem>,
}

impl MenuBarItem {
    pub fn new(title: &str, submenu: Menu) -> Self {
        let m = mtm();
        let ns = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                m.alloc::<NSMenuItem>(),
                &NSString::from_str(title),
                None,
                &NSString::from_str(""),
            )
        };
        ns.setSubmenu(Some(&submenu.ns));
        Self { ns }
    }
}

/// The whole top menu bar, installed via `Window::set_menu_bar`.
#[derive(Clone)]
pub struct MenuBar {
    ns: Retained<NSMenu>,
}

impl MenuBar {
    pub fn new(items: Vec<MenuBarItem>) -> Self {
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

        for item in &items {
            ns.addItem(&item.ns);
        }
        Self { ns }
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

/// AppKit's `Dispatcher` (docs/elwindui_spec.md 付録P.5): hops back to the main thread via GCD's
/// main queue, which `NSApplication.run()` (`application::run()` below) actively services as part
/// of its own event loop — so a job enqueued from any thread (a background `tokio` task
/// completing, say) is guaranteed to run promptly. See `elwindui_core::task` for how this lets a
/// suspended `#[command(async)]` body resume back on the UI thread, the same role C#'s
/// `SynchronizationContext.Post` plays.
pub struct AppKitDispatcher;

impl elwindui_core::task::Dispatcher for AppKitDispatcher {
    fn enqueue(&self, job: Box<dyn FnOnce() + Send + 'static>) {
        dispatch2::DispatchQueue::main().exec_async(job);
    }
}

thread_local! {
    /// `NSApplication.delegate` is an unretained (weak) reference (same situation as
    /// `TextArea::delegate_storage`), so this keeps it alive for the process's lifetime.
    static APP_DELEGATE: RefCell<Option<Retained<AppDelegate>>> = const { RefCell::new(None) };
}

define_class!(
    #[unsafe(super(objc2_foundation::NSObject))]
    #[thread_kind = objc2::MainThreadOnly]
    struct AppDelegate;

    unsafe impl NSObjectProtocol for AppDelegate {}

    unsafe impl NSApplicationDelegate for AppDelegate {
        /// Without this, AppKit's default behavior leaves the process running after the last
        /// (only, for `notepad`) window is closed via its close button.
        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn should_terminate_after_last_window_closed(&self, _sender: &NSApplication) -> bool {
            true
        }
    }
);

impl AppDelegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

/// The single entry point that owns "enter the platform event loop" — kept separate from
/// `Window::show()` so that there's one well-defined place to install the task executor (see
/// `elwindui_core::task::set_current`) and the app delegate before any generated code runs. Call
/// once, after showing the app's window(s).
pub mod application {
    use super::{mtm, AppDelegate, AppKitDispatcher, APP_DELEGATE};
    use elwindui_core::task::LocalExecutor;
    use objc2_app_kit::NSApplication;

    /// Blocking: enters the AppKit main event loop.
    pub fn run() {
        elwindui_core::task::set_current(LocalExecutor::new(AppKitDispatcher));

        let mtm = mtm();
        let app = NSApplication::sharedApplication(mtm);
        let delegate = AppDelegate::new(mtm);
        app.setDelegate(Some(objc2::runtime::ProtocolObject::from_ref(&*delegate)));
        APP_DELEGATE.with(|d| *d.borrow_mut() = Some(delegate));

        app.run();
    }
}
