//! The WinRT seam: the erased native-element handle (`AnyView`) every other layer passes
//! around instead of a concrete XAML type, plus the `usize`-keyed UI-event callback registry the
//! generated WinRT handlers need (they require `Send`, which an `Rc`-holding closure is not).
//!
//! `AnyView` is re-exported at the crate root because `elwindui-codegen` generates references to
//! `elwindui::backend::AnyView` directly — that path must stay stable.

use crate::bindings;
use crate::bindings::Microsoft::UI::Input::InputKeyboardSource;
use crate::bindings::Microsoft::UI::Xaml::Controls::{
    Button as XamlButton, Canvas, MenuFlyoutItem, MenuFlyoutItemBase, PasswordBox as XamlPasswordBox,
    ScrollMode, ScrollViewer, TabView as XamlTabView, TabViewCloseButtonOverlayMode, TabViewItem,
    TabViewTabCloseRequestedEventArgs, TextBlock, TextBox as XamlTextBox,
};
use crate::bindings::Microsoft::UI::Xaml::Input::{
    CharacterReceivedRoutedEventArgs, KeyEventHandler, KeyboardAccelerator,
};
use crate::bindings::Microsoft::UI::Xaml::Media::SolidColorBrush;
use crate::bindings::Microsoft::UI::Xaml::{
    FrameworkElement, RoutedEventHandler, SizeChangedEventHandler, UIElement, Window as XamlWindow,
};
use crate::bindings::Microsoft::Graphics::Canvas::UI::Composition::CanvasComposition;
use crate::bindings::Microsoft::Graphics::Canvas::{
    CanvasActiveLayer, CanvasAntialiasing, CanvasBitmap, CanvasBlend, CanvasEdgeBehavior, CanvasImageInterpolation,
    ICanvasResourceCreator,
};
use crate::bindings::Microsoft::UI::Composition::CompositionDrawingSurface;
use crate::bindings::Microsoft::Graphics::Canvas::Brushes::{
    CanvasGradientStop, CanvasImageBrush, CanvasLinearGradientBrush, CanvasRadialGradientBrush,
    CanvasSolidColorBrush, ICanvasBrush,
};
use crate::bindings::Microsoft::Graphics::Canvas::Geometry::{
    CanvasArcSize, CanvasFigureLoop, CanvasFilledRegionDetermination, CanvasGeometry,
    CanvasPathBuilder, CanvasSweepDirection,
};
use crate::bindings::Microsoft::UI::Xaml::Controls::{
    SelectionChangedEventHandler, TextChangedEventHandler,
};
use windows::Foundation::{PropertyValue, Size, TypedEventHandler};
use windows::Graphics::{PointInt32, SizeInt32};
use windows::System::{VirtualKey, VirtualKeyModifiers};
use windows::UI::{Color, Core::CoreVirtualKeyStates};
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream, IRandomAccessStream};
use elwindui_core::input::{
    FocusState, Key, KeyModifiers, KeyboardDispatcher, RawKeyEvent, RawKeyEventKind,
    RawTextInputEvent, ShortcutRegistry,
};
use elwindui_core::ui::{FocusHost, UIElementExt as _};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicUsize, Ordering};
use windows::core::{HSTRING, Interface, Result};

pub(crate) static NEXT_UI_EVENT_CALLBACK: AtomicUsize = AtomicUsize::new(1);

pub(crate) fn register_ui_event_callback(callback: Rc<dyn Fn()>) -> usize {
    let id = NEXT_UI_EVENT_CALLBACK.fetch_add(1, Ordering::Relaxed);
    UI_EVENT_CALLBACKS.with(|callbacks| {
        callbacks.borrow_mut().insert(id, callback);
    });
    id
}

pub(crate) fn invoke_ui_event_callback(id: usize) {
    // Clone the `Rc` out and drop the borrow *before* calling it — the callback body (e.g. opening
    // a file, which can synchronously rebuild native controls and register new callbacks) can
    // legitimately re-enter `register_ui_event_callback`/this same map. Holding `borrow()` across
    // the call would make that a `RefCell` double-borrow panic (`already borrowed`), not just a
    // theoretical risk — reproduced via `notepad`'s File > Open.
    let callback = UI_EVENT_CALLBACKS.with(|callbacks| callbacks.borrow().get(&id).cloned());
    if let Some(callback) = callback {
        callback();
    }
}

pub(crate) fn register_ui_index_event_callback(callback: Rc<dyn Fn(usize)>) -> usize {
    let id = NEXT_UI_EVENT_CALLBACK.fetch_add(1, Ordering::Relaxed);
    UI_INDEX_EVENT_CALLBACKS.with(|callbacks| { callbacks.borrow_mut().insert(id, callback); });
    id
}

pub(crate) fn invoke_ui_index_event_callback(id: usize, index: usize) {
    // See `invoke_ui_event_callback`'s doc comment — same re-entrancy hazard, same fix.
    let callback = UI_INDEX_EVENT_CALLBACKS.with(|callbacks| callbacks.borrow().get(&id).cloned());
    if let Some(callback) = callback {
        callback(index);
    }
}

pub(crate) fn register_ui_key_event_callback(callback: Rc<dyn Fn(RawKeyEvent)>) -> usize {
    let id = NEXT_UI_EVENT_CALLBACK.fetch_add(1, Ordering::Relaxed);
    UI_KEY_EVENT_CALLBACKS.with(|callbacks| { callbacks.borrow_mut().insert(id, callback); });
    id
}

pub(crate) fn invoke_ui_key_event_callback(id: usize, event: RawKeyEvent) {
    // See `invoke_ui_event_callback`'s doc comment — same re-entrancy hazard, same fix.
    let callback = UI_KEY_EVENT_CALLBACKS.with(|callbacks| callbacks.borrow().get(&id).cloned());
    if let Some(callback) = callback {
        callback(event);
    }
}

pub(crate) fn register_ui_text_event_callback(callback: Rc<dyn Fn(String)>) -> usize {
    let id = NEXT_UI_EVENT_CALLBACK.fetch_add(1, Ordering::Relaxed);
    UI_TEXT_EVENT_CALLBACKS.with(|callbacks| { callbacks.borrow_mut().insert(id, callback); });
    id
}

pub(crate) fn invoke_ui_text_event_callback(id: usize, text: String) {
    // See `invoke_ui_event_callback`'s doc comment — same re-entrancy hazard, same fix.
    let callback = UI_TEXT_EVENT_CALLBACKS.with(|callbacks| callbacks.borrow().get(&id).cloned());
    if let Some(callback) = callback {
        callback(text);
    }
}

/// The capability a type needs to be usable as an `AnyView` — implemented once per raw XAML element
/// type (`XamlTextBox`/`XamlButton`/`XamlTabView`) instead of matched on centrally, so a future native
/// leaf (`Dialog`, `VirtualList`, ...) only needs its own `impl WinUiHandle`, never a change to
/// `AnyView` itself or to any `match` over it — mirrors `elwindui-backend-appkit`'s `AppKitHandle`
/// (see that trait's own doc comment for the rationale).
///
/// Implemented on the raw XAML element type itself (a foreign type — allowed since `WinUiHandle` is
/// a local trait) rather than on `TextArea`/`Button`/`NativeTabView`, since those now each
/// compose this crate's own `NativeControl` (see `native_ui.rs`) as their own `base` field
/// (docs/elwindui_spec.md 付録H.2.1a) — an `AnyView` wrapping the not-yet-fully-constructed widget
/// itself would be a self-reference. Wrapping just the raw element instead lets `base.handle` be
/// built (`AnyView::from(xaml.clone())`) before the rest of the widget struct exists.
pub(crate) trait WinUiHandle: elwindui_core::base::AsAny {
    fn as_element(&self) -> FrameworkElement;
}

impl WinUiHandle for XamlTextBox {
    fn as_element(&self) -> FrameworkElement {
        self.cast().expect("TextBox implements FrameworkElement")
    }
}

impl WinUiHandle for XamlPasswordBox {
    fn as_element(&self) -> FrameworkElement {
        self.cast().expect("PasswordBox implements FrameworkElement")
    }
}

impl WinUiHandle for ScrollViewer {
    fn as_element(&self) -> FrameworkElement {
        self.cast().expect("ScrollViewer implements FrameworkElement")
    }
}

impl WinUiHandle for XamlButton {
    fn as_element(&self) -> FrameworkElement {
        self.cast().expect("Button implements FrameworkElement")
    }
}

impl WinUiHandle for XamlTabView {
    fn as_element(&self) -> FrameworkElement {
        self.cast().expect("TabView implements FrameworkElement")
    }
}

/// Everything the generated code can pass as a `Window`/`NativeTabView` child.
/// `VerticalLayout`/`HorizontalLayout`/`Rectangle`/`Ellipse`/`TextBlock` have no variant here —
/// they're purely `elwindui_core::ui::UIElement` values (see `TreeHostPanel` below). An
/// `Rc<dyn WinUiHandle>` (not a closed `enum`) so adding a new native leaf never requires touching
/// this type — see `WinUiHandle`'s own doc comment. Re-exported at the crate root (`lib.rs`) since
/// `elwindui-codegen`'s generated code references `elwindui::backend::AnyView` directly.
#[derive(Clone)]
pub struct AnyView(Rc<dyn WinUiHandle>);

impl AnyView {
    fn as_element(&self) -> FrameworkElement {
        self.0.as_element()
    }
}

impl AnyView {
    /// Lets `NativeControl::measure_override` (in `native_ui.rs` — shared by every `TextArea`/
    /// `Button`/`TabView` leaf) measure any wrapped widget uniformly through the base
    /// `FrameworkElement`/`UIElement` API regardless of which concrete widget it wraps — no
    /// per-widget re-implementation of the actual `Measure`/`DesiredSize` calls needed. A plain
    /// inherent method, not a shared `elwindui-core`-defined trait — measuring a native handle is
    /// entirely backend-specific, so `elwindui_core::ui::NativeControl` (a pure marker trait)
    /// doesn't know how to do it.
    pub(crate) fn measure(
        &self,
        available: elwindui_core::base::Size,
    ) -> elwindui_core::base::Size {
        let element = self.as_element();
        // `arrange` (below) sets an explicit `Width`/`Height` on this same `FrameworkElement` so a
        // plain `Canvas` (which has no native measure-driven arrange of its own) gives it concrete
        // bounds. That explicit size persists across relayout passes and, once set, permanently
        // overrides the element's own natural/content-driven measurement — a `FrameworkElement`
        // with an explicit `Width` always reports that `Width` from `Measure()` regardless of
        // content, clamped to (not derived from) `available`. Without resetting it back to
        // `NaN` ("unset"/Auto) here first, the very first relayout pass — where `available` is
        // `{0, 0}` before the window has a real size — sets `Width`/`Height` to 0 via `arrange`,
        // and every subsequent `Measure` call then reports `DesiredSize` of 0 forever, regardless
        // of how large `available` later becomes: a self-reinforcing feedback loop, not a one-time
        // glitch. (Containers that arrange children from an external allocation independent of the
        // child's own measured size — e.g. a `Grid` `Star` row — escape this, since their own
        // `arrange` overwrites `Width`/`Height` with that external allocation every pass regardless
        // of what `Measure` last reported; content-driven/Auto containers, whose `arrange` derives
        // each child's final size *from* that same child's `DesiredSize`, do not.)
        let _ = element.SetWidth(f64::NAN);
        let _ = element.SetHeight(f64::NAN);
        // `SetWidth`/`SetHeight` alone mark the element measure-dirty going forward, but don't
        // retroactively guarantee *this* `Measure` call re-runs the native measure pass rather than
        // short-circuiting on a still-cached `DesiredSize` from before the reset — invalidate
        // explicitly so the call below is never skipped.
        let _ = element.InvalidateMeasure();
        let _ = element.Measure(Size {
            Width: available.width as f32,
            Height: available.height as f32,
        });
        let desired = element.DesiredSize().unwrap_or(Size {
            Width: 0.0,
            Height: 0.0,
        });
        elwindui_core::base::Size {
            width: desired.Width,
            height: desired.Height,
        }
    }

    /// Positions this native leaf — like `measure` above, a plain inherent method (elwindui-core's
    /// generic layout code never calls either) — called directly by `TreeHostPanel`'s own render
    /// loop below, after `layout_root` and RenderTree reconciliation have produced its native
    /// command. Unlike AppKit (where `arrange` calls `setFrame` directly),
    /// a `Canvas`'s children are still measured/arranged by the real XAML layout system on every
    /// layout pass — this only needs to set the `Width`/`Height` and `Canvas.Left`/`Canvas.Top`
    /// attached properties once; `Canvas`'s own (built-in) `ArrangeOverride` does the rest, unlike
    /// AppKit's plain `NSView` which has no attached-property positioning at all.
    fn arrange(&mut self, final_rect: elwindui_core::base::Rect) {
        let element = self.as_element();
        let _ = element.SetWidth(final_rect.width as f64);
        let _ = element.SetHeight(final_rect.height as f64);
        let _ = Canvas::SetLeft(&element, final_rect.x as f64);
        let _ = Canvas::SetTop(&element, final_rect.y as f64);
    }
}

impl<T: WinUiHandle + 'static> From<T> for AnyView {
    fn from(v: T) -> Self {
        AnyView(Rc::new(v))
    }
}
