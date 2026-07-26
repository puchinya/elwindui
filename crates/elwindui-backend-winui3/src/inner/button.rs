//! The XAML `Button` and its click handler.

use crate::bindings::Microsoft::UI::Xaml::Controls::Button as XamlButton;
use crate::bindings::Microsoft::UI::Xaml::RoutedEventHandler;
use crate::ffi::{AnyView, invoke_ui_event_callback, register_ui_event_callback};
use std::cell::RefCell;
use std::rc::Rc;
use windows::Foundation::PropertyValue;
use windows::core::HSTRING;

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
                if let Some(callback) = callback.borrow().as_ref() {
                    callback();
                }
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

/// Windows-only hosted-XAML regression tests. The `AnyView::measure`/`arrange` `Width`/`Height`
/// stickiness case below needs a real `Application`, and the text-style checks intentionally share
/// that one application instance because WinUI 3 cannot be started twice in one test process.
/// bug — see that method's own doc comment and `docs/agents/winui3_current_state.md`'s "`AnyView::
/// measure` resets `Width`/`Height` to `NaN`..." section for the full root cause. Needs a real,
/// fully-hosted `Application` (via `crate::application::run`/the C++/WinRT shim) — not just COM/
/// Bootstrap `init()` — because `Button`'s default style/template only resolves once `Application.
/// Resources` actually has `XamlControlsResources` merged in; without that, `Button::new()` would
/// either fail outright or always measure at 0 regardless of this bug, defeating the test.
#[cfg(test)]
mod hosted_xaml_regression_tests {
    use super::*;
    use crate::bindings;
    use crate::bindings::Microsoft::UI::Xaml::Controls::{
        Canvas, Control, TextBlock, TextBox as XamlTextBox,
    };
    use crate::bindings::Microsoft::UI::Xaml::Media::{
        FontFamily as XamlFontFamily, SolidColorBrush,
    };
    use crate::bindings::Microsoft::UI::Xaml::Window as XamlWindow;
    use crate::bindings::winui_text::{FontStretch as XamlFontStretch, FontStyle as XamlFontStyle};
    use crate::render::{
        WinUi3TextBackend, apply_text_style_to_control, apply_text_style_to_text_block,
    };
    use elwindui_core::base::{Rect, Size as CoreSize};
    use elwindui_core::graphics::{
        Brush, Color, ComputedTextStyle, FontFamily, FontStretch, FontStyle, FontWeight,
        TextBackend, TextMeasureRequest, TextWrapping,
    };
    use windows::core::{HSTRING, Interface};

    fn assert_text_style_round_trip(canvas: &Canvas) {
        let style = ComputedTextStyle {
            font_family: FontFamily::new("Consolas, Segoe UI"),
            font_size: 24.0,
            font_weight: FontWeight(650),
            font_style: FontStyle::Italic,
            font_stretch: FontStretch::SemiExpanded,
            character_spacing: 80,
            foreground: Brush::Solid(Color::rgb(0, 102, 204)),
        };

        let text = TextBlock::new().expect("TextBlock::new");
        text.SetText(&HSTRING::from("The quick brown fox"))
            .expect("TextBlock.SetText");
        apply_text_style_to_text_block(&text, &style).expect("apply TextBlock text style");
        let text_element: crate::bindings::Microsoft::UI::Xaml::FrameworkElement = text
            .clone()
            .cast()
            .expect("TextBlock implements FrameworkElement");
        canvas
            .Children()
            .expect("Canvas.Children")
            .Append(&text_element)
            .expect("append TextBlock");

        assert_eq!(
            text.FontFamily()
                .expect("TextBlock.FontFamily")
                .Source()
                .expect("FontFamily.Source")
                .to_string_lossy(),
            "Consolas, Segoe UI"
        );
        assert_eq!(text.FontSize().expect("TextBlock.FontSize"), 24.0);
        assert_eq!(text.FontWeight().expect("TextBlock.FontWeight").Weight, 650);
        assert_eq!(
            text.FontStyle().expect("TextBlock.FontStyle"),
            XamlFontStyle::Italic
        );
        assert_eq!(
            text.FontStretch().expect("TextBlock.FontStretch"),
            XamlFontStretch::SemiExpanded
        );
        assert_eq!(
            text.CharacterSpacing().expect("TextBlock.CharacterSpacing"),
            80
        );
        let foreground: SolidColorBrush = text
            .Foreground()
            .expect("TextBlock.Foreground")
            .cast()
            .expect("solid foreground brush");
        let color = foreground.Color().expect("SolidColorBrush.Color");
        assert_eq!((color.R, color.G, color.B), (0, 102, 204));

        let native = XamlTextBox::new().expect("TextBox::new");
        let control: Control = native.clone().cast().expect("TextBox implements Control");
        apply_text_style_to_control(&control, &style).expect("apply native control text style");
        assert_eq!(control.FontSize().expect("Control.FontSize"), 24.0);
        assert_eq!(
            control.FontWeight().expect("Control.FontWeight").Weight,
            650
        );
        assert_eq!(
            control.FontStyle().expect("Control.FontStyle"),
            XamlFontStyle::Italic
        );

        // A named family must be replaceable with the backend default on the same reused element.
        let system_style = ComputedTextStyle {
            font_family: FontFamily::system(),
            ..style.clone()
        };
        apply_text_style_to_text_block(&text, &system_style).expect("reset TextBlock font family");
        let expected_system = XamlFontFamily::XamlAutoFontFamily()
            .expect("XamlAutoFontFamily")
            .Source()
            .expect("XamlAutoFontFamily.Source")
            .to_string_lossy();
        assert_eq!(
            text.FontFamily()
                .expect("reset TextBlock.FontFamily")
                .Source()
                .expect("reset FontFamily.Source")
                .to_string_lossy(),
            expected_system
        );

        let backend = WinUi3TextBackend;
        let small = ComputedTextStyle {
            font_size: 12.0,
            ..system_style.clone()
        };
        let large = ComputedTextStyle {
            font_size: 24.0,
            ..system_style.clone()
        };
        let spaced = ComputedTextStyle {
            character_spacing: 240,
            ..large.clone()
        };
        fn request(style: &ComputedTextStyle) -> TextMeasureRequest<'_> {
            TextMeasureRequest {
                text: "The quick brown fox",
                style,
                available: CoreSize {
                    width: f32::INFINITY,
                    height: f32::INFINITY,
                },
                wrapping: TextWrapping::NoWrap,
                alignment: elwindui_core::graphics::TextAlignment::Left,
                max_lines: None,
                scale: 1.0,
            }
        }
        let small_measure = backend.measure_text(&request(&small));
        let large_measure = backend.measure_text(&request(&large));
        let spaced_measure = backend.measure_text(&request(&spaced));
        assert!(large_measure.size.height > small_measure.size.height);
        assert!(spaced_measure.size.width > large_measure.size.width);

        let missing = ComputedTextStyle {
            font_family: FontFamily::new("Definitely Not A Real Font Family XYZ"),
            ..large
        };
        assert!(backend.measure_text(&request(&missing)).size.width > 0.0);
    }

    #[test]
    fn button_measure_and_text_style_round_trip_work_in_a_hosted_application() {
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
            let _ = canvas.Children().expect("Canvas.Children").Append(&element);
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
                        view.arrange(Rect {
                            x: 0.0,
                            y: 0.0,
                            width: 0.0,
                            height: 0.0,
                        });
                        let desired = view.measure(CoreSize {
                            width: 500.0,
                            height: 200.0,
                        });
                        WIDTH.with(|slot| *slot.borrow_mut() = Some(desired.width));
                    }
                });

                assert_text_style_round_trip(&canvas);

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
