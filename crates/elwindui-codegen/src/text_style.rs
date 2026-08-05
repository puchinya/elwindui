//! The seven fields `#[text_style]` (指示書 §9, `docs/specs/dsl_spec.md` 付録A) injects into a
//! component's field set — shared by `parser.rs` (injection) and `validate.rs` (duplicate-name
//! checking) so the two can never drift out of sync with each other.

#[cfg(test)]
use crate::ast::{Attr, FieldDef, FieldKind};

/// `(field name, declared DSL type)`. The `foreground` type string must stay byte-identical to
/// `Shape.fill`'s own declared type — `codegen::coerce_color_literal` matches
/// on the literal path `"elwindui::core::graphics::Brush"`, so `foreground: "#3a3a3c"` only keeps
/// working through that existing mechanism if the spelling agrees exactly.
pub(crate) const TEXT_STYLE_FIELDS: [(&str, &str); 7] = [
    ("font_family", "Option<elwindui::core::graphics::FontFamily>"),
    ("font_size", "Option<f32>"),
    ("font_weight", "Option<elwindui::core::graphics::FontWeight>"),
    ("font_style", "Option<elwindui::core::graphics::FontStyle>"),
    (
        "font_stretch",
        "Option<elwindui::core::graphics::FontStretch>",
    ),
    ("character_spacing", "Option<i32>"),
    ("foreground", "Option<elwindui::core::graphics::Brush>"),
];

/// Builds the seven injected `FieldDef`s, in the order above — `parser.rs`'s test-only
/// `Parser::parse_module` prepends these to a `#[text_style]` component's own hand-written fields
/// (指示書 §9's ban on hand-writing the same six-plus-one properties per component). The macro-path
/// frontend (`component_frontend.rs`, production) injects the same fields its own way and never
/// calls this.
#[cfg(test)]
pub(crate) fn text_style_field_defs() -> Vec<FieldDef> {
    TEXT_STYLE_FIELDS
        .iter()
        .map(|(name, ty)| FieldDef {
            name: (*name).to_string(),
            ty: (*ty).to_string(),
            kind: FieldKind::Prop,
            attrs: vec![Attr::TextStyle],
            initializer: None,
        })
        .collect()
}

/// Every field name `#[text_style]` injects — used by `validate.rs` to reject a component that
/// also hand-declares one of these itself.
pub(crate) fn is_text_style_field_name(name: &str) -> bool {
    TEXT_STYLE_FIELDS.iter().any(|(field_name, _)| *field_name == name)
}
