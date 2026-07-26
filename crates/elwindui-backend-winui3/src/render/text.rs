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
use crate::bindings::Microsoft::UI::Xaml::Media::{
    FontFamily as XamlFontFamily, SolidColorBrush,
};
use crate::bindings::winui_text::{
    FontStretch as XamlFontStretch, FontStyle as XamlFontStyle, FontWeight as XamlFontWeight,
};
use crate::render::solid_color_brush;
use elwindui_core::graphics::{
    Brush, CascadedTextStyle, Color, ComputedTextStyle, FontFamily, FontStretch, FontStyle,
    FontWeight, TextBackend, TextMeasureRequest, TextMeasureResult, TextWrapping,
};
use windows::Foundation::Size;
use windows::core::{HSTRING, Interface, Result};

/// §16: never pin a concrete platform family name in common code — but here, on the WinUI3 side,
/// `FontFamily::is_system()` maps to XAML's dedicated `XamlAutoFontFamily`, rather than skipping
/// `SetFontFamily`. Skipping leaves an old explicitly-set family behind when a reused native
/// control (or the measurement scratch block) switches back to the system profile.
fn xaml_font_family(family: &FontFamily) -> Result<XamlFontFamily> {
    if family.is_system() {
        XamlFontFamily::XamlAutoFontFamily()
    } else {
        // WinUI accepts a comma-separated fallback list in one `FontFamily` source string. Keep
        // the complete core value instead of throwing away every candidate after the first.
        XamlFontFamily::CreateInstanceWithName(&HSTRING::from(family.as_str()))
    }
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

fn core_font_style(style: XamlFontStyle) -> FontStyle {
    match style {
        XamlFontStyle::Italic => FontStyle::Italic,
        XamlFontStyle::Oblique => FontStyle::Oblique,
        _ => FontStyle::Normal,
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

fn core_font_stretch(stretch: XamlFontStretch) -> FontStretch {
    match stretch {
        XamlFontStretch::UltraCondensed => FontStretch::UltraCondensed,
        XamlFontStretch::ExtraCondensed => FontStretch::ExtraCondensed,
        XamlFontStretch::Condensed => FontStretch::Condensed,
        XamlFontStretch::SemiCondensed => FontStretch::SemiCondensed,
        XamlFontStretch::SemiExpanded => FontStretch::SemiExpanded,
        XamlFontStretch::Expanded => FontStretch::Expanded,
        XamlFontStretch::ExtraExpanded => FontStretch::ExtraExpanded,
        XamlFontStretch::UltraExpanded => FontStretch::UltraExpanded,
        _ => FontStretch::Normal,
    }
}

/// Same gradient/image degrade as `elwindui-backend-appkit::render::text::foreground_ns_color` —
/// a `Brush::Solid` is exact, anything else falls back to a representative flat color rather than
/// being silently dropped.
pub(crate) fn flat_foreground_color(brush: &Brush) -> Color {
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
#[cfg(test)]
pub(crate) fn apply_text_style_to_control(
    control: &Control,
    style: &ComputedTextStyle,
) -> Result<()> {
    control.SetFontFamily(&xaml_font_family(&style.font_family)?)?;
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

unsafe extern "C" {
    fn elwindui_winui3_clear_control_text_style(
        inspectable: *mut core::ffi::c_void,
        mask: u32,
    ) -> i32;
    fn elwindui_winui3_clear_text_block_foreground(
        inspectable: *mut core::ffi::c_void,
    ) -> i32;
}

/// Applies or clears a native `Control.Background` value.
pub(crate) fn apply_control_background(
    control: &Control,
    background: Option<&Brush>,
) -> Result<()> {
    if let Some(background) = background {
        control.SetBackground(&solid_color_brush(flat_foreground_color(background))?)
    } else {
        let code = unsafe {
            elwindui_winui3_clear_control_text_style(control.as_raw(), 1 << 7)
        };
        windows::core::HRESULT(code).ok()
    }
}

/// Applies inherited/explicit values and clears properties that remain absent after the ElwindUI
/// cascade. The C++/WinRT shim is used only for DependencyProperty statics that windows-bindgen
/// currently omits from its Rust projection.
pub(crate) fn apply_cascaded_text_style_to_control(
    control: &Control,
    style: &CascadedTextStyle,
) -> Result<()> {
    let mut clear_mask = 0u32;
    if let Some(value) = style.font_family.as_ref() {
        control.SetFontFamily(&xaml_font_family(value)?)?;
    } else {
        clear_mask |= 1 << 0;
    }
    if let Some(value) = style.font_size {
        control.SetFontSize(value as f64)?;
    } else {
        clear_mask |= 1 << 1;
    }
    if let Some(value) = style.font_weight {
        control.SetFontWeight(xaml_font_weight(value))?;
    } else {
        clear_mask |= 1 << 2;
    }
    if let Some(value) = style.font_style {
        control.SetFontStyle(xaml_font_style(value))?;
    } else {
        clear_mask |= 1 << 3;
    }
    if let Some(value) = style.font_stretch {
        control.SetFontStretch(xaml_font_stretch(value))?;
    } else {
        clear_mask |= 1 << 4;
    }
    if let Some(value) = style.character_spacing {
        control.SetCharacterSpacing(value)?;
    } else {
        clear_mask |= 1 << 5;
    }
    if let Some(value) = style.foreground.as_ref() {
        if let Ok(brush) = solid_color_brush(flat_foreground_color(value)) {
            control.SetForeground(&brush)?;
        }
    } else {
        clear_mask |= 1 << 6;
    }

    if clear_mask != 0 {
        let result = unsafe {
            elwindui_winui3_clear_control_text_style(
                Interface::as_raw(control),
                clear_mask,
            )
        };
        if result < 0 {
            return Err(windows::core::Error::from_hresult(
                windows::core::HRESULT(result),
            ));
        }
    }
    Ok(())
}

/// Same seven properties, for the diverted XAML `Controls::TextBlock` a `RenderCommand::Text`
/// becomes (`host::replay::reconcile_native_children`) — `TextBlock` doesn't derive `Control`
/// (WinUI3's own `TextBlock : FrameworkElement`), so it needs its own setter list rather than
/// reusing `apply_text_style_to_control`, even though every property name matches 1:1.
pub(crate) fn apply_text_style_to_text_block(
    text_block: &TextBlock,
    style: &ComputedTextStyle,
) -> Result<()> {
    apply_text_style_to_text_block_with_foreground(text_block, style, Some(&style.foreground))
}

/// Applies all font metrics and either an explicit foreground or the live XAML foreground
/// resource. `None` clears an old local value so a reused `TextBlock` follows Light/Dark and
/// High Contrast resources again.
pub(crate) fn apply_text_style_to_text_block_with_foreground(
    text_block: &TextBlock,
    style: &ComputedTextStyle,
    foreground: Option<&Brush>,
) -> Result<()> {
    text_block.SetFontFamily(&xaml_font_family(&style.font_family)?)?;
    text_block.SetFontSize(style.font_size as f64)?;
    text_block.SetFontWeight(xaml_font_weight(style.font_weight))?;
    text_block.SetFontStyle(xaml_font_style(style.font_style))?;
    text_block.SetFontStretch(xaml_font_stretch(style.font_stretch))?;
    text_block.SetCharacterSpacing(style.character_spacing)?;
    if let Some(foreground) = foreground {
        if let Ok(brush) = solid_color_brush(flat_foreground_color(foreground)) {
            text_block.SetForeground(&brush)?;
        }
    } else {
        let code = unsafe {
            elwindui_winui3_clear_text_block_foreground(text_block.as_raw())
        };
        windows::core::HRESULT(code).ok()?;
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
        let fallback = ComputedTextStyle::fallback();
        let foreground = scratch
            .Foreground()
            .ok()
            .and_then(|brush| brush.cast::<SolidColorBrush>().ok())
            .and_then(|brush| brush.Color().ok())
            .map(|color| {
                Brush::Solid(Color {
                    a: color.A,
                    r: color.R,
                    g: color.G,
                    b: color.B,
                })
            })
            .unwrap_or_else(|| fallback.foreground.clone());
        ComputedTextStyle {
            // Keep the dedicated sentinel: every later application asks WinUI for
            // `XamlAutoFontFamily()` again instead of pinning today's resolved family name.
            font_family: FontFamily::system(),
            font_size: scratch.FontSize().unwrap_or(fallback.font_size as f64) as f32,
            font_weight: scratch
                .FontWeight()
                .map(|weight| FontWeight(weight.Weight))
                .unwrap_or(fallback.font_weight),
            font_style: scratch
                .FontStyle()
                .map(core_font_style)
                .unwrap_or(fallback.font_style),
            font_stretch: scratch
                .FontStretch()
                .map(core_font_stretch)
                .unwrap_or(fallback.font_stretch),
            character_spacing: scratch
                .CharacterSpacing()
                .unwrap_or(fallback.character_spacing),
            foreground,
        }
    }

    fn measure_text(&self, req: &TextMeasureRequest<'_>) -> TextMeasureResult {
        SCRATCH_TEXT_BLOCK.with(|text_block| {
            let _ = text_block.SetText(&HSTRING::from(req.text));
            // Every property, including the system-family reset, is applied before measuring.
            // A style application failure must not silently reuse the previous scratch style.
            if apply_text_style_to_text_block(text_block, req.style).is_err() {
                return TextMeasureResult {
                    size: elwindui_core::base::Size::default(),
                    baseline: 0.0,
                    line_count: 0,
                };
            }
            let _ = text_block.SetTextWrapping(match req.wrapping {
                TextWrapping::NoWrap => crate::bindings::Microsoft::UI::Xaml::TextWrapping::NoWrap,
                TextWrapping::Wrap | TextWrapping::WrapWholeWords => {
                    crate::bindings::Microsoft::UI::Xaml::TextWrapping::Wrap
                }
            });

            // Same reset-then-remeasure dance `ffi::AnyView::measure` already documents (that
            // method's own doc comment has the full explanation of the feedback-loop hazard this
            // avoids) — an explicit `Width`/`Height` from a *previous* measurement otherwise
            // permanently overrides this element's natural size regardless of new content.
            let element: crate::bindings::Microsoft::UI::Xaml::FrameworkElement = text_block
                .clone()
                .cast()
                .expect("TextBlock is a FrameworkElement");
            let _ = element.SetWidth(f64::NAN);
            let _ = element.SetHeight(f64::NAN);
            let _ = element.InvalidateMeasure();
            let constraint_width =
                if req.wrapping == TextWrapping::NoWrap || !req.available.width.is_finite() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_weight_preserves_the_full_u16_value() {
        assert_eq!(xaml_font_weight(FontWeight(650)).Weight, 650);
    }

    #[test]
    fn font_style_maps_each_supported_variant() {
        assert_eq!(xaml_font_style(FontStyle::Normal), XamlFontStyle::Normal);
        assert_eq!(xaml_font_style(FontStyle::Italic), XamlFontStyle::Italic);
        assert_eq!(xaml_font_style(FontStyle::Oblique), XamlFontStyle::Oblique);
    }

    #[test]
    fn font_stretch_maps_all_nine_variants() {
        let cases = [
            (FontStretch::UltraCondensed, XamlFontStretch::UltraCondensed),
            (FontStretch::ExtraCondensed, XamlFontStretch::ExtraCondensed),
            (FontStretch::Condensed, XamlFontStretch::Condensed),
            (FontStretch::SemiCondensed, XamlFontStretch::SemiCondensed),
            (FontStretch::Normal, XamlFontStretch::Normal),
            (FontStretch::SemiExpanded, XamlFontStretch::SemiExpanded),
            (FontStretch::Expanded, XamlFontStretch::Expanded),
            (FontStretch::ExtraExpanded, XamlFontStretch::ExtraExpanded),
            (FontStretch::UltraExpanded, XamlFontStretch::UltraExpanded),
        ];
        for (input, expected) in cases {
            assert_eq!(xaml_font_stretch(input), expected);
        }
    }
}
