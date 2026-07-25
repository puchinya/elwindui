//! The XAML `Button` and its click handler.

use crate::ffi::{AnyView, register_ui_event_callback, invoke_ui_event_callback, register_ui_index_event_callback, invoke_ui_index_event_callback, register_ui_key_event_callback, invoke_ui_key_event_callback, register_ui_text_event_callback, invoke_ui_text_event_callback};
use crate::bindings;
use crate::render::composition::{
    CompositionClipSpec, CompositionPrimitive, CompositionRenderer, DesiredCompositionIsland,
    DesiredCompositionNode, IslandId,
};
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

/// Raw `XamlButton` + click wiring — composed by `native_ui::Button`.
pub(crate) struct InnerButton {
    handle: AnyView,
    xaml: XamlButton,
    on_click: Rc<RefCell<Option<Box<dyn Fn()>>>>,
}

impl InnerButton {
    pub(crate) fn new() -> Self {
        let xaml = XamlButton::new().expect("Button::new");
        let handle = AnyView::from(xaml.clone());
        let this = Self {
            handle,
            xaml,
            on_click: Rc::new(RefCell::new(None)),
        };
        {
        let callback = this.on_click.clone();
        let callback_id = register_ui_event_callback(Rc::new(move || {
            if let Some(callback) = callback.borrow().as_ref() { callback(); }
        }));
        let _ = this.xaml.Click(&RoutedEventHandler::new(move |_, _| {
            invoke_ui_event_callback(callback_id);
            Ok(())
        }));
        }
        this
    }

    pub(crate) fn handle(&self) -> AnyView {
        self.handle.clone()
    }

    pub(crate) fn set_enabled(&self, enabled: bool) {
        let _ = self.xaml.SetIsEnabled(enabled);
    }

    pub(crate) fn set_on_click(&self, callback: Box<dyn Fn()>) {
        *self.on_click.borrow_mut() = Some(callback);
    }

    pub(crate) fn set_text(&self, text: &str) {
        if let Ok(value) = PropertyValue::CreateString(&HSTRING::from(text)) {
            let _ = self.xaml.SetContent(&value);
        }
    }
}

/// Windows-only regression test for the `AnyView::measure`/`arrange` `Width`/`Height`-stickiness
/// bug — see that method's own doc comment and `docs/agents/winui3_current_state.md`'s "`AnyView::
/// measure` resets `Width`/`Height` to `NaN`..." section for the full root cause. Needs a real,
/// fully-hosted `Application` (via `crate::application::run`/the C++/WinRT shim) — not just COM/
/// Bootstrap `init()` — because `Button`'s default style/template only resolves once `Application.
/// Resources` actually has `XamlControlsResources` merged in; without that, `Button::new()` would
/// either fail outright or always measure at 0 regardless of this bug, defeating the test.
#[cfg(test)]
mod button_measure_regression_tests {
    use super::*;
    use elwindui_core::base::{Rect, Size as CoreSize};

    #[test]
    fn button_recovers_a_nonzero_natural_width_after_a_zero_size_arrange() {
        // `RoutedEventHandler::new`'s generated wrapper requires `Send`, whereas `AnyView`/`Rc` are
        // deliberately UI-thread-local (same constraint `application::run`'s own `STARTUP`/
        // `WINDOWS` TLS work around — see that module's doc comment). Keeping both the view and the
        // recorded width in TLS lets the `Loaded` handler below capture nothing and stay genuinely
        // zero-argument, rather than moving a non-`Send` `AnyView` into the callback directly.
        thread_local! {
            static VIEW: RefCell<Option<AnyView>> = const { RefCell::new(None) };
            static WIDTH: RefCell<Option<f32>> = const { RefCell::new(None) };
        }

        crate::init().expect("elwindui_backend_winui3::init");

        crate::application::run(move || {
            // A bare, unparented `FrameworkElement` never resolves real text metrics (no
            // `XamlRoot`) — attach it to a real `Window.Content` first, exactly like
            // `reconcile_native_children` does in the real render path, so `Measure()` here
            // exercises the same conditions a real relayout pass does.
            let window = XamlWindow::new().expect("Window::new");
            let canvas = Canvas::new().expect("Canvas::new");
            let _ = window.SetContent(&canvas);

            let button = InnerButton::new();
            // A genuinely distinguishing label, not "Save"/"Open" — both already happen to exceed
            // the Fluent `Button`'s minimum width, which wouldn't by itself prove content is
            // contributing to the measured size (see this test's own history in
            // docs/agents/winui3_current_state.md).
            button.set_text("a very long button label");
            let view = button.handle();
            let element = view.as_element();
            let _ = canvas
                .Children()
                .expect("Canvas.Children")
                .Append(&element);
            VIEW.with(|slot| *slot.borrow_mut() = Some(view));

            // A `FrameworkElement` isn't genuinely ready to report real content-driven
            // `DesiredSize`s until its `Loaded` event fires (template application/text layout are
            // only guaranteed complete by then) — `Activate()` alone doesn't guarantee this
            // synchronously, and this backend's real relayout passes only ever happen once a
            // window is already live for the same reason. Do the actual arrange/measure sequence
            // from inside `Loaded` so this test reflects genuine already-connected-window
            // conditions instead of an arbitrary earlier point.
            let loaded = RoutedEventHandler::new(move |_, _| {
                VIEW.with(|slot| {
                    if let Some(mut view) = slot.borrow_mut().take() {
                        // Reproduces the exact poisoning condition this regression guards
                        // against: the very first relayout pass runs before the window has a real
                        // size, so `arrange` sets `Width`/`Height` to 0 — and, without the fix,
                        // every subsequent `measure()` would report 0 forever after regardless of
                        // how much `available` space or content follows.
                        view.arrange(Rect { x: 0.0, y: 0.0, width: 0.0, height: 0.0 });
                        let desired = view.measure(CoreSize { width: 500.0, height: 200.0 });
                        WIDTH.with(|slot| *slot.borrow_mut() = Some(desired.width));
                    }
                });

                bindings::Microsoft::UI::Xaml::Application::Current()?.Exit()?;
                Ok(())
            });
            let _ = element.Loaded(&loaded);
            let _ = window.Activate();
        });

        let width = WIDTH
            .with(|slot| *slot.borrow())
            .expect("the Loaded handler should have run and recorded a width");
        assert!(
            width > 10.0,
            "Button must recover a nonzero natural width after a zero-size arrange, got {width}"
        );
    }
}
