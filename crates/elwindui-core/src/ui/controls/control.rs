//! `builtin::Control` — the self-drawn templated-control base, and its local text-style storage.

use super::*;

/// A composable, multi-part component (WinUI3's `Control`) — Visually built from any number of
/// other `UIElement`s (`VerticalLayout`/`HorizontalLayout`/`Shape`/`TextBlock`/
/// `NativeControlImpl`/other `Control`s), stored as its own `UIElementCollection` (the Logical
/// tree this component declares, docs/design/gui_framework_design.md §5.2) — unlike `Shape`, which has
/// no children at all. `padding` shrinks the area its children are overlaid into, the
/// `Control`-level analog of `margin` on an individual element.
///
/// Scope note: this is intentionally minimal for now — `content_horizontal_alignment`/
/// `content_vertical_alignment` are stored but not yet consulted by `arrange_override` (each
/// child's *own* `horizontal_alignment`/`vertical_alignment`, applied generically by `arrange`
/// below, already governs its placement within the padded content area); template
/// replacement is future work.
/// `Control`'s own class trait (docs/design/gui_framework_design.md §5.1) — exposes the fields a
/// DSL-level subclass composed via `base: Control` (e.g. `builtin::ContentControl`,
/// `elwindui-core::ui`) delegates to.
#[elwindui_macros::class(inherits = crate::ui::UIElement)]
#[text_style]
#[content(children)]
#[prop(children: crate::ui::UIElementCollection)]
#[prop(padding: Option<f32>)]
pub struct Control {
    pub padding: Cell<f32>,
    pub content_horizontal_alignment: Cell<HorizontalAlignment>,
    pub content_vertical_alignment: Cell<VerticalAlignment>,
    /// `Control`-level font/foreground properties (指示書 §10: "Control派生型からも、基底の
    /// フォントプロパティをDSLで直接指定できること") — inherited by any Visual descendant via
    /// [`TextStyleOwner`], regardless of whether the elements in between are themselves owners.
    pub text_style: crate::graphics::TextStyleStorage,
}

#[elwindui_macros::class]
impl Control {
    #[overrides]
    fn measure_override(&self, available: Size) -> Size {
        let inner = self
            .visual_children()
            .iter()
            .fold(Size::default(), |acc, c| {
                c.measure(available);
                let s = c.measured_size().unwrap_or_default();
                Size {
                    width: acc.width.max(s.width),
                    height: acc.height.max(s.height),
                }
            });
        grow_by_margin(inner, self.padding.get())
    }
    #[overrides]
    fn arrange_override(&self, final_size: Size) -> Size {
        let full = Rect {
            x: 0.0,
            y: 0.0,
            width: final_size.width,
            height: final_size.height,
        };
        let content_area = shrink_rect_by_margin(full, self.padding.get());
        for child in self.visual_children().iter() {
            child.arrange(content_area);
        }
        final_size
    }
    fn padding(&self) -> f32 {
        self.padding.get()
    }
    fn content_horizontal_alignment(&self) -> HorizontalAlignment {
        self.content_horizontal_alignment.get()
    }
    fn content_vertical_alignment(&self) -> VerticalAlignment {
        self.content_vertical_alignment.get()
    }
    /// `Control`/`ContentControl` have no `Background`/`Fill` concept either — see
    /// `Layout::hit_test_content`'s own doc comment for the identical rationale.
    #[overrides]
    fn hit_test_content(&self) -> bool {
        false
    }
    #[overrides]
    fn as_text_style_owner(&self) -> Option<&dyn TextStyleOwner> {
        Some(self)
    }
    fn set_padding(&self, padding: f32) {
        self.padding.set(padding);
        self.invalidate_measure();
    }
    fn set_content_horizontal_alignment(&self, alignment: HorizontalAlignment) {
        self.content_horizontal_alignment.set(alignment);
        self.invalidate_arrange();
    }
    fn set_content_vertical_alignment(&self, alignment: VerticalAlignment) {
        self.content_vertical_alignment.set(alignment);
        self.invalidate_arrange();
    }
    fn construct() -> Self {
        Self {
            base: UIElement::construct(),
            padding: Cell::new(0.0),
            content_horizontal_alignment: Cell::new(HorizontalAlignment::Stretch),
            content_vertical_alignment: Cell::new(VerticalAlignment::Stretch),
            text_style: crate::graphics::TextStyleStorage::new(),
        }
    }
}

impl TextStyleOwner for Control {
    fn text_style_storage(&self) -> &crate::graphics::TextStyleStorage {
        &self.text_style
    }

    fn cascaded_text_style(&self) -> crate::graphics::CascadedTextStyle {
        let inherited = inherited_cascaded_text_style(self.as_ui_element());
        let mut style = self.text_style_storage().cascade_onto(&inherited);
        apply_standard_text_theme(&self.theme_handle(), "control", &mut style);
        style
    }
}
