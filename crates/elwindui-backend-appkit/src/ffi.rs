//! The Objective-C seam: the `MainThreadMarker` accessor and the erased native-view handle
//! (`AnyView`) every other layer passes around instead of a concrete `NSView` subclass.
//!
//! `AnyView` is re-exported at the crate root because `elwindui-codegen` generates references to
//! `elwindui::backend::AnyView` directly — that path must stay stable.


use elwindui_core::base::AsAny;
use objc2::rc::Retained;
use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSButton, NSScrollView, NSStackView, NSTextField, NSUserInterfaceLayoutOrientation, NSView,
};
use objc2_foundation::NSRect;
use std::rc::Rc;

pub(crate) fn mtm() -> MainThreadMarker {
    MainThreadMarker::new().expect("elwindui-backend-appkit must run on the main thread")
}

/// The capability a type needs to be usable as an `AnyView` — implemented once per raw native view
/// type (`Retained<NSScrollView>`/`Retained<NSButton>`/`Retained<NSStackView>`) instead of matched
/// on centrally, so a future native leaf only needs its own `impl AppKitHandle`, never a change to
/// `AnyView` itself or to any `match` over it.
pub(crate) trait AppKitHandle: AsAny {
    fn as_nsview(&self) -> Retained<NSView>;
}

impl AppKitHandle for Retained<NSScrollView> {
    fn as_nsview(&self) -> Retained<NSView> {
        Retained::into_super(self.clone())
    }
}
impl AppKitHandle for Retained<NSButton> {
    fn as_nsview(&self) -> Retained<NSView> {
        let control: Retained<objc2_app_kit::NSControl> = Retained::into_super(self.clone());
        Retained::into_super(control)
    }
}
impl AppKitHandle for Retained<NSStackView> {
    fn as_nsview(&self) -> Retained<NSView> {
        Retained::into_super(self.clone())
    }
}
impl AppKitHandle for Retained<NSTextField> {
    fn as_nsview(&self) -> Retained<NSView> {
        let control: Retained<objc2_app_kit::NSControl> = Retained::into_super(self.clone());
        Retained::into_super(control)
    }
}

/// Everything the generated code can pass as a `Window`/`TabView` child. An `Rc<dyn AppKitHandle>`
/// (not a closed `enum`) so adding a new native leaf never requires touching this type — see
/// `AppKitHandle`'s own doc comment. Re-exported at the crate root (`lib.rs`) since
/// `elwindui-codegen`'s generated code references `elwindui::backend::AnyView` directly.
#[derive(Clone)]
pub struct AnyView(Rc<dyn AppKitHandle>);

impl AnyView {
    /// Stable identity of the retained native handle. Reusing its container across relayouts is
    /// essential: AppKit resigns a control that is temporarily removed from its superview.
    pub(crate) fn identity(&self) -> usize {
        Rc::as_ptr(&self.0) as *const () as usize
    }

    pub(crate) fn as_nsview(&self) -> Retained<NSView> {
        self.0.as_nsview()
    }

    /// Lets every native leaf's `measure_override` (in `native_ui.rs::NativeControl`) measure any
    /// wrapped widget uniformly through the base `NSView` API (`fittingSize`) regardless of which
    /// concrete widget it wraps.
    pub(crate) fn measure(
        &self,
        _available: elwindui_core::base::Size,
    ) -> elwindui_core::base::Size {
        let fitting = self.as_nsview().fittingSize();
        elwindui_core::base::Size {
            width: fitting.width as f32,
            height: fitting.height as f32,
        }
    }

    /// Positions this native leaf via plain `NSView.setFrame` — called directly by `TreeHostView`'s
    /// own render loop below, after `layout_root` and RenderTree reconciliation have produced its
    /// retained native command.
    pub(crate) fn arrange(&mut self, final_rect: elwindui_core::base::Rect) {
        self.as_nsview().setFrame(NSRect::new(
            objc2_foundation::NSPoint::new(final_rect.x as f64, final_rect.y as f64),
            objc2_foundation::NSSize::new(final_rect.width as f64, final_rect.height as f64),
        ));
    }
}

impl<T: AppKitHandle + 'static> From<T> for AnyView {
    fn from(v: T) -> Self {
        AnyView(Rc::new(v))
    }
}

pub(crate) fn new_stack(
    children: Vec<AnyView>,
    orientation: NSUserInterfaceLayoutOrientation,
) -> Retained<NSStackView> {
    let m = mtm();
    let views: Vec<Retained<NSView>> = children.iter().map(AnyView::as_nsview).collect();
    let ns =
        NSStackView::stackViewWithViews(&objc2_foundation::NSArray::from_retained_slice(&views), m);
    ns.setOrientation(orientation);
    ns
}
