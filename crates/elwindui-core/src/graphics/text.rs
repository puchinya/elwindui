//! Backend-independent font/text-style model — WinUI3's `FontFamily`/`FontSize`/`FontWeight`/
//! `FontStyle`/`FontStretch`/`CharacterSpacing`/`Foreground` taken as individually-settable,
//! individually-inherited properties (指示書 §2), aggregated internally into one
//! [`TextStyleStorage`] per owning element the same way WinUI3 aggregates its own dependency
//! properties into a `TextFormatting` struct. See `docs/elwindui_font_status.md` for the full
//! design writeup (inheritance model, per-backend mapping tables, and what's deliberately
//! unimplemented).
//!
//! Resolution order (指示書 §6): local value -> nearest `TextStyleOwner` ancestor's *resolved*
//! value (walked via [`super::super::ui::UIElement::inheritance_parent`] with
//! `InheritanceParentKind::Visual`) -> backend default. Each of the seven properties is resolved
//! independently — never a single "whole style" hop (指示書 §7/§33 forbid replacing the entire
//! style wholesale).

use super::brush::Brush;
use crate::base::Size;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// WinUI3 `FontFamily` — a comma-separated fallback list, stored as one string. Never construct
/// this with a concrete platform name (`"Segoe UI"`, `"Yu Gothic UI"`, ...) in backend-independent
/// code — 指示書 §16 forbids common-layer code from pinning a platform's default family; use
/// [`FontFamily::system`] and let each backend resolve its own default.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FontFamily(Arc<str>);

/// Sentinel spelling for "let the backend pick its own default family" — never a real font name,
/// so no backend could ever coincidentally resolve a real family called this.
const SYSTEM_FAMILY: &str = "system-ui";

impl FontFamily {
    pub fn new(spec: impl Into<Arc<str>>) -> Self {
        Self(spec.into())
    }

    /// The backend-default UI font family. This is the value every `TextStyleStorage` field
    /// starts unset (`None`) against; `ComputedTextStyle::fallback` uses this, not a literal name.
    pub fn system() -> Self {
        Self(Arc::from(SYSTEM_FAMILY))
    }

    pub fn is_system(&self) -> bool {
        self.0.as_ref() == SYSTEM_FAMILY
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Splits the comma-separated fallback list, trimming whitespace around each entry.
    pub fn families(&self) -> impl Iterator<Item = &str> + '_ {
        self.0.split(',').map(str::trim).filter(|s| !s.is_empty())
    }
}

impl Default for FontFamily {
    fn default() -> Self {
        Self::system()
    }
}

/// WinUI3's `Windows.UI.Text.FontWeight` is itself a `{ Weight: u16 }` struct, and AppKit's
/// `NSFontWeightTrait` is a continuous `-1.0..1.0` float — a numeric newtype (rather than a
/// `Thin..Black` enum) is the only representation that round-trips both without loss, and still
/// represents the off-grid weights (450, 550, ...) variable fonts use. Confirmed with the user as
/// an intentional, narrow exception to CLAUDE.md's "enums are the only value-set mechanism" rule,
/// which targets DSL anonymous unions, not a numeric measurement like this one. DSL ergonomics are
/// unaffected either way: `font_weight: FontWeight::BOLD` is a plain path expression, exactly like
/// `horizontal_alignment: HorizontalAlignment::Center` today.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FontWeight(pub u16);

impl FontWeight {
    pub const THIN: Self = Self(100);
    pub const EXTRA_LIGHT: Self = Self(200);
    pub const LIGHT: Self = Self(300);
    pub const NORMAL: Self = Self(400);
    pub const MEDIUM: Self = Self(500);
    pub const SEMI_BOLD: Self = Self(600);
    pub const BOLD: Self = Self(700);
    pub const EXTRA_BOLD: Self = Self(800);
    pub const BLACK: Self = Self(900);
}

impl Default for FontWeight {
    fn default() -> Self {
        Self::NORMAL
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FontStyle {
    #[default]
    Normal,
    Italic,
    Oblique,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FontStretch {
    UltraCondensed,
    ExtraCondensed,
    Condensed,
    SemiCondensed,
    #[default]
    Normal,
    SemiExpanded,
    Expanded,
    ExtraExpanded,
    UltraExpanded,
}

impl FontStretch {
    /// CSS/DirectWrite percentage (50.0..=200.0), the common unit every backend mapping (AppKit's
    /// `NSFontWidthTrait`, WinUI3's `FontStretch`) converts through.
    pub fn percent(self) -> f32 {
        match self {
            Self::UltraCondensed => 50.0,
            Self::ExtraCondensed => 62.5,
            Self::Condensed => 75.0,
            Self::SemiCondensed => 87.5,
            Self::Normal => 100.0,
            Self::SemiExpanded => 112.5,
            Self::Expanded => 125.0,
            Self::ExtraExpanded => 150.0,
            Self::UltraExpanded => 200.0,
        }
    }
}

/// Identifies one of the seven text-style properties — used by
/// [`TextStyleOwner::on_text_style_property_changed`](crate::ui::TextStyleOwner) (change
/// notification, 指示書 §8/§23) and by `clear_text_style_property`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextStyleProperty {
    FontFamily,
    FontSize,
    FontWeight,
    FontStyle,
    FontStretch,
    CharacterSpacing,
    Foreground,
}

impl TextStyleProperty {
    pub const ALL: [Self; 7] = [
        Self::FontFamily,
        Self::FontSize,
        Self::FontWeight,
        Self::FontStyle,
        Self::FontStretch,
        Self::CharacterSpacing,
        Self::Foreground,
    ];

    /// Matches the DSL field name `#[text_style]` injects (`crates/elwindui-codegen/src/text_style.rs`).
    pub fn name(self) -> &'static str {
        match self {
            Self::FontFamily => "font_family",
            Self::FontSize => "font_size",
            Self::FontWeight => "font_weight",
            Self::FontStyle => "font_style",
            Self::FontStretch => "font_stretch",
            Self::CharacterSpacing => "character_spacing",
            Self::Foreground => "foreground",
        }
    }
}

/// Per-property local values. `None` means "unset — inherit" (指示書 §5/§6); this is deliberately
/// never used to distinguish "unset" from "explicitly set to the same value as the default" (指示書
/// §26 forbids judging locality from the value alone) since it's a real `Option`, not a
/// value comparison.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextStyleValues {
    pub font_family: Option<FontFamily>,
    pub font_size: Option<f32>,
    pub font_weight: Option<FontWeight>,
    pub font_style: Option<FontStyle>,
    pub font_stretch: Option<FontStretch>,
    pub character_spacing: Option<i32>,
    pub foreground: Option<Brush>,
}

/// The property-wise result of local values and Visual-parent inheritance, before backend
/// defaults are materialized. A remaining `None` is semantically significant: native controls
/// must clear that platform property instead of receiving a framework-chosen fixed value.
pub type CascadedTextStyle = TextStyleValues;

impl TextStyleValues {
    /// Fills only values that are still absent after Visual inheritance.
    pub fn materialize(&self, fallback: &ComputedTextStyle) -> ComputedTextStyle {
        ComputedTextStyle {
            font_family: self
                .font_family
                .clone()
                .unwrap_or_else(|| fallback.font_family.clone()),
            font_size: self.font_size.unwrap_or(fallback.font_size),
            font_weight: self.font_weight.unwrap_or(fallback.font_weight),
            font_style: self.font_style.unwrap_or(fallback.font_style),
            font_stretch: self.font_stretch.unwrap_or(fallback.font_stretch),
            character_spacing: self
                .character_spacing
                .unwrap_or(fallback.character_spacing),
            foreground: self
                .foreground
                .clone()
                .unwrap_or_else(|| fallback.foreground.clone()),
        }
    }
}

/// All seven properties resolved — no `Option`s. This is what measurement and drawing consume;
/// `TextBlock::render`/`measure_override` and every backend's `apply_text_style` take this, never
/// `TextStyleValues` (指示書 §7: "描画および計測では...解決済みTextStyleを使用すること").
#[derive(Debug, Clone, PartialEq)]
pub struct ComputedTextStyle {
    pub font_family: FontFamily,
    pub font_size: f32,
    pub font_weight: FontWeight,
    pub font_style: FontStyle,
    pub font_stretch: FontStretch,
    pub character_spacing: i32,
    pub foreground: Brush,
}

impl ComputedTextStyle {
    /// The absolute last-resort fallback, used only when no `TextBackend` is registered (plain
    /// `elwindui-core` unit tests) and as the starting point each real backend's own
    /// `TextBackend::default_text_style` overrides from. Deliberately reproduces the numbers
    /// `TextBlock::measure_override` used before this feature existed (`8.0`/char, `16.0` tall via
    /// `DummyTextBackend`), so no pre-existing size assertion in `ui.rs` changes.
    pub fn fallback() -> Self {
        Self {
            font_family: FontFamily::system(),
            font_size: 16.0,
            font_weight: FontWeight::NORMAL,
            font_style: FontStyle::Normal,
            font_stretch: FontStretch::Normal,
            character_spacing: 0,
            foreground: Brush::Solid(crate::graphics::Color::black()),
        }
    }
}

impl Default for ComputedTextStyle {
    fn default() -> Self {
        Self::fallback()
    }
}

/// The interior-mutable field type every text-style-capable class declares (`Control`,
/// `TextBlock`, each backend's `NativeControl` — 指示書 §5 recommends exactly this shape). Not a
/// `#[class]`-managed field: it's plain private state behind getters/setters, the same as
/// `UIElement`'s own `Cell`/`RefCell` fields.
#[derive(Debug, Default)]
pub struct TextStyleStorage {
    local: RefCell<TextStyleValues>,
}

impl TextStyleStorage {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn local(&self) -> TextStyleValues {
        self.local.borrow().clone()
    }

    pub fn is_set(&self, property: TextStyleProperty) -> bool {
        let local = self.local.borrow();
        match property {
            TextStyleProperty::FontFamily => local.font_family.is_some(),
            TextStyleProperty::FontSize => local.font_size.is_some(),
            TextStyleProperty::FontWeight => local.font_weight.is_some(),
            TextStyleProperty::FontStyle => local.font_style.is_some(),
            TextStyleProperty::FontStretch => local.font_stretch.is_some(),
            TextStyleProperty::CharacterSpacing => local.character_spacing.is_some(),
            TextStyleProperty::Foreground => local.foreground.is_some(),
        }
    }

    pub fn font_family(&self) -> Option<FontFamily> {
        self.local.borrow().font_family.clone()
    }
    /// Returns `true` iff the local value actually changed — callers use this to skip
    /// invalidation on a no-op write (指示書 §23 implies change notification is per actual change).
    pub fn set_font_family(&self, value: Option<FontFamily>) -> bool {
        let mut local = self.local.borrow_mut();
        if local.font_family == value {
            return false;
        }
        local.font_family = value;
        true
    }

    pub fn font_size(&self) -> Option<f32> {
        self.local.borrow().font_size
    }
    pub fn set_font_size(&self, value: Option<f32>) -> bool {
        let mut local = self.local.borrow_mut();
        if local.font_size == value {
            return false;
        }
        local.font_size = value;
        true
    }

    pub fn font_weight(&self) -> Option<FontWeight> {
        self.local.borrow().font_weight
    }
    pub fn set_font_weight(&self, value: Option<FontWeight>) -> bool {
        let mut local = self.local.borrow_mut();
        if local.font_weight == value {
            return false;
        }
        local.font_weight = value;
        true
    }

    pub fn font_style(&self) -> Option<FontStyle> {
        self.local.borrow().font_style
    }
    pub fn set_font_style(&self, value: Option<FontStyle>) -> bool {
        let mut local = self.local.borrow_mut();
        if local.font_style == value {
            return false;
        }
        local.font_style = value;
        true
    }

    pub fn font_stretch(&self) -> Option<FontStretch> {
        self.local.borrow().font_stretch
    }
    pub fn set_font_stretch(&self, value: Option<FontStretch>) -> bool {
        let mut local = self.local.borrow_mut();
        if local.font_stretch == value {
            return false;
        }
        local.font_stretch = value;
        true
    }

    pub fn character_spacing(&self) -> Option<i32> {
        self.local.borrow().character_spacing
    }
    pub fn set_character_spacing(&self, value: Option<i32>) -> bool {
        let mut local = self.local.borrow_mut();
        if local.character_spacing == value {
            return false;
        }
        local.character_spacing = value;
        true
    }

    pub fn foreground(&self) -> Option<Brush> {
        self.local.borrow().foreground.clone()
    }
    pub fn set_foreground(&self, value: Option<Brush>) -> bool {
        let mut local = self.local.borrow_mut();
        if local.foreground == value {
            return false;
        }
        local.foreground = value;
        true
    }

    /// Clears one property's local value, returning `true` iff it was actually set beforehand.
    pub fn clear(&self, property: TextStyleProperty) -> bool {
        match property {
            TextStyleProperty::FontFamily => self.set_font_family(None),
            TextStyleProperty::FontSize => self.set_font_size(None),
            TextStyleProperty::FontWeight => self.set_font_weight(None),
            TextStyleProperty::FontStyle => self.set_font_style(None),
            TextStyleProperty::FontStretch => self.set_font_stretch(None),
            TextStyleProperty::CharacterSpacing => self.set_character_spacing(None),
            TextStyleProperty::Foreground => self.set_foreground(None),
        }
    }

    pub fn clear_all(&self) {
        for property in TextStyleProperty::ALL {
            self.clear(property);
        }
    }

    /// Property-wise overlay (指示書 §7): each of the seven properties is resolved independently —
    /// this element's local value wins, `inherited`'s already-resolved value fills every hole.
    /// Never replaces the whole style wholesale (指示書 §33 forbids that).
    pub fn resolve_onto(&self, inherited: &ComputedTextStyle) -> ComputedTextStyle {
        let local = self.local.borrow();
        ComputedTextStyle {
            font_family: local
                .font_family
                .clone()
                .unwrap_or_else(|| inherited.font_family.clone()),
            font_size: local.font_size.unwrap_or(inherited.font_size),
            font_weight: local.font_weight.unwrap_or(inherited.font_weight),
            font_style: local.font_style.unwrap_or(inherited.font_style),
            font_stretch: local.font_stretch.unwrap_or(inherited.font_stretch),
            character_spacing: local
                .character_spacing
                .unwrap_or(inherited.character_spacing),
            foreground: local
                .foreground
                .clone()
                .unwrap_or_else(|| inherited.foreground.clone()),
        }
    }

    /// Overlays local values onto an inherited cascade without inventing backend defaults.
    pub fn cascade_onto(&self, inherited: &CascadedTextStyle) -> CascadedTextStyle {
        let local = self.local.borrow();
        CascadedTextStyle {
            font_family: local
                .font_family
                .clone()
                .or_else(|| inherited.font_family.clone()),
            font_size: local.font_size.or(inherited.font_size),
            font_weight: local.font_weight.or(inherited.font_weight),
            font_style: local.font_style.or(inherited.font_style),
            font_stretch: local.font_stretch.or(inherited.font_stretch),
            character_spacing: local.character_spacing.or(inherited.character_spacing),
            foreground: local
                .foreground
                .clone()
                .or_else(|| inherited.foreground.clone()),
        }
    }
}

/// How text should wrap within its available width (指示書 §21). `TextBlock` doesn't expose this
/// as a DSL property yet (未対応 — outside the seven properties decided for this pass); it's
/// carried in [`TextMeasureRequest`] so adding the DSL field later needs no shape change here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextWrapping {
    #[default]
    NoWrap,
    Wrap,
    WrapWholeWords,
}

/// Everything a backend needs to measure one run of text consistently with how it will later be
/// drawn (指示書 §21's "計測時と描画時で...一致させる" list, minus DPI/text-scale/locale concepts
/// that don't exist anywhere in `elwindui-core` yet — see `docs/elwindui_font_status.md`).
#[derive(Debug, Clone)]
pub struct TextMeasureRequest<'a> {
    pub text: &'a str,
    pub style: &'a ComputedTextStyle,
    /// `f32::INFINITY` on an unconstrained axis, matching the convention `ui::layout_root`
    /// already uses for the root measure pass.
    pub available: Size,
    pub wrapping: TextWrapping,
    pub alignment: super::command::TextAlignment,
    pub max_lines: Option<u32>,
    /// Device scale factor. Always `1.0` today — no DPI/scale concept exists in `elwindui-core`
    /// (未対応, `docs/elwindui_font_status.md`). Carried here so a backend that gains one has a
    /// slot to fill in without another signature change.
    pub scale: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextMeasureResult {
    pub size: Size,
    pub baseline: f32,
    pub line_count: u32,
}

/// The seam a backend implements to give `elwindui-core` real font metrics — registered via
/// [`set_text_backend`], read via [`text_backend`]. `elwindui-core` itself has no font engine
/// (指示書's own premise), so this is the *only* place measurement/default-font knowledge crosses
/// from a backend into core.
pub trait TextBackend {
    /// The platform's own default text style — the final fallback once no ancestor sets a given
    /// property (指示書 §16). Must not hardcode a specific family name; read the platform's actual
    /// default.
    fn default_text_style(&self) -> ComputedTextStyle;

    fn measure_text(&self, req: &TextMeasureRequest<'_>) -> TextMeasureResult;
}

thread_local! {
    static TEXT_BACKEND: RefCell<Option<Rc<dyn TextBackend>>> = const { RefCell::new(None) };
}

/// Registers the backend used by [`text_backend`]. Each backend crate's `init()` calls this once,
/// on the main thread — matching the single-main-thread assumption the rest of the runtime already
/// makes (`invalidate_host: Rc<dyn RelayoutHost>`, `AnyView(Rc<dyn AppKitHandle>)`).
pub fn set_text_backend(backend: Rc<dyn TextBackend>) {
    TEXT_BACKEND.with(|cell| *cell.borrow_mut() = Some(backend));
}

/// Un-registers the current backend, reverting to [`DummyTextBackend`]. Mainly for test hygiene —
/// a backend integration test can restore the deterministic dummy afterward.
pub fn clear_text_backend() {
    TEXT_BACKEND.with(|cell| *cell.borrow_mut() = None);
}

/// The registered backend, or a shared [`DummyTextBackend`] if none is registered (plain
/// `elwindui-core` unit tests). Returns an owned `Rc` rather than exposing a `with(..)`-style
/// closure: callers routinely hold a `RefCell` borrow (e.g. `TextBlock::text`) across the call, and
/// a re-entrant `with` on this same thread_local would be a `BorrowMutError` waiting to happen the
/// first time a backend's own measurement path touched a `TextStyleStorage` recursively.
pub fn text_backend() -> Rc<dyn TextBackend> {
    TEXT_BACKEND.with(|cell| {
        cell.borrow()
            .clone()
            .unwrap_or_else(|| Rc::new(DummyTextBackend) as Rc<dyn TextBackend>)
    })
}

/// Deterministic, font-metrics-free fallback used whenever no real backend is registered — every
/// plain `elwindui-core` unit test runs against this. Its numbers are chosen to exactly reproduce
/// `TextBlock::measure_override`'s pre-existing approximation (`chars().count() * 8.0` wide,
/// `16.0` tall) so none of the pre-existing size assertions in `ui.rs` need to change:
/// `font_size` defaults to `16.0` (via `ComputedTextStyle::fallback`) and each character advances
/// by `0.5 * font_size == 8.0`.
pub struct DummyTextBackend;

impl TextBackend for DummyTextBackend {
    fn default_text_style(&self) -> ComputedTextStyle {
        ComputedTextStyle::fallback()
    }

    fn measure_text(&self, req: &TextMeasureRequest<'_>) -> TextMeasureResult {
        let advance = 0.5 * req.style.font_size;
        let kerning = req.style.character_spacing as f32 / 1000.0 * req.style.font_size;
        let line_height = req.style.font_size;

        let mut lines: Vec<usize> = Vec::new();
        if req.wrapping == TextWrapping::NoWrap || !req.available.width.is_finite() {
            lines.push(req.text.chars().count());
        } else {
            let max_chars = ((req.available.width / (advance + kerning).max(0.01)).floor()
                as usize)
                .max(1);
            let mut remaining = req.text.chars().count();
            if remaining == 0 {
                lines.push(0);
            }
            while remaining > 0 {
                let take = remaining.min(max_chars);
                lines.push(take);
                remaining -= take;
            }
        }

        let width = lines
            .iter()
            .map(|&count| count as f32 * (advance + kerning))
            .fold(0.0_f32, f32::max);
        let height = lines.len() as f32 * line_height;

        TextMeasureResult {
            size: Size { width, height },
            baseline: 0.8 * req.style.font_size,
            line_count: lines.len() as u32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics::Color;

    #[test]
    fn font_weight_constants_are_ordered() {
        assert!(FontWeight::THIN < FontWeight::NORMAL);
        assert!(FontWeight::NORMAL < FontWeight::BOLD);
        assert!(FontWeight::BOLD < FontWeight::BLACK);
    }

    #[test]
    fn font_stretch_percent_is_monotone() {
        let all = [
            FontStretch::UltraCondensed,
            FontStretch::ExtraCondensed,
            FontStretch::Condensed,
            FontStretch::SemiCondensed,
            FontStretch::Normal,
            FontStretch::SemiExpanded,
            FontStretch::Expanded,
            FontStretch::ExtraExpanded,
            FontStretch::UltraExpanded,
        ];
        for pair in all.windows(2) {
            assert!(pair[0].percent() < pair[1].percent());
        }
        assert_eq!(FontStretch::Normal.percent(), 100.0);
    }

    #[test]
    fn font_family_families_splits_and_trims() {
        let family = FontFamily::new("Helvetica, Arial ,  sans-serif");
        assert_eq!(
            family.families().collect::<Vec<_>>(),
            vec!["Helvetica", "Arial", "sans-serif"]
        );
    }

    #[test]
    fn font_family_system_sentinel() {
        assert!(FontFamily::system().is_system());
        assert!(!FontFamily::new("Helvetica").is_system());
        assert!(FontFamily::default().is_system());
    }

    #[test]
    fn text_style_values_default_is_all_unset() {
        let values = TextStyleValues::default();
        assert_eq!(values.font_family, None);
        assert_eq!(values.font_size, None);
        assert_eq!(values.font_weight, None);
        assert_eq!(values.font_style, None);
        assert_eq!(values.font_stretch, None);
        assert_eq!(values.character_spacing, None);
        assert_eq!(values.foreground, None);
    }

    #[test]
    fn resolve_onto_local_wins_per_property() {
        let storage = TextStyleStorage::new();
        storage.set_font_size(Some(20.0));
        let inherited = ComputedTextStyle {
            font_family: FontFamily::new("Inherited"),
            font_size: 12.0,
            font_weight: FontWeight::BOLD,
            font_style: FontStyle::Italic,
            font_stretch: FontStretch::Condensed,
            character_spacing: 50,
            foreground: Brush::Solid(Color::white()),
        };
        let computed = storage.resolve_onto(&inherited);
        assert_eq!(computed.font_size, 20.0); // local wins
        assert_eq!(computed.font_family, inherited.font_family); // inherited, untouched
        assert_eq!(computed.font_weight, inherited.font_weight);
        assert_eq!(computed.character_spacing, inherited.character_spacing);
    }

    #[test]
    fn resolve_onto_all_unset_equals_inherited() {
        let storage = TextStyleStorage::new();
        let inherited = ComputedTextStyle::fallback();
        assert_eq!(storage.resolve_onto(&inherited), inherited);
    }

    #[test]
    fn setter_returns_false_on_no_op_write() {
        let storage = TextStyleStorage::new();
        assert!(storage.set_font_size(Some(20.0)));
        assert!(!storage.set_font_size(Some(20.0))); // same value again -> no-op
        assert!(storage.set_font_size(Some(24.0))); // different value -> real change
    }

    #[test]
    fn clear_returns_whether_a_value_was_set() {
        let storage = TextStyleStorage::new();
        assert!(!storage.clear(TextStyleProperty::FontSize)); // nothing to clear
        storage.set_font_size(Some(20.0));
        assert!(storage.clear(TextStyleProperty::FontSize));
        assert_eq!(storage.font_size(), None);
    }

    #[test]
    fn clear_all_only_touches_local_flags() {
        let storage = TextStyleStorage::new();
        storage.set_font_size(Some(20.0));
        storage.set_font_family(Some(FontFamily::new("Helvetica")));
        storage.clear_all();
        assert_eq!(storage.font_size(), None);
        assert_eq!(storage.font_family(), None);
    }

    #[test]
    fn font_size_local_flag_independent_of_font_family() {
        let storage = TextStyleStorage::new();
        storage.set_font_size(Some(20.0));
        assert!(storage.is_set(TextStyleProperty::FontSize));
        assert!(!storage.is_set(TextStyleProperty::FontFamily));
    }

    #[test]
    fn dummy_text_backend_reproduces_legacy_metrics() {
        let backend = DummyTextBackend;
        let style = ComputedTextStyle::fallback();
        let result = backend.measure_text(&TextMeasureRequest {
            text: "hello",
            style: &style,
            available: Size {
                width: f32::INFINITY,
                height: f32::INFINITY,
            },
            wrapping: TextWrapping::NoWrap,
            alignment: super::super::command::TextAlignment::Left,
            max_lines: None,
            scale: 1.0,
        });
        assert_eq!(result.size.width, 5.0 * 8.0);
        assert_eq!(result.size.height, 16.0);
    }

    #[test]
    fn text_backend_falls_back_to_dummy_when_unregistered() {
        clear_text_backend();
        let backend = text_backend();
        assert_eq!(backend.default_text_style(), ComputedTextStyle::fallback());
    }

    #[test]
    fn set_and_clear_text_backend_round_trip() {
        struct RecordingBackend;
        impl TextBackend for RecordingBackend {
            fn default_text_style(&self) -> ComputedTextStyle {
                ComputedTextStyle {
                    font_size: 99.0,
                    ..ComputedTextStyle::fallback()
                }
            }
            fn measure_text(&self, _req: &TextMeasureRequest<'_>) -> TextMeasureResult {
                TextMeasureResult {
                    size: Size {
                        width: 0.0,
                        height: 0.0,
                    },
                    baseline: 0.0,
                    line_count: 0,
                }
            }
        }
        set_text_backend(Rc::new(RecordingBackend));
        assert_eq!(text_backend().default_text_style().font_size, 99.0);
        clear_text_backend();
        assert_eq!(
            text_backend().default_text_style().font_size,
            ComputedTextStyle::fallback().font_size
        );
    }
}
