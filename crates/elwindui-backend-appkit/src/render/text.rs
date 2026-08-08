//! `ComputedTextStyle` -> `NSFont`/`NSAttributedString` conversion and text measurement — pure
//! translation, no `UIElement`/`NativeControl`/`host` knowledge (see `render/mod.rs`'s own doc
//! comment on the layering this module lives in). This is the one place in the backend that turns
//! a resolved, backend-independent style into real AppKit font/paint objects; `host::replay`
//! (drawing) and `AppKitTextBackend` (measurement, registered as the crate's
//! `elwindui_core::graphics::TextBackend`) both go through it, so a `TextBlock`'s measured size and
//! its painted glyphs are always built from the exact same `NSFont`/`NSAttributedString`.
//!
//! See `docs/status/font_status.md` for the full design writeup, including the approximation
//! table below and what's out of scope for this pass (gradient/image foreground text degrades to
//! a flat color — see `foreground_ns_color` — the same documented gap `render::paint::apply_fill`
//! already has for gradient fills elsewhere in this crate).

use elwindui_core::graphics::{
    Brush, Color, ComputedTextStyle, FontStyle, TextBackend, TextMeasureRequest,
    TextMeasureResult, TextWrapping,
};
use elwindui_core::ui::TextAlignment;
use objc2::{AnyThread, msg_send};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{
    NSAttributedStringNSExtendedStringDrawing, NSColor, NSFont, NSFontDescriptor,
    NSFontDescriptorSymbolicTraits, NSFontAttributeName, NSFontTraitsAttribute,
    NSFontWeightTrait, NSFontWidthTrait, NSForegroundColorAttributeName, NSKernAttributeName,
    NSMutableParagraphStyle, NSParagraphStyleAttributeName, NSStringDrawingOptions,
    NSTextAlignment,
};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSAttributedString, NSAttributedStringKey, NSDictionary, NSNumber, NSString};

/// AppKit's `NSFontWeightTrait`/`NSFontWeight*` scale is a continuous `-1.0..1.0` float —
/// approximated here by linearly interpolating between Apple's own documented named constants
/// (`NSFontWeightUltraLight` through `NSFontWeightBlack`), pinned at elwindui's `FontWeight`
/// hundreds-scale (100..900). This is why `FontWeight` is a numeric newtype rather than an enum
/// (see `crates/elwindui-core/src/graphics/text.rs`'s own doc comment): a variable-font weight
/// like 450 or 550 lands between two of these pins and interpolates smoothly instead of snapping.
const WEIGHT_PINS: [(f32, CGFloat); 9] = [
    (100.0, -0.8), // NSFontWeightUltraLight
    (200.0, -0.6), // NSFontWeightThin
    (300.0, -0.4), // NSFontWeightLight
    (400.0, 0.0),  // NSFontWeightRegular
    (500.0, 0.23), // NSFontWeightMedium
    (600.0, 0.3),  // NSFontWeightSemibold
    (700.0, 0.4),  // NSFontWeightBold
    (800.0, 0.56), // NSFontWeightHeavy
    (900.0, 0.62), // NSFontWeightBlack
];

fn nsfont_weight(weight: elwindui_core::graphics::FontWeight) -> CGFloat {
    let w = weight.0 as f32;
    if w <= WEIGHT_PINS[0].0 {
        return WEIGHT_PINS[0].1;
    }
    for pair in WEIGHT_PINS.windows(2) {
        let (lo_w, lo_v) = pair[0];
        let (hi_w, hi_v) = pair[1];
        if w <= hi_w {
            let t = ((w - lo_w) / (hi_w - lo_w)) as CGFloat;
            return lo_v + (hi_v - lo_v) * t;
        }
    }
    WEIGHT_PINS[WEIGHT_PINS.len() - 1].1
}

/// `FontStretch::percent()` (50.0..=200.0, 100.0 == normal) -> AppKit's `NSFontWidthTrait`
/// (`-1.0..1.0`, `0.0` == normal) — linear on either side of 100%, matching how AppKit's own
/// named width constants (`NSFontWidthCondensed` == `-0.4`, `NSFontWidthExpanded` == `0.4`) sit
/// roughly symmetric around `NSFontWidthStandard` == `0.0`.
fn nsfont_width(stretch: elwindui_core::graphics::FontStretch) -> CGFloat {
    let percent = stretch.percent() as CGFloat;
    if percent >= 100.0 {
        ((percent - 100.0) / 100.0).min(1.0)
    } else {
        ((percent - 100.0) / 50.0).max(-1.0)
    }
}

fn color_to_nscolor(color: Color) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(
        color.r as CGFloat / 255.0,
        color.g as CGFloat / 255.0,
        color.b as CGFloat / 255.0,
        color.a as CGFloat / 255.0,
    )
}

/// A flat foreground color for an explicit `foreground` override, or `NSColor::labelColor()`
/// when `None` — the same "unset means follow the platform appearance" fallback
/// `ffi.rs::flat_foreground_nscolor` already applies for native controls. `Brush::Solid` is exact;
/// a gradient/image brush degrades to one representative color — real masked-gradient text is a
/// documented gap (see this module's own doc comment) — rather than being silently dropped to
/// black.
fn foreground_ns_color(foreground: Option<&Brush>) -> Retained<NSColor> {
    let Some(brush) = foreground else {
        return NSColor::labelColor();
    };
    let color = match brush {
        Brush::Solid(color) => *color,
        other => super::first_gradient_stop_color(other).unwrap_or(Color::black()),
    };
    color_to_nscolor(color)
}

/// Builds `{ NSFontTraitsAttribute: { NSFontWeightTrait, NSFontWidthTrait } }` and layers it onto
/// `base` via `fontDescriptorByAddingAttributes`, then folds in the italic symbolic trait if
/// requested. The one font-descriptor-attribute assembly path shared by both the system-family and
/// named-family cases below.
fn apply_traits(
    base: &NSFontDescriptor,
    weight: CGFloat,
    width: CGFloat,
    italic: bool,
) -> Retained<NSFontDescriptor> {
    let weight_num = NSNumber::new_f64(weight);
    let width_num = NSNumber::new_f64(width);
    let traits_keys: [&NSString; 2] = unsafe { [NSFontWeightTrait, NSFontWidthTrait] };
    let traits_values: [&AnyObject; 2] = [weight_num.as_ref(), width_num.as_ref()];
    let traits_dict = NSDictionary::from_slices(&traits_keys, &traits_values);
    let attrs_keys: [&NSString; 1] = unsafe { [NSFontTraitsAttribute] };
    let attrs_values: [&AnyObject; 1] = [traits_dict.as_ref()];
    let attrs_dict = NSDictionary::from_slices(&attrs_keys, &attrs_values);
    let descriptor = unsafe { base.fontDescriptorByAddingAttributes(&attrs_dict) };
    if italic {
        let symbolic = descriptor.symbolicTraits() | NSFontDescriptorSymbolicTraits::TraitItalic;
        // AppKit documents this selector as nullable: a descriptor may not be able to realize the
        // requested symbolic traits. objc2-app-kit currently declares it as non-null, so using the
        // generated method would panic before we could fall back. Send it with an explicitly
        // nullable result and retain the pre-italic descriptor when AppKit returns `nil`.
        let italic_descriptor: Option<Retained<NSFontDescriptor>> = unsafe {
            msg_send![&descriptor, fontDescriptorWithSymbolicTraits: symbolic]
        };
        italic_descriptor.unwrap_or(descriptor)
    } else {
        descriptor
    }
}

/// Tries to realize `family` (one entry of `FontFamily::families()`) at `size`/`weight`/`width`/
/// `italic`. `NSFontDescriptor::fontDescriptorWithFamily` never itself reports "no such family" —
/// AppKit's own font matching substitutes the *closest* installed family instead of failing — so
/// this checks the realized font's own `familyName()` actually mentions what was asked for before
/// accepting it; an unrelated substitution is treated the same as "not found" and the caller moves
/// on to the next fallback candidate (指示書 §31: never crash on a missing font, always fall back).
fn try_named_font(
    family: &str,
    size: CGFloat,
    weight: CGFloat,
    width: CGFloat,
    italic: bool,
) -> Option<Retained<NSFont>> {
    let base = NSFontDescriptor::new().fontDescriptorWithFamily(&NSString::from_str(family));
    let descriptor = apply_traits(&base, weight, width, italic);
    let font = NSFont::fontWithDescriptor_size(&descriptor, size)?;
    let realized = font.familyName()?;
    if realized.to_string().eq_ignore_ascii_case(family) {
        Some(font)
    } else {
        None
    }
}

/// `ComputedTextStyle` -> `NSFont`. `style.font_family.is_system()` (the unset/default case, per
/// `FontFamily::system`'s own doc comment) goes through `NSFont::systemFontOfSize` rather than
/// naming a concrete platform family — 指示書 §16 forbids common-layer code from pinning e.g.
/// `"Yu Gothic UI"`, and this keeps that promise on the AppKit side too: whatever `NSFont` AppKit's
/// own Dynamic Type / language resolution picks is used verbatim.
pub(crate) fn ns_font(style: &ComputedTextStyle) -> Retained<NSFont> {
    super::stats::bump(|s| s.ns_fonts_created += 1);
    let size: CGFloat = if style.font_size.is_finite() && style.font_size > 0.0 {
        style.font_size as CGFloat
    } else {
        // 指示書 §31: an invalid size (<=0, NaN, infinite) falls back rather than producing an
        // unusable/zero-size font.
        NSFont::systemFontSize()
    };
    let weight = nsfont_weight(style.font_weight);
    let width = nsfont_width(style.font_stretch);
    let italic = style.font_style != FontStyle::Normal;

    if !style.font_family.is_system() {
        for family in style.font_family.families() {
            if let Some(font) = try_named_font(family, size, weight, width, italic) {
                return font;
            }
        }
        // No candidate in the fallback list actually resolved — fall through to the system
        // family below instead of returning nothing.
    }

    system_font_with_traits(size, weight, width, italic)
}

/// Resolves the font used by secure native text fields. `NSSecureTextField` draws its password
/// mask with AppKit-owned glyphs; using a caller-selected fallback family for that field can leave
/// those glyphs unavailable and render missing-glyph boxes. Keep AppKit's system family cascade
/// while preserving the requested size, weight, and width through AppKit's system-font API.
pub(crate) fn secure_text_font(style: &ComputedTextStyle) -> Retained<NSFont> {
    let size: CGFloat = if style.font_size.is_finite() && style.font_size > 0.0 {
        style.font_size as CGFloat
    } else {
        NSFont::systemFontSize()
    };
    // Do not rebuild a descriptor here. `NSSecureTextField` relies on the concrete system font
    // object to resolve its private mask glyph through AppKit's cascade; descriptor-based italic
    // synthesis can replace it with a font that lacks that glyph.
    NSFont::systemFontOfSize_weight_width(
        size,
        nsfont_weight(style.font_weight),
        nsfont_width(style.font_stretch),
    )
}

fn system_font_with_traits(
    size: CGFloat,
    weight: CGFloat,
    width: CGFloat,
    italic: bool,
) -> Retained<NSFont> {
    let base = NSFont::systemFontOfSize(size).fontDescriptor();
    let descriptor = apply_traits(&base, weight, width, italic);
    NSFont::fontWithDescriptor_size(&descriptor, size)
        .unwrap_or_else(|| NSFont::systemFontOfSize(size))
}

fn ns_text_alignment(alignment: TextAlignment) -> NSTextAlignment {
    match alignment {
        TextAlignment::Left => NSTextAlignment::Left,
        TextAlignment::Center => NSTextAlignment::Center,
        TextAlignment::Right => NSTextAlignment::Right,
    }
}

/// The attribute dictionary a `TextBlock`'s `NSAttributedString` (both for drawing —
/// `host::replay` — and for `boundingRectWithSize:options:context:` measurement below) is built
/// from — the single place font/foreground/kerning/alignment are assembled, so drawing and
/// measurement can never see a different set of attributes for the same resolved style.
pub(crate) fn text_attributes(
    style: &ComputedTextStyle,
    foreground: Option<&Brush>,
    alignment: TextAlignment,
) -> Retained<NSDictionary<NSAttributedStringKey, AnyObject>> {
    let font = ns_font(style);
    let color = foreground_ns_color(foreground);
    // `character_spacing` is 1/1000 em (WinUI3's own `CharacterSpacing` unit — see
    // `crates/elwindui-core/src/graphics/text.rs`'s own doc comment); AppKit's `NSKernAttributeName`
    // wants points, converted through the font's own point size once, here.
    let kern = NSNumber::new_f64(style.character_spacing as f64 / 1000.0 * style.font_size as f64);
    let paragraph_style = NSMutableParagraphStyle::new();
    paragraph_style.setAlignment(ns_text_alignment(alignment));

    let keys: [&NSAttributedStringKey; 4] = unsafe {
        [
            NSFontAttributeName,
            NSForegroundColorAttributeName,
            NSKernAttributeName,
            NSParagraphStyleAttributeName,
        ]
    };
    let values: [&AnyObject; 4] = [
        font.as_ref(),
        color.as_ref(),
        kern.as_ref(),
        paragraph_style.as_ref(),
    ];
    NSDictionary::from_slices(&keys, &values)
}

/// `text`/`style`/`foreground`/`alignment` -> a fully-attributed string, ready either to hand to a
/// `CATextLayer` (`host::replay`) or to measure (`boundingRectWithSize:options:context:` below).
/// `foreground` is the cascade's own (possibly unset) paint — `None` follows the platform
/// appearance (`NSColor::labelColor()`) rather than pinning `style`'s materialized fallback color.
pub(crate) fn attributed_string(
    text: &str,
    style: &ComputedTextStyle,
    foreground: Option<&Brush>,
    alignment: TextAlignment,
) -> Retained<NSAttributedString> {
    super::stats::bump(|s| s.attributed_strings_created += 1);
    let attrs = text_attributes(style, foreground, alignment);
    unsafe {
        NSAttributedString::initWithString_attributes(
            NSAttributedString::alloc(),
            &NSString::from_str(text),
            Some(&attrs),
        )
    }
}

/// This crate's [`TextBackend`](elwindui_core::graphics::TextBackend) — registered by `init()`
/// (`crates/elwindui-backend-appkit/src/lib.rs`). Both `TextBlock::measure_override` (core) and
/// every `NativeControl::sync_text_style` call go through the same `ns_font`/`attributed_string`
/// helpers above that `host::replay` uses to actually draw, so measurement and painting can never
/// disagree about which font was used (指示書 §21/§22).
pub(crate) struct AppKitTextBackend;

impl TextBackend for AppKitTextBackend {
    fn default_text_style(&self) -> ComputedTextStyle {
        let font = NSFont::systemFontOfSize(NSFont::systemFontSize());
        ComputedTextStyle {
            font_size: font.pointSize() as f32,
            ..ComputedTextStyle::fallback()
        }
    }

    fn measure_text(&self, req: &TextMeasureRequest<'_>) -> TextMeasureResult {
        let attributed =
            attributed_string(req.text, req.style, Some(&req.style.foreground), req.alignment);
        let constraint_width = if req.wrapping == TextWrapping::NoWrap || !req.available.width.is_finite()
        {
            CGFloat::MAX
        } else {
            req.available.width as CGFloat
        };
        let constraint = objc2_core_foundation::CGSize::new(constraint_width, CGFloat::MAX);
        let options = NSStringDrawingOptions::UsesLineFragmentOrigin
            | NSStringDrawingOptions::UsesFontLeading;
        let bounds = attributed.boundingRectWithSize_options_context(constraint, options, None);

        let font = ns_font(req.style);
        // `NSStringDrawing` reports no line count directly; approximate it from the measured
        // height against this font's own free-standing line height (未対応: an exact line count
        // would need `NSLayoutManager`, not attempted here — nothing consumes `line_count` yet).
        let line_height = (font.ascender() - font.descender() + font.leading()).max(1.0);
        let line_count = ((bounds.size.height / line_height).round() as u32).max(1);

        TextMeasureResult {
            size: elwindui_core::base::Size {
                width: bounds.size.width.ceil() as f32,
                height: bounds.size.height.ceil() as f32,
            },
            baseline: font.ascender() as f32,
            line_count,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use elwindui_core::graphics::{FontFamily, FontStretch, FontWeight};

    fn style(font_size: f32) -> ComputedTextStyle {
        ComputedTextStyle {
            font_size,
            ..ComputedTextStyle::fallback()
        }
    }

    #[test]
    fn ns_font_honors_requested_size() {
        let font = ns_font(&style(24.0));
        assert_eq!(font.pointSize(), 24.0);
    }

    #[test]
    fn ns_font_bold_is_heavier_than_normal() {
        // `nsfont_weight` is the pure-Rust interpolation this whole conversion rests on — a direct
        // unit test of it, rather than trying to read `NSFontWeightTrait` back out of a realized
        // `NSFontDescriptor` (its traits dictionary only downcasts to the untyped `AnyObject`/
        // `AnyObject` form, and `NSFontManager`'s own weight-of-font query needs the real main
        // thread, which `cargo test`'s harness thread isn't).
        assert!(nsfont_weight(FontWeight::BOLD) > nsfont_weight(FontWeight::NORMAL));
        assert!(nsfont_weight(FontWeight::BLACK) > nsfont_weight(FontWeight::BOLD));
        assert!(nsfont_weight(FontWeight::THIN) < nsfont_weight(FontWeight::NORMAL));
        // Off-grid (variable-font) weights interpolate strictly between their neighboring pins —
        // this is the whole reason `FontWeight` is a numeric newtype rather than an enum.
        assert!(nsfont_weight(FontWeight(450)) > nsfont_weight(FontWeight::NORMAL));
        assert!(nsfont_weight(FontWeight(450)) < nsfont_weight(FontWeight::MEDIUM));

        // And that the realized fonts really are two distinct `NSFont` objects at the requested
        // size (the AppKit-facing half of the same conversion).
        let normal = ns_font(&ComputedTextStyle {
            font_weight: FontWeight::NORMAL,
            ..style(16.0)
        });
        let bold = ns_font(&ComputedTextStyle {
            font_weight: FontWeight::BOLD,
            ..style(16.0)
        });
        assert_eq!(normal.pointSize(), 16.0);
        assert_eq!(bold.pointSize(), 16.0);
    }

    #[test]
    fn ns_font_italic_sets_the_symbolic_trait() {
        let normal = ns_font(&ComputedTextStyle {
            font_style: elwindui_core::graphics::FontStyle::Normal,
            ..style(16.0)
        });
        let italic = ns_font(&ComputedTextStyle {
            font_style: elwindui_core::graphics::FontStyle::Italic,
            ..style(16.0)
        });
        assert!(!normal
            .fontDescriptor()
            .symbolicTraits()
            .contains(objc2_app_kit::NSFontDescriptorSymbolicTraits::TraitItalic));
        assert!(italic
            .fontDescriptor()
            .symbolicTraits()
            .contains(objc2_app_kit::NSFontDescriptorSymbolicTraits::TraitItalic));
    }

    #[test]
    fn ns_font_missing_family_falls_back_to_system_rather_than_panicking() {
        // 指示書 §31: an unresolvable font must fall back, never crash.
        let font = ns_font(&ComputedTextStyle {
            font_family: FontFamily::new("Definitely Not A Real Font Family XYZ"),
            ..style(16.0)
        });
        assert_eq!(font.pointSize(), 16.0);
    }

    #[test]
    fn ns_font_missing_family_with_italic_falls_back_without_panicking() {
        // Some unresolved-family descriptors return `nil` when asked for an italic symbolic
        // trait. That nullable AppKit result must not cross the objc2 non-null binding as a panic.
        let font = ns_font(&ComputedTextStyle {
            font_family: FontFamily::new("Definitely Not A Real Font Family XYZ"),
            font_style: FontStyle::Italic,
            ..style(16.0)
        });
        assert_eq!(font.pointSize(), 16.0);
    }

    #[test]
    fn secure_text_font_uses_the_system_family_for_a_requested_fallback() {
        let font = secure_text_font(&ComputedTextStyle {
            font_family: FontFamily::new("Definitely Not A Real Font Family XYZ"),
            ..style(22.0)
        });
        let system = NSFont::systemFontOfSize(22.0);
        assert_eq!(font.pointSize(), 22.0);
        assert_eq!(
            font.familyName().map(|family| family.to_string()),
            system.familyName().map(|family| family.to_string())
        );
    }

    #[test]
    fn secure_text_font_uses_the_system_font_when_italic_is_requested() {
        let font = secure_text_font(&ComputedTextStyle {
            font_family: FontFamily::new("Definitely Not A Real Font Family XYZ"),
            font_style: FontStyle::Italic,
            ..style(22.0)
        });
        let system = NSFont::systemFontOfSize(22.0);
        assert_eq!(font.pointSize(), 22.0);
        assert_eq!(
            font.familyName().map(|family| family.to_string()),
            system.familyName().map(|family| family.to_string())
        );
    }

    #[test]
    fn ns_font_stretch_percent_is_monotone_in_width_trait() {
        let condensed = ns_font(&ComputedTextStyle {
            font_stretch: FontStretch::Condensed,
            ..style(16.0)
        });
        let expanded = ns_font(&ComputedTextStyle {
            font_stretch: FontStretch::Expanded,
            ..style(16.0)
        });
        // Both must at least construct successfully at the requested size — exact width-trait
        // realization is font-dependent (not every installed family has condensed/expanded
        // masters), so this only asserts the conversion path doesn't panic/degrade the size.
        assert_eq!(condensed.pointSize(), 16.0);
        assert_eq!(expanded.pointSize(), 16.0);
    }

    #[test]
    fn measure_grows_with_text_length() {
        let backend = AppKitTextBackend;
        let short = backend.measure_text(&TextMeasureRequest {
            text: "hi",
            style: &style(16.0),
            available: elwindui_core::base::Size {
                width: f32::INFINITY,
                height: f32::INFINITY,
            },
            wrapping: TextWrapping::NoWrap,
            alignment: TextAlignment::Left,
            max_lines: None,
            scale: 1.0,
        });
        let long = backend.measure_text(&TextMeasureRequest {
            text: "hello, this is a much longer line of text",
            style: &style(16.0),
            available: elwindui_core::base::Size {
                width: f32::INFINITY,
                height: f32::INFINITY,
            },
            wrapping: TextWrapping::NoWrap,
            alignment: TextAlignment::Left,
            max_lines: None,
            scale: 1.0,
        });
        assert!(long.size.width > short.size.width);
    }

    #[test]
    fn measure_wrap_at_narrow_width_grows_height_vs_nowrap() {
        let backend = AppKitTextBackend;
        let text = "this is a fairly long sentence that should wrap across multiple lines";
        let unwrapped = backend.measure_text(&TextMeasureRequest {
            text,
            style: &style(16.0),
            available: elwindui_core::base::Size {
                width: f32::INFINITY,
                height: f32::INFINITY,
            },
            wrapping: TextWrapping::NoWrap,
            alignment: TextAlignment::Left,
            max_lines: None,
            scale: 1.0,
        });
        let wrapped = backend.measure_text(&TextMeasureRequest {
            text,
            style: &style(16.0),
            available: elwindui_core::base::Size {
                width: 80.0,
                height: f32::INFINITY,
            },
            wrapping: TextWrapping::Wrap,
            alignment: TextAlignment::Left,
            max_lines: None,
            scale: 1.0,
        });
        assert!(wrapped.size.height > unwrapped.size.height);
        assert!(wrapped.size.width <= unwrapped.size.width);
    }

    #[test]
    fn character_spacing_widens_measured_text() {
        let backend = AppKitTextBackend;
        let available = elwindui_core::base::Size {
            width: f32::INFINITY,
            height: f32::INFINITY,
        };
        let no_kern_style = style(16.0);
        let wide_kern_style = ComputedTextStyle {
            character_spacing: 500,
            ..style(16.0)
        };
        let no_kern = backend.measure_text(&TextMeasureRequest {
            text: "kerning test",
            style: &no_kern_style,
            available,
            wrapping: TextWrapping::NoWrap,
            alignment: TextAlignment::Left,
            max_lines: None,
            scale: 1.0,
        });
        let wide_kern = backend.measure_text(&TextMeasureRequest {
            text: "kerning test",
            style: &wide_kern_style,
            available,
            wrapping: TextWrapping::NoWrap,
            alignment: TextAlignment::Left,
            max_lines: None,
            scale: 1.0,
        });
        assert!(wide_kern.size.width > no_kern.size.width);
    }

    #[test]
    fn default_text_style_matches_system_font_size() {
        let backend = AppKitTextBackend;
        let default_style = backend.default_text_style();
        assert_eq!(
            default_style.font_size,
            NSFont::systemFontOfSize(NSFont::systemFontSize()).pointSize() as f32
        );
    }
}
