//! The Objective-C seam: the `MainThreadMarker` accessor and the erased native-view handle
//! (`AnyView`) every other layer passes around instead of a concrete `NSView` subclass.
//!
//! `AnyView` is re-exported at the crate root because `elwindui-codegen` generates references to
//! `elwindui::backend::AnyView` directly — that path must stay stable.


use elwindui_core::base::AsAny;
use elwindui_core::graphics::{
    Brush, CascadedTextStyle, Color, ComputedTextStyle, TextBackend,
};
use objc2::rc::Retained;
use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSButton, NSScrollView, NSStackView, NSTextField, NSTextView, NSUserInterfaceLayoutOrientation,
    NSView,
};
use objc2_foundation::NSRect;
use std::rc::Rc;

pub(crate) fn mtm() -> MainThreadMarker {
    MainThreadMarker::new().expect("elwindui-backend-appkit must run on the main thread")
}

/// The one flat foreground color a handle can actually apply through a plain (non-attributed)
/// `NSColor` setter — same gradient/image degrade as `render::text::foreground_ns_color` (a
/// `Brush::Solid` is exact, anything else falls back to a representative flat color).
fn materialized_text_style(style: &CascadedTextStyle) -> ComputedTextStyle {
    style.materialize(&crate::render::AppKitTextBackend.default_text_style())
}

fn flat_foreground_nscolor(style: &CascadedTextStyle) -> Retained<objc2_app_kit::NSColor> {
    let Some(foreground) = style.foreground.as_ref() else {
        return objc2_app_kit::NSColor::labelColor();
    };
    let color = match foreground {
        Brush::Solid(color) => *color,
        other => crate::render::first_gradient_stop_color(other).unwrap_or(Color::black()),
    };
    objc2_app_kit::NSColor::colorWithSRGBRed_green_blue_alpha(
        color.r as f64 / 255.0,
        color.g as f64 / 255.0,
        color.b as f64 / 255.0,
        color.a as f64 / 255.0,
    )
}

/// The capability a type needs to be usable as an `AnyView` — implemented once per raw native view
/// type (`Retained<NSScrollView>`/`Retained<NSButton>`/`Retained<NSStackView>`) instead of matched
/// on centrally, so a future native leaf only needs its own `impl AppKitHandle`, never a change to
/// `AnyView` itself or to any `match` over it.
pub(crate) trait AppKitHandle: AsAny {
    fn as_nsview(&self) -> Retained<NSView>;

    /// Returns the ElwindUI standard-token prefix for this concrete native control.
    fn theme_prefix(&self) -> &'static str {
        "native_control"
    }

    /// Deliberately a no-op: a real AppKit native control (`NSButton`/`NSTextField`/`NSTextView`/
    /// `NSTabView`/...) has no supported way to take an arbitrary background color without
    /// abandoning native rendering. Forcing `wantsLayer(true)` on these system-drawn controls to
    /// paint a custom `CALayer` background corrupts their own internal Cocoa-managed drawing —
    /// observed as the control's whole content silently disappearing after a later redraw/
    /// appearance-invalidation pass, not merely the wrong color. Native controls on this backend
    /// therefore always keep the system's own Light/Dark-following background; only non-native,
    /// ElwindUI-painted elements (`Window`/layout backgrounds, `Rectangle`, ...) apply a themed
    /// `Brush` background.
    fn apply_background(&self, _background: Option<&Brush>) {}

    /// Pushes a resolved text style onto the real widget this handle wraps. No-op by default — a
    /// handle with no text of its own (`NSStackView`) simply inherits its font for any *content* it
    /// hosts, not for itself. Called from `NativeControl::sync_text_style`
    /// (`native_ui/control.rs`), itself pulled from `measure_override` — see that method's own doc
    /// comment for why this is pull-based rather than pushed from the storage side.
    fn apply_text_style(&self, _style: &CascadedTextStyle) {}

    /// Whether this handle actually has somewhere to put a text style — used so a font-incapable
    /// native leaf can be told apart from one that silently ignored the request (指示書 §17: never
    /// treat "discarded" as "applied").
    fn supports_text_style(&self) -> bool {
        false
    }
}

impl AppKitHandle for Retained<NSScrollView> {
    fn as_nsview(&self) -> Retained<NSView> {
        Retained::into_super(self.clone())
    }
    fn theme_prefix(&self) -> &'static str {
        if self.supports_text_style() {
            "text_area"
        } else {
            "scroll_view"
        }
    }
    fn apply_text_style(&self, style: &CascadedTextStyle) {
        // `ScrollView`'s own document view is `ElwinduiContentRoot` (a nested tree host, not
        // text) — this only actually does anything when the wrapped leaf is `TextArea`'s
        // `NSTextView`, so it's a natural no-op for `ScrollView` itself.
        let Some(document) = self.documentView() else {
            return;
        };
        let Ok(text_view) = document.downcast::<NSTextView>() else {
            return;
        };
        let materialized = materialized_text_style(style);
        text_view.setFont(Some(&crate::render::ns_font(&materialized)));
        text_view.setTextColor(Some(&flat_foreground_nscolor(style)));
        // `character_spacing` isn't applied here: `NSTextView`'s kerning would need a
        // whole-text-storage attribute rewrite (or `typingAttributes`, which only affects text
        // typed *after* this call) rather than a single property setter the way `NSButton`'s
        // attributed-title rebuild works — narrower scope, documented in
        // `docs/elwindui_font_status.md`, not a silent drop.
    }
    fn supports_text_style(&self) -> bool {
        self.documentView()
            .is_some_and(|d| d.downcast::<NSTextView>().is_ok())
    }
}
impl AppKitHandle for Retained<NSButton> {
    fn as_nsview(&self) -> Retained<NSView> {
        let control: Retained<objc2_app_kit::NSControl> = Retained::into_super(self.clone());
        Retained::into_super(control)
    }
    fn theme_prefix(&self) -> &'static str {
        "button"
    }
    fn apply_text_style(&self, style: &CascadedTextStyle) {
        let materialized = materialized_text_style(style);
        self.setFont(Some(&crate::render::ns_font(&materialized)));
        // A plain `title`/`setFont` pair can't express kerning or an explicit foreground — only
        // an attributed title can. Rebuilding it unconditionally would also silently discard the
        // system's own tinting for special bezel styles, so this only happens when kerning is
        // actually non-zero (the one property a plain title genuinely cannot represent at all).
        if style.character_spacing.is_some_and(|spacing| spacing != 0)
            || style.foreground.is_some()
        {
            let plain = self.title().to_string();
            let attributed = crate::render::attributed_string(
                &plain,
                &materialized,
                style.foreground.as_ref(),
                elwindui_core::ui::TextAlignment::Left,
            );
            self.setAttributedTitle(&attributed);
        } else {
            let plain = self.title();
            self.setTitle(&plain);
        }
    }
    fn supports_text_style(&self) -> bool {
        true
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
    fn theme_prefix(&self) -> &'static str {
        // AppKit uses NSTextField for both TextBox and PasswordBox. Password controls are secure
        // text fields, but both share the same base foreground/background fallback behavior.
        "text_box"
    }
    fn apply_text_style(&self, style: &CascadedTextStyle) {
        let materialized = materialized_text_style(style);
        self.setFont(Some(&crate::render::ns_font(&materialized)));
        self.setTextColor(Some(&flat_foreground_nscolor(style)));
        // Same narrower scope as `NSTextView` above: kerning isn't applied to an editable
        // `NSTextField`'s plain `stringValue` (an `attributedStringValue` rewrite would fight with
        // in-place editing) — documented, not silently dropped.
    }
    fn supports_text_style(&self) -> bool {
        true
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

    /// Forwards to the wrapped handle's own `AppKitHandle::apply_text_style` — called by
    /// `NativeControl::sync_text_style` (`native_ui/control.rs`).
    pub(crate) fn apply_text_style(&self, style: &CascadedTextStyle) {
        self.0.apply_text_style(style);
    }

    /// Forwards to the wrapped handle's own `AppKitHandle::supports_text_style`.
    pub(crate) fn supports_text_style(&self) -> bool {
        self.0.supports_text_style()
    }

    /// Returns the standard-token prefix for the concrete wrapped control.
    pub(crate) fn theme_prefix(&self) -> &'static str {
        self.0.theme_prefix()
    }

    /// Applies an explicit background or restores the native toolkit default.
    pub(crate) fn apply_background(&self, background: Option<&Brush>) {
        self.0.apply_background(background);
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
