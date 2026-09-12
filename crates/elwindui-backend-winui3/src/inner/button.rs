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
    use crate::bindings::Microsoft;
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
    use elwindui_core::ui::{UIElement, UIElementExt, WindowExt};
    use std::cell::{Cell, RefCell};
    use std::rc::{Rc, Weak};
    use windows::Foundation::IPropertyValue;
    use windows::core::{HSTRING, Interface};

    thread_local! {
        static TEST_CALLBACK_OWNERS: RefCell<Vec<crate::ffi::UiCallbackRegistryOwner>> = const { RefCell::new(Vec::new()) };
    }

    #[elwindui_macros::class(inherits = elwindui_core::ui::UIElement)]
    struct ReentrantMeasureProbe {
        reentered: Cell<bool>,
        self_weak: RefCell<Weak<dyn ReentrantMeasureProbeExt>>,
    }

    #[elwindui_macros::class]
    impl ReentrantMeasureProbe {
        #[overrides]
        fn measure_override(&self, _available: CoreSize) -> CoreSize {
            if !self.reentered.replace(true) {
                if let Some(probe) = self.self_weak.borrow().upgrade() {
                    let element: Rc<dyn UIElementExt> = probe;
                    element.invalidate_measure();
                }
            }
            CoreSize {
                width: 160.0,
                height: 48.0,
            }
        }

        fn construct() -> Self {
            Self {
                base: UIElement::construct(),
                reentered: Cell::new(false),
                self_weak: RefCell::new(__self_weak.clone()),
            }
        }
    }

    fn enqueue_test_callback(callback: Rc<dyn Fn()>) {
        let owner = crate::ffi::UiCallbackRegistryOwner::default();
        let callback_id = owner.register_one_shot_event(callback);
        TEST_CALLBACK_OWNERS.with(|owners| owners.borrow_mut().push(owner));
        let queue = Microsoft::UI::Dispatching::DispatcherQueue::GetForCurrentThread()
            .expect("DispatcherQueue::GetForCurrentThread");
        let handler = Microsoft::UI::Dispatching::DispatcherQueueHandler::new(move || {
            invoke_ui_event_callback(callback_id);
            Ok(())
        });
        let _ = queue.TryEnqueue(&handler);
    }

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
            static RELAYOUT_PASS_COUNT_BEFORE_DRAIN: RefCell<Option<u32>> = const { RefCell::new(None) };
            static RELAYOUT_PASS_COUNT_AFTER_DRAIN: RefCell<Option<u32>> = const { RefCell::new(None) };
            static RELAYOUT_PASS_COUNT_AFTER_500: RefCell<Option<u32>> = const { RefCell::new(None) };
            static LIFECYCLE_VISIBLE_AFTER_CLOSE: RefCell<Option<bool>> = const { RefCell::new(None) };
            static RETAINED_COUNT_AFTER_CLOSE: RefCell<Option<usize>> = const { RefCell::new(None) };
            static RELEASE_CALL_COUNT_AFTER_CLOSE: RefCell<Option<usize>> = const { RefCell::new(None) };
            static SCHEDULER_INITIAL_HISTORY: RefCell<Option<Vec<crate::host::RelayoutRealizationRecord>>> = const { RefCell::new(None) };
            static SCHEDULER_STRONGEST_HISTORY: RefCell<Option<Vec<crate::host::RelayoutRealizationRecord>>> = const { RefCell::new(None) };
            static SCHEDULER_RENDER_HISTORY: RefCell<Option<Vec<crate::host::RelayoutRealizationRecord>>> = const { RefCell::new(None) };
            static SCHEDULER_FLUSH_HISTORY: RefCell<Option<Vec<crate::host::RelayoutRealizationRecord>>> = const { RefCell::new(None) };
            static SCHEDULER_CALLBACK_BASELINE: RefCell<Option<usize>> = const { RefCell::new(None) };
            static SCHEDULER_CALLBACK_COUNT_AFTER_QUEUE: RefCell<Option<usize>> = const { RefCell::new(None) };
            static SCHEDULER_CALLBACK_COUNT_AFTER_FLUSH: RefCell<Option<usize>> = const { RefCell::new(None) };
            static SCHEDULER_CALLBACK_COUNT_AFTER_STALE: RefCell<Option<usize>> = const { RefCell::new(None) };
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
                // Issue #254: `InnerWindow` stores only the final owner's `Weak<dyn WindowExt>`;
                // the application registry becomes the strong lifetime authority after show().
                let lifecycle_owner: Rc<dyn elwindui_core::ui::WindowExt> =
                    crate::native_ui::Window::new();
                let weak_owner_a = Rc::downgrade(&lifecycle_owner);
                let lifecycle_window = InnerWindow::new(weak_owner_a.clone());

                lifecycle_window.show();
                drop(lifecycle_owner);
                assert!(
                    lifecycle_window.is_visible_for_test(),
                    "show() must make the AppWindow visible"
                );
                assert_eq!(crate::app::retained_window_count_for_test(), 1);
                assert!(
                    weak_owner_a.upgrade().is_some(),
                    "the application registry must keep A alive after the caller drops its Rc"
                );

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

                // Exercise the real backend Window path as well as the scheduler host above:
                // `Window::new()` wires `__self_weak` into its owned `InnerWindow`, so these
                // caller-side Rc drops prove the application registry is the lifetime authority
                // for the object actual callers construct.
                let real_window_a = crate::native_ui::Window::new();
                let weak_real_a = Rc::downgrade(&real_window_a);
                real_window_a.show();
                drop(real_window_a);
                assert!(weak_real_a.upgrade().is_some());
                assert_eq!(crate::app::retained_window_count_for_test(), 2);

                let real_window_b = crate::native_ui::Window::new();
                let weak_real_b = Rc::downgrade(&real_window_b);
                real_window_b.show();
                drop(real_window_b);
                assert!(weak_real_a.upgrade().is_some());
                assert!(weak_real_b.upgrade().is_some());
                assert_eq!(crate::app::retained_window_count_for_test(), 3);

                // A never-shown Window must not create or release a registry entry while the
                // unrelated shown Windows remain alive.
                let real_window_c = crate::native_ui::Window::new();
                let weak_real_c = Rc::downgrade(&real_window_c);
                let releases_before_real_c = crate::app::release_window_call_count_for_test();
                assert_eq!(crate::app::retained_window_count_for_test(), 3);
                real_window_c.close();
                assert_eq!(
                    crate::app::release_window_call_count_for_test(),
                    releases_before_real_c
                );
                drop(real_window_c);
                assert!(weak_real_c.upgrade().is_none());

                // Closing one real Window releases only its own application entry.
                {
                    let real_window_a = weak_real_a.upgrade().expect("real Window A retained");
                    real_window_a.close();
                }
                assert!(weak_real_a.upgrade().is_none());
                assert!(weak_real_b.upgrade().is_some());
                assert_eq!(crate::app::retained_window_count_for_test(), 2);
                {
                    let real_window_b = weak_real_b.upgrade().expect("real Window B retained");
                    real_window_b.close();
                }
                assert!(weak_real_b.upgrade().is_none());
                assert_eq!(crate::app::retained_window_count_for_test(), 1);
                assert_eq!(crate::app::release_window_call_count_for_test(), 2);

                // Scheduler-level Issue #261 regressions. The probe's first measure raises one
                // same-host invalidation while the host is already in progress; the host must own
                // that rerun through `run_coalesced`, without contaminating the next queued batch.
                // All assertions remain outside native callbacks below.
                let scheduler_probe = ReentrantMeasureProbe::new();
                crate::host::reset_relayout_realization_history_for_test();
                lifecycle_window.set_content(scheduler_probe.clone());
                SCHEDULER_INITIAL_HISTORY.with(|slot| {
                    *slot.borrow_mut() = Some(crate::host::relayout_realization_history_for_test())
                });

                let callback_baseline = crate::ffi::ui_event_callback_count();
                SCHEDULER_CALLBACK_BASELINE
                    .with(|slot| *slot.borrow_mut() = Some(callback_baseline));
                scheduler_probe.invalidate_render();
                scheduler_probe.invalidate_arrange();
                scheduler_probe.invalidate_render();
                scheduler_probe.invalidate_measure();
                scheduler_probe.invalidate_arrange();
                SCHEDULER_CALLBACK_COUNT_AFTER_QUEUE
                    .with(|slot| *slot.borrow_mut() = Some(crate::ffi::ui_event_callback_count()));

                let scheduler_probe_for_render = scheduler_probe.clone();
                let lifecycle_window_for_scheduler = lifecycle_window.clone();
                enqueue_test_callback(Rc::new(move || {
                    SCHEDULER_STRONGEST_HISTORY.with(|slot| {
                        *slot.borrow_mut() =
                            Some(crate::host::relayout_realization_history_for_test())
                    });

                    // The previous batch has been consumed/reset. A new Render-only request must
                    // not inherit the reentrant/strongest Measure claim from that earlier batch.
                    scheduler_probe_for_render.invalidate_render();
                    let scheduler_probe_for_flush = scheduler_probe_for_render.clone();
                    let lifecycle_window_for_flush = lifecycle_window_for_scheduler.clone();
                    enqueue_test_callback(Rc::new(move || {
                        SCHEDULER_RENDER_HISTORY.with(|slot| {
                            *slot.borrow_mut() =
                                Some(crate::host::relayout_realization_history_for_test())
                        });

                        // Queue one ordinary batch, consume it synchronously, and then enqueue a
                        // follow-up callback after the stale dispatcher ticket. That callback can
                        // assert both one realization and one-shot callback cleanup.
                        let flush_baseline = crate::ffi::ui_event_callback_count();
                        SCHEDULER_CALLBACK_BASELINE
                            .with(|slot| *slot.borrow_mut() = Some(flush_baseline));
                        scheduler_probe_for_flush.invalidate_measure();
                        SCHEDULER_CALLBACK_COUNT_AFTER_QUEUE.with(|slot| {
                            *slot.borrow_mut() = Some(crate::ffi::ui_event_callback_count())
                        });
                        scheduler_probe_for_flush.flush_interactive_relayout();
                        SCHEDULER_FLUSH_HISTORY.with(|slot| {
                            *slot.borrow_mut() =
                                Some(crate::host::relayout_realization_history_for_test())
                        });
                        SCHEDULER_CALLBACK_COUNT_AFTER_FLUSH.with(|slot| {
                            *slot.borrow_mut() = Some(crate::ffi::ui_event_callback_count())
                        });

                        let lifecycle_window_for_burst = lifecycle_window_for_flush.clone();
                        enqueue_test_callback(Rc::new(move || {
                            SCHEDULER_CALLBACK_COUNT_AFTER_STALE.with(|slot| {
                                *slot.borrow_mut() = Some(crate::ffi::ui_event_callback_count())
                            });

                            // Existing hosted burst/sibling coverage is deliberately kept in the
                            // same real Application test. Run it once with 20 writes and again
                            // with 500 writes; each host must still realize exactly one pass.
                            use elwindui_core::ui::TextBlockExt;
                            let probe = elwindui_core::ui::TextBlock::new();
                            lifecycle_window_for_burst.set_content(probe.clone());
                            let sibling_owner: Rc<dyn elwindui_core::ui::WindowExt> =
                                crate::native_ui::Window::new();
                            let weak_owner_b = Rc::downgrade(&sibling_owner);
                            let sibling_window = InnerWindow::new(weak_owner_b.clone());
                            sibling_window.show();
                            drop(sibling_owner);
                            assert_eq!(
                                crate::app::retained_window_count_for_test(),
                                2,
                                "a second shown window must be retained independently"
                            );
                            assert!(weak_owner_a.upgrade().is_some(), "A must still be alive");
                            assert!(weak_owner_b.upgrade().is_some(), "B must still be alive");

                            // T12: a never-shown Window creates no registry entry and cannot
                            // release either retained sibling.
                            let owner_c: Rc<dyn elwindui_core::ui::WindowExt> =
                                crate::native_ui::Window::new();
                            let weak_owner_c = Rc::downgrade(&owner_c);
                            let lifecycle_window_c = InnerWindow::new(weak_owner_c.clone());
                            assert_eq!(crate::app::retained_window_count_for_test(), 2);
                            let releases_before_c =
                                crate::app::release_window_call_count_for_test();
                            lifecycle_window_c.close();
                            assert_eq!(
                                crate::app::release_window_call_count_for_test(),
                                releases_before_c,
                                "closing a never-shown Window must not release another Window"
                            );
                            drop(owner_c);
                            assert!(weak_owner_c.upgrade().is_none());
                            let sibling_probe = elwindui_core::ui::TextBlock::new();
                            sibling_window.set_content(sibling_probe.clone());

                            crate::host::reset_relayout_static_pass_count_for_test();
                            for i in 0..20 {
                                probe.set_text(&format!("probe {i}"));
                            }
                            sibling_probe.set_text("sibling probe");
                            let pass_count_before_drain =
                                crate::host::relayout_static_pass_count_for_test();
                            RELAYOUT_PASS_COUNT_BEFORE_DRAIN
                                .with(|slot| *slot.borrow_mut() = Some(pass_count_before_drain));

                            let lifecycle_window_for_500 = lifecycle_window_for_burst.clone();
                            enqueue_test_callback(Rc::new(move || {
                                RELAYOUT_PASS_COUNT_AFTER_DRAIN.with(|slot| {
                                    *slot.borrow_mut() =
                                        Some(crate::host::relayout_static_pass_count_for_test())
                                });

                                crate::host::reset_relayout_static_pass_count_for_test();
                                for i in 0..500 {
                                    probe.set_text(&format!("probe-500 {i}"));
                                }
                                sibling_probe.set_text("sibling probe 500");

                                let sibling_window_for_close = sibling_window.clone();
                                let weak_owner_a_for_close = weak_owner_a.clone();
                                let weak_owner_b_for_close = weak_owner_b.clone();
                                let lifecycle_window_for_close = lifecycle_window_for_500.clone();
                                enqueue_test_callback(Rc::new(move || {
                                    RELAYOUT_PASS_COUNT_AFTER_500.with(|slot| {
                                        *slot.borrow_mut() =
                                            Some(crate::host::relayout_static_pass_count_for_test())
                                    });
                                    sibling_window_for_close.close();
                                    drop(sibling_window_for_close);
                                    assert_eq!(
                                        crate::app::retained_window_count_for_test(),
                                        1,
                                        "closing B must release only B while A remains retained"
                                    );
                                    assert!(
                                        weak_owner_b_for_close.upgrade().is_none(),
                                        "B's owner must drop after its registry entry is released"
                                    );
                                    assert!(
                                        weak_owner_a_for_close.upgrade().is_some(),
                                        "A must remain alive while B is closed"
                                    );
                                    lifecycle_window_for_close.close();
                                    LIFECYCLE_VISIBLE_AFTER_CLOSE.with(|slot| {
                                        *slot.borrow_mut() =
                                            Some(lifecycle_window_for_close.is_visible_for_test())
                                    });
                                    RETAINED_COUNT_AFTER_CLOSE.with(|slot| {
                                        *slot.borrow_mut() =
                                            Some(crate::app::retained_window_count_for_test())
                                    });
                                    RELEASE_CALL_COUNT_AFTER_CLOSE.with(|slot| {
                                        *slot.borrow_mut() =
                                            Some(crate::app::release_window_call_count_for_test())
                                    });
                                }));
                            }));
                        }));
                    }));
                }));

                Ok(())
            });
            let _ = element.Loaded(&loaded);
            let _ = window.Activate();
        });
        TEST_CALLBACK_OWNERS.with(|owners| owners.borrow_mut().clear());

        let width = WIDTH
            .with(|slot| *slot.borrow())
            .expect("the Loaded handler should have run and recorded a width");
        assert!(
            width > 10.0,
            "Button must recover a nonzero natural width after a zero-size arrange, got {width}"
        );

        // Issue #261 regression assertions (deferred from inside native callbacks — see that
        // code's own comment on why panicking there is unsafe).
        let pass_count_before_drain = RELAYOUT_PASS_COUNT_BEFORE_DRAIN
            .with(|slot| *slot.borrow())
            .expect("pass count before drain should have been recorded");
        assert_eq!(
            pass_count_before_drain, 0,
            "many consecutive invalidations against one host, plus an independent invalidation \
             against a sibling host, must not run any real relayout pass before this UI-turn drains"
        );
        let pass_count_after_drain = RELAYOUT_PASS_COUNT_AFTER_DRAIN
            .with(|slot| *slot.borrow())
            .expect("pass count after drain should have been recorded");
        // Each host settles in exactly 1 real pass here (confirmed independent of burst size --
        // 20 vs. 500 `set_text` calls against `probe` both produce the same total: 2). The host
        // does not observe its own native Canvas size output at all: viewport changes enter only
        // through the owner-supplied `TreeHostViewport` API. Consequently this assertion covers
        // only the per-host queued-batch and same-host reentrancy invariants; it does not rely on a
        // Canvas `SizeChanged` event being routed through the scheduler.
        assert_eq!(
            pass_count_after_drain, 2,
            "the coalesced burst against the first host and the independent invalidation against \
             the sibling host must each settle in the same small, bounded number of real passes \
             once this UI-turn drains, regardless of how many invalidations were coalesced into \
             it -- neither host suppresses the other's own pass"
        );
        let pass_count_after_500 = RELAYOUT_PASS_COUNT_AFTER_500
            .with(|slot| *slot.borrow())
            .expect("500-write pass count should have been recorded");
        assert_eq!(
            pass_count_after_500, 2,
            "a 500-write burst must have the same one-pass-per-host result as the 20-write burst"
        );

        let initial_history = SCHEDULER_INITIAL_HISTORY
            .with(|slot| slot.borrow().clone())
            .expect("initial scheduler realization history should have been recorded");
        assert!(
            initial_history.iter().any(|record| {
                record.source == crate::host::RelayoutSource::SetTreeInitial
                    && record.kind == elwindui_core::ui::InvalidationKind::Measure
            }),
            "initial probe content must realize as SetTreeInitial/Measure: {initial_history:?}"
        );
        let strongest_history = SCHEDULER_STRONGEST_HISTORY
            .with(|slot| slot.borrow().clone())
            .expect("strongest-kind scheduler history should have been recorded");
        assert_eq!(
            strongest_history.len(),
            initial_history.len() + 1,
            "Render/Arrange/Render/Measure/Arrange must realize one queued batch"
        );
        assert_eq!(
            strongest_history
                .last()
                .map(|record| (record.source, record.kind)),
            Some((
                crate::host::RelayoutSource::QueuedRequest,
                elwindui_core::ui::InvalidationKind::Measure
            ))
        );
        let render_history = SCHEDULER_RENDER_HISTORY
            .with(|slot| slot.borrow().clone())
            .expect("render scheduler history should have been recorded");
        assert_eq!(
            render_history.len(),
            strongest_history.len() + 1,
            "the subsequent Render-only batch must realize exactly once"
        );
        assert_eq!(
            render_history
                .last()
                .map(|record| (record.source, record.kind)),
            Some((
                crate::host::RelayoutSource::QueuedRequest,
                elwindui_core::ui::InvalidationKind::Render
            )),
            "a reentrant Measure must not contaminate the later Render batch"
        );
        let flush_history = SCHEDULER_FLUSH_HISTORY
            .with(|slot| slot.borrow().clone())
            .expect("flush scheduler history should have been recorded");
        assert_eq!(
            flush_history.len(),
            render_history.len() + 1,
            "interactive flush must realize its pending batch exactly once"
        );
        assert_eq!(
            flush_history
                .last()
                .map(|record| (record.source, record.kind)),
            Some((
                crate::host::RelayoutSource::InteractiveFlush,
                elwindui_core::ui::InvalidationKind::Measure
            ))
        );
        let callback_baseline = SCHEDULER_CALLBACK_BASELINE
            .with(|slot| *slot.borrow())
            .expect("callback baseline should have been recorded");
        let callback_count_after_queue = SCHEDULER_CALLBACK_COUNT_AFTER_QUEUE
            .with(|slot| *slot.borrow())
            .expect("callback count after queue should have been recorded");
        let callback_count_after_flush = SCHEDULER_CALLBACK_COUNT_AFTER_FLUSH
            .with(|slot| *slot.borrow())
            .expect("callback count after flush should have been recorded");
        let callback_count_after_stale = SCHEDULER_CALLBACK_COUNT_AFTER_STALE
            .with(|slot| *slot.borrow())
            .expect("callback count after stale ticket should have been recorded");
        assert_eq!(callback_count_after_queue, callback_baseline + 1);
        assert_eq!(
            callback_count_after_flush, callback_count_after_queue,
            "interactive flush leaves the stale one-shot ticket for later consumption"
        );
        assert_eq!(
            callback_count_after_stale, callback_baseline,
            "the stale callback must be removed without a duplicate realization"
        );
        let lifecycle_visible_after_close = LIFECYCLE_VISIBLE_AFTER_CLOSE
            .with(|slot| *slot.borrow())
            .expect("lifecycle window visibility after close should have been recorded");
        assert!(
            !lifecycle_visible_after_close,
            "close() must leave no visible native window"
        );
        let retained_count_after_close = RETAINED_COUNT_AFTER_CLOSE
            .with(|slot| *slot.borrow())
            .expect("retained window count after close should have been recorded");
        assert_eq!(
            retained_count_after_close, 0,
            "the Closed handler must release every retained native window"
        );
        let release_call_count_after_close = RELEASE_CALL_COUNT_AFTER_CLOSE
            .with(|slot| *slot.borrow())
            .expect("release_window call count after close should have been recorded");
        assert_eq!(
            release_call_count_after_close, 4,
            "programmatic close() on every retained test window must release exactly once each"
        );
    }
}
