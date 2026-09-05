//! The XAML `Button` and its click handler.

use crate::bindings::Microsoft::UI::Xaml::Controls::{Button as XamlButton, Control};
use crate::bindings::Microsoft::UI::Xaml::Input::{
    KeyboardAccelerator, KeyboardAcceleratorInvokedEventArgs,
};
use crate::bindings::Microsoft::UI::Xaml::Media::Brush;
use crate::bindings::Microsoft::UI::Xaml::{Application, RoutedEventHandler, Style};
use crate::ffi::{AnyView, invoke_ui_event_callback, register_ui_event_callback};
use elwindui_core::ui::ButtonRole;
use std::cell::RefCell;
use std::rc::Rc;
use windows::Foundation::PropertyValue;
use windows::Foundation::TypedEventHandler;
use windows::System::VirtualKey;
use windows::core::{HSTRING, Interface};

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

    /// Applies a `ButtonRole`'s native emphasis, always resetting the previous role's treatment
    /// first so switching roles at runtime can't leave a stale style behind.
    ///
    /// Asymmetric with AppKit, and honestly so:
    ///
    /// - `Primary` maps to the real Fluent `AccentButtonStyle`, looked up from the application
    ///   resources — the direct counterpart of AppKit's accent `bezelColor`.
    /// - `Destructive` has **no** WinUI 3 built-in equivalent of AppKit's `hasDestructiveAction`.
    ///   Fluent expects destructive intent to be carried by wording and confirmation, not by a
    ///   stock red button style. Rather than invent one, this clears the style and sets the
    ///   foreground to the system critical brush, which is the closest honest approximation.
    ///   Recorded as a known gap in `docs/status/control_status.md`, the same way
    ///   `PasswordBox::reveal_enabled`'s reverse asymmetry already is.
    pub(crate) fn set_role(&self, role: ButtonRole) {
        let style = match role {
            ButtonRole::Primary => lookup_resource::<Style>("AccentButtonStyle"),
            ButtonRole::Normal | ButtonRole::Destructive => None,
        };
        let _ = self.xaml.SetStyle(style.as_ref());

        if role == ButtonRole::Destructive {
            let foreground = lookup_resource::<Brush>("SystemFillColorCriticalBrush");
            let _ = self.xaml.SetForeground(foreground.as_ref());
        } else if let Ok(control) = self.xaml.clone().cast::<Control>() {
            // `SetForeground(None)` creates a local null DependencyProperty value and therefore
            // hides AccentButtonStyle's own foreground. Clear the local value so the active
            // Fluent style supplies a readable theme-aware label color.
            let _ = crate::render::clear_control_foreground(&control);
        }
    }

    /// Makes this the window's default button, so Enter activates it.
    ///
    /// WinUI 3's `Button` has no `IsDefault` (that lives on `ContentDialog`'s buttons only), so
    /// this uses the general mechanism: an `Enter` `KeyboardAccelerator`. The accelerator is added
    /// once and removed when unset, rather than accumulating one per call.
    pub(crate) fn set_is_default(&self, is_default: bool) {
        let Ok(accelerators) = self.xaml.KeyboardAccelerators() else {
            return;
        };
        let _ = accelerators.Clear();
        if !is_default {
            return;
        }
        if let Ok(accelerator) = KeyboardAccelerator::new() {
            let _ = accelerator.SetKey(VirtualKey::Enter);
            let callback = self.on_click.clone();
            let callback_id = register_ui_event_callback(Rc::new(move || {
                if let Some(callback) = callback.borrow().as_ref() {
                    callback();
                }
            }));
            let _ = accelerator.Invoked(&TypedEventHandler::<
                KeyboardAccelerator,
                KeyboardAcceleratorInvokedEventArgs,
            >::new(move |_, args| {
                invoke_ui_event_callback(callback_id);
                if let Some(args) = args.cloned() {
                    let _ = args.SetHandled(true);
                }
                Ok(())
            }));
            let _ = accelerators.Append(&accelerator);
        }
    }
}

/// Resolves a Fluent theme resource by key from `Application.Current.Resources`.
///
/// Returns `None` rather than panicking when the key is missing or has another type: the Fluent
/// resource dictionary is only fully populated once `XamlControlsResources` is merged in, and a
/// button that renders unstyled is a far better failure than one that aborts the app.
fn lookup_resource<T: Interface>(key: &str) -> Option<T> {
    let resources = Application::Current().ok()?.Resources().ok()?;
    let key = PropertyValue::CreateString(&HSTRING::from(key)).ok()?;
    let value = resources.Lookup(&key).ok()?;
    value.cast::<T>().ok()
}

/// Windows-only hosted-XAML regression tests. The `AnyView::measure`/`arrange` `Width`/`Height`
/// stickiness case below needs a real `Application`; the text-style and Window lifecycle checks
/// intentionally share that one application instance because WinUI 3 cannot be started twice in
/// one test process.
/// bug — see that method's own doc comment and `docs/design/backends/winui3_backend_design.md`'s "`AnyView::
/// measure` resets `Width`/`Height` to `NaN`..." section for the full root cause. Needs a real,
/// fully-hosted `Application` (via `crate::application::run`/the C++/WinRT shim) — not just COM/
/// Bootstrap `init()` — because `Button`'s default style/template only resolves once `Application.
/// Resources` actually has `XamlControlsResources` merged in; without that, `Button::new()` would
/// either fail outright or always measure at 0 regardless of this bug, defeating the test.
#[cfg(test)]
mod hosted_xaml_regression_tests {
    use super::*;
    use crate::bindings::Microsoft::UI::Xaml::Controls::{
        Canvas, Control, TextBlock, TextBox as XamlTextBox, ToolTip as XamlToolTip, ToolTipService,
    };
    use crate::bindings::Microsoft::UI::Xaml::Media::{
        FontFamily as XamlFontFamily, SolidColorBrush,
    };
    use crate::bindings::Microsoft::UI::Xaml::Window as XamlWindow;
    use crate::bindings::winui_text::{FontStretch as XamlFontStretch, FontStyle as XamlFontStyle};
    use crate::inner::InnerWindow;
    use crate::render::{
        WinUi3TextBackend, apply_cascaded_text_style_to_control, apply_text_style_to_control,
        apply_text_style_to_text_block, apply_text_style_to_text_block_with_foreground,
    };
    use elwindui_core::base::{Rect, Size as CoreSize};
    use elwindui_core::graphics::{
        Brush, CascadedTextStyle, Color, ComputedTextStyle, FontFamily, FontStretch, FontStyle,
        FontWeight, TextBackend, TextMeasureRequest, TextWrapping,
    };
    use windows::Foundation::IPropertyValue;
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

        // An absent ElwindUI foreground must remove this local blue brush rather than retaining
        // it on the reused XAML child. The resource-backed value is intentionally not compared to
        // a fixed RGB value: its exact color is owned by the active Windows appearance.
        apply_text_style_to_text_block_with_foreground(&text, &style, None)
            .expect("clear TextBlock foreground");
        let cleared_foreground: SolidColorBrush = text
            .Foreground()
            .expect("cleared TextBlock.Foreground")
            .cast()
            .expect("solid cleared foreground brush");
        let cleared_color = cleared_foreground
            .Color()
            .expect("cleared SolidColorBrush.Color");
        assert_ne!(
            (cleared_color.R, cleared_color.G, cleared_color.B),
            (0, 102, 204),
            "clear must not retain the prior explicit foreground"
        );

        let native = XamlTextBox::new().expect("TextBox::new");
        let control: Control = native.clone().cast().expect("TextBox implements Control");
        let default_font_size = control.FontSize().expect("default Control.FontSize");
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

        // Clearing every local text DependencyProperty (an unset `CascadedTextStyle` field)
        // rather than materializing a fixed ElwindUI default. The resolved value therefore
        // returns to the live XAML theme resource on the same reused control.
        apply_cascaded_text_style_to_control(&control, &CascadedTextStyle::default())
            .expect("clear native control text style");
        assert_eq!(
            control.FontSize().expect("cleared Control.FontSize"),
            default_font_size
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
    fn hosted_button_text_and_window_lifecycle_regressions_work() {
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
            // docs/design/backends/winui3_backend_design.md).
            button.set_text("a very long button label");
            let view = button.handle();
            view.set_tooltip(Some("hosted tooltip"))
                .expect("set hosted tooltip");
            let tooltip: XamlToolTip = ToolTipService::GetToolTip(&view.as_element())
                .expect("get hosted tooltip")
                .cast()
                .expect("tooltip is a native ToolTip");
            let tooltip: IPropertyValue = tooltip
                .Content()
                .expect("get hosted tooltip content")
                .cast()
                .expect("tooltip content is a boxed string");
            assert_eq!(
                tooltip
                    .GetString()
                    .expect("read hosted tooltip")
                    .to_string_lossy(),
                "hosted tooltip"
            );
            button.set_is_default(true);
            let accelerators = button
                .xaml
                .KeyboardAccelerators()
                .expect("Button.KeyboardAccelerators");
            assert_eq!(accelerators.Size().expect("accelerator count"), 1);
            assert_eq!(
                accelerators
                    .GetAt(0)
                    .expect("default accelerator")
                    .Key()
                    .expect("default accelerator key"),
                VirtualKey::Enter
            );
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
                crate::inner::menu::live_menu_item_icon_tests::
                    live_inner_menu_item_icon_set_replace_clear();
                crate::inner::menu::live_menu_item_icon_tests::
                    create_flyout_snapshots_icon_onto_a_distinct_realization();
                crate::inner::menu::live_menu_item_icon_tests::
                    failed_icon_conversion_does_not_remove_the_action();

                crate::app::reset_window_lifecycle_test_state();
                let lifecycle_window = InnerWindow::new();

                lifecycle_window.show();
                assert!(
                    lifecycle_window.is_visible_for_test(),
                    "show() must make the AppWindow visible"
                );
                assert_eq!(crate::app::retained_window_count_for_test(), 1);

                lifecycle_window.hide();
                assert!(
                    !lifecycle_window.is_visible_for_test(),
                    "hide() must make the AppWindow invisible"
                );
                assert_eq!(
                    crate::app::retained_window_count_for_test(),
                    1,
                    "hide() must retain the existing native window"
                );

                lifecycle_window.show();
                assert!(
                    lifecycle_window.is_visible_for_test(),
                    "show() after hide() must make the same AppWindow visible again"
                );
                assert_eq!(
                    crate::app::retained_window_count_for_test(),
                    1,
                    "re-showing must not retain the same native window twice"
                );

                lifecycle_window.close();
                assert!(
                    !lifecycle_window.is_visible_for_test(),
                    "close() must leave no visible native window"
                );
                assert_eq!(
                    crate::app::retained_window_count_for_test(),
                    0,
                    "the Closed handler must release the retained native window"
                );
                assert_eq!(
                    crate::app::release_window_call_count_for_test(),
                    1,
                    "programmatic close() must reach release_window exactly once"
                );
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
