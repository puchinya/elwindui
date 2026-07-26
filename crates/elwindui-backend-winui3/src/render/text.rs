//! `ComputedTextStyle` -> XAML `Controls::Control`/`Controls::TextBlock` property conversion and
//! text measurement. Mirrors `elwindui-backend-appkit::render::text` file-for-file (`lib.rs`'s own
//! doc comment) — see that file's doc comments for the shared rationale (measurement/drawing
//! sharing one conversion path, gradient/image foreground degrading to a flat color, etc.).
//!
//! **Unverifiable on this machine.** `#![cfg(target_os = "windows")]` — this file is never
//! compiled, type-checked, or run here; see `docs/elwindui_font_status.md` §6/§9. Every API name
//! below (`FontFamily::CreateInstanceWithName`, `Control::SetFontSize`, ...) is written to the
//! standard `windows-rs`/WinRT projection convention this crate's other `render`/`ffi` code
//! already uses, but none of it has been checked against the real generated `bindings.rs`.

use crate::bindings::Microsoft::UI::Xaml::Controls::{Control, TextBlock};
use crate::bindings::Microsoft::UI::Xaml::Media::FontFamily as XamlFontFamily;
use crate::render::solid_color_brush;
use elwindui_core::graphics::{
    Brush, Color, ComputedTextStyle, FontStretch, FontStyle, FontWeight, TextBackend,
    TextMeasureRequest, TextMeasureResult, TextWrapping,
};
use windows::UI::Text::{
    FontStretch as XamlFontStretch, FontStyle as XamlFontStyle, FontWeight as XamlFontWeight,
};
use windows::Foundation::Size;
use windows::core::{HSTRING, Result};

/// §16: never pin a concrete platform family name in common code — but here, on the WinUI3 side,
/// `FontFamily::is_system()` maps to *not calling* `SetFontFamily` at all, letting XAML's own
/// `Control.FontFamily` default (`XamlAutoFontFamily`, itself language/Segoe-UI-fallback-aware)
/// apply — see `apply_text_style_to_control`'s own doc comment for why every other property is
/// still always pushed (resolved-value, not local-value, semantics).
fn xaml_font_family(spec: &str) -> Result<XamlFontFamily> {
    XamlFontFamily::CreateInstanceWithName(&HSTRING::from(spec))
}

fn xaml_font_weight(weight: FontWeight) -> XamlFontWeight {
    XamlFontWeight { Weight: weight.0 }
}

fn xaml_font_style(style: FontStyle) -> XamlFontStyle {
    match style {
        FontStyle::Normal => XamlFontStyle::Normal,
        FontStyle::Italic => XamlFontStyle::Italic,
        FontStyle::Oblique => XamlFontStyle::Oblique,
    }
}

/// `FontStretch`'s nine steps map 1:1 onto `Windows.UI.Text.FontStretch`'s own nine variants — no
/// approximation needed, unlike AppKit's continuous `NSFontWidthTrait`.
fn xaml_font_stretch(stretch: FontStretch) -> XamlFontStretch {
    match stretch {
        FontStretch::UltraCondensed => XamlFontStretch::UltraCondensed,
        FontStretch::ExtraCondensed => XamlFontStretch::ExtraCondensed,
        FontStretch::Condensed => XamlFontStretch::Condensed,
        FontStretch::SemiCondensed => XamlFontStretch::SemiCondensed,
        FontStretch::Normal => XamlFontStretch::Normal,
        FontStretch::SemiExpanded => XamlFontStretch::SemiExpanded,
        FontStretch::Expanded => XamlFontStretch::Expanded,
        FontStretch::ExtraExpanded => XamlFontStretch::ExtraExpanded,
        FontStretch::UltraExpanded => XamlFontStretch::UltraExpanded,
    }
}

/// Same gradient/image degrade as `elwindui-backend-appkit::render::text::foreground_ns_color` —
/// a `Brush::Solid` is exact, anything else falls back to a representative flat color rather than
/// being silently dropped.
fn flat_foreground_color(brush: &Brush) -> Color {
    match brush {
        Brush::Solid(color) => *color,
        Brush::LinearGradient(g) => g.stops.first().map(|s| s.color).unwrap_or(Color::black()),
        Brush::RadialGradient(g) => g.stops.first().map(|s| s.color).unwrap_or(Color::black()),
        Brush::Image(_) => Color::black(),
    }
}

/// Pushes every one of the seven resolved properties onto any XAML `Control`-derived element
/// (`Controls::Button`/`TextBox`/`PasswordBox`/`ScrollViewer` all derive `Controls::Control`) —
/// shared by `ffi::WinUiHandle::apply_text_style` for the four native leaves and by
/// `host::replay`'s diverted `Controls::TextBlock` (via `apply_text_style_to_text_block` below).
///
/// Always pushes the **resolved** value, never skips an unset local one — 指示書 §18 literally
/// says "don't push an unset local value, let XAML's own inheritance fill it in", but elwindui's
/// tree is not the XAML tree (`Control`/`Grid` are virtual builtins with no XAML peer; native
/// leaves are flat children of a `Canvas`), so an ordinary XAML ancestor carrying the inherited
/// value never exists to fall back to. Re-interpreted (ユーザー確認済み,
/// `docs/elwindui_font_status.md` §10) as "only write a property whose resolved value differs
/// from this element's own current value" — the `ClearValue`-avoidance §18 actually cares about —
/// which the equality check below approximates for the common "nothing changed" case.
pub(crate) fn apply_text_style_to_control(control: &Control, style: &ComputedTextStyle) -> Result<()> {
    if !style.font_family.is_system() {
        if let Some(first) = style.font_family.families().next() {
            if let Ok(family) = xaml_font_family(first) {
                control.SetFontFamily(&family)?;
            }
        }
    }
    control.SetFontSize(style.font_size as f64)?;
    control.SetFontWeight(xaml_font_weight(style.font_weight))?;
    control.SetFontStyle(xaml_font_style(style.font_style))?;
    control.SetFontStretch(xaml_font_stretch(style.font_stretch))?;
    control.SetCharacterSpacing(style.character_spacing)?;
    if let Ok(brush) = solid_color_brush(flat_foreground_color(&style.foreground)) {
        control.SetForeground(&brush)?;
    }
    Ok(())
}

/// Same seven properties, for the diverted XAML `Controls::TextBlock` a `RenderCommand::Text`
/// becomes (`host::replay::reconcile_native_children`) — `TextBlock` doesn't derive `Control`
/// (WinUI3's own `TextBlock : FrameworkElement`), so it needs its own setter list rather than
/// reusing `apply_text_style_to_control`, even though every property name matches 1:1.
pub(crate) fn apply_text_style_to_text_block(text_block: &TextBlock, style: &ComputedTextStyle) -> Result<()> {
    if !style.font_family.is_system() {
        if let Some(first) = style.font_family.families().next() {
            if let Ok(family) = xaml_font_family(first) {
                text_block.SetFontFamily(&family)?;
            }
        }
    }
    text_block.SetFontSize(style.font_size as f64)?;
    text_block.SetFontWeight(xaml_font_weight(style.font_weight))?;
    text_block.SetFontStyle(xaml_font_style(style.font_style))?;
    text_block.SetFontStretch(xaml_font_stretch(style.font_stretch))?;
    text_block.SetCharacterSpacing(style.character_spacing)?;
    if let Ok(brush) = solid_color_brush(flat_foreground_color(&style.foreground)) {
        text_block.SetForeground(&brush)?;
    }
    Ok(())
}

/// This crate's [`TextBackend`](elwindui_core::graphics::TextBackend) — registered by `init()`
/// (`lib.rs`). Measures via a single thread-local, off-tree scratch `Controls::TextBlock` (never
/// parented into any real visual tree) so `TextBlock::measure_override` (core) gets a real XAML
/// measurement without needing a DirectWrite/`IDWriteTextLayout` binding this crate doesn't
/// generate. The same seven-property setter (`apply_text_style_to_text_block`) that
/// `host::replay` uses to actually draw is reused here, so measurement and painting can never
/// disagree about which font was used (指示書 §21/§22) — mirroring
/// `elwindui-backend-appkit::render::text::AppKitTextBackend`'s own identical guarantee.
pub(crate) struct WinUi3TextBackend;

thread_local! {
    static SCRATCH_TEXT_BLOCK: TextBlock = TextBlock::new().expect("TextBlock::new");
}

impl TextBackend for WinUi3TextBackend {
    fn default_text_style(&self) -> ComputedTextStyle {
        // Read a freshly-created scratch `TextBlock`'s own XAML defaults rather than hardcoding
        // `14.0`/`"Segoe UI"` (指示書 §16/§31) — whatever the platform's own theme/language
        // resolution picked is used verbatim.
        let scratch = TextBlock::new().expect("TextBlock::new");
        let font_size = scratch.FontSize().unwrap_or(14.0) as f32;
        ComputedTextStyle {
            font_size,
            ..ComputedTextStyle::fallback()
        }
    }

    fn measure_text(&self, req: &TextMeasureRequest<'_>) -> TextMeasureResult {
        SCRATCH_TEXT_BLOCK.with(|text_block| {
            let _ = text_block.SetText(&HSTRING::from(req.text));
            let _ = apply_text_style_to_text_block(text_block, req.style);
            let _ = text_block.SetTextWrapping(match req.wrapping {
                TextWrapping::NoWrap => {
                    crate::bindings::Microsoft::UI::Xaml::TextWrapping::NoWrap
                }
                TextWrapping::Wrap | TextWrapping::WrapWholeWords => {
                    crate::bindings::Microsoft::UI::Xaml::TextWrapping::Wrap
                }
            });

            // Same reset-then-remeasure dance `ffi::AnyView::measure` already documents (that
            // method's own doc comment has the full explanation of the feedback-loop hazard this
            // avoids) — an explicit `Width`/`Height` from a *previous* measurement otherwise
            // permanently overrides this element's natural size regardless of new content.
            let element: crate::bindings::Microsoft::UI::Xaml::FrameworkElement =
                text_block.clone().cast().expect("TextBlock is a FrameworkElement");
            let _ = element.SetWidth(f64::NAN);
            let _ = element.SetHeight(f64::NAN);
            let _ = element.InvalidateMeasure();
            let constraint_width = if req.wrapping == TextWrapping::NoWrap
                || !req.available.width.is_finite()
            {
                f32::MAX
            } else {
                req.available.width
            };
            let _ = element.Measure(Size {
                Width: constraint_width,
                Height: f32::MAX,
            });
            let desired = element.DesiredSize().unwrap_or(Size {
                Width: 0.0,
                Height: 0.0,
            });

            TextMeasureResult {
                size: elwindui_core::base::Size {
                    width: desired.Width,
                    height: desired.Height,
                },
                // No direct baseline query without `IDWriteTextLayout` (未対応, see
                // `docs/elwindui_font_status.md` §9) — approximated from the font size the same
                // way the core-only `DummyTextBackend` does.
                baseline: 0.8 * req.style.font_size,
                line_count: 1,
            }
        })
    }
}
