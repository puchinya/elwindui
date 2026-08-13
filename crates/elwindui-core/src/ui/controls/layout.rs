//! The abstract base for the self-drawn layout panels (`VerticalLayout`/`HorizontalLayout`/`Grid`).

use super::*;

/// `Layout`'s own class trait (docs/design/runtime/ui_tree_design.md) — empty marker over `UIElement`,
/// implemented by every layout-container virtual builtin (`VerticalLayout`/
/// `HorizontalLayout`/`Grid`), the same way `NativeControl` groups every native leaf.
///
/// Holds `children` plus an optional, explicitly assigned background shared by every layout
/// container. An unset background remains transparent; nothing applies a background implicitly.
/// `spacing` is *not* here: it only means anything to
/// `VerticalLayout`/`HorizontalLayout` (`Grid` has no use for it), so each of those two declares
/// its own `spacing` field instead of it living on this shared base. `VerticalLayout`/
/// `HorizontalLayout` do their own layout math directly against `elwindui_core::layout`'s
/// `stack_arrange`/`stack_natural_size` free functions with their own fixed `Orientation` literal —
/// neither delegates its `measure_override`/`arrange_override` to this struct's own (trivial, "take
/// no space" — see `UIElement::measure_override`'s own doc comment) default, since the orientation
/// (and so the entire layout algorithm) is a property of *which concrete type this is*, not of
/// shared state a common base could hold.
///
/// `abstract_class`: `Layout` itself is never instantiated (no `new`, and `#[class]`'s
/// `abstract_class` never auto-generates one even though `Layout` defines `construct` below) — only
/// its concrete subclasses (`VerticalLayout`/`HorizontalLayout`) are, each calling `Layout::
/// construct()` for their own `base` field (see e.g. `Shape::construct`/`Control::construct` for the
/// same shape one level up the hierarchy, where the base *is* directly instantiable).
#[elwindui_macros::class(inherits = crate::ui::UIElement, abstract_class)]
#[prop(children: crate::ui::UIElementCollection)]
#[prop(background: Option<crate::graphics::Brush>)]
pub struct Layout {
    /// Logical children for this layout. Its mutations update the owner's Visual collection.
    pub children: UIElementCollection,
    /// An explicitly assigned background, or `None` for transparent platform-neutral layout.
    pub background: RefCell<Option<Brush>>,
}

#[elwindui_macros::class]
impl Layout {
    /// Not `#[inherent]` — a plain method here becomes a default `LayoutExt` trait method
    /// (dispatched through `__dyn_layout`, docs/specs/macro_class_spec.md), so
    /// `VerticalLayout`/`HorizontalLayout`/`Grid` all get `self.children()` for free without
    /// redeclaring it themselves, the same way every `UIElement` (root class) method is inherited
    /// by every concrete leaf/container for free.
    fn children(&self) -> &UIElementCollection {
        &self.children
    }

    /// Returns the explicitly assigned layout background.
    fn background(&self) -> Option<Brush> {
        self.background.borrow().clone()
    }

    /// Sets an explicit background drawn behind arranged children.
    fn set_background(&self, background: Option<Brush>) {
        *self.background.borrow_mut() = background;
        self.invalidate();
    }

    /// Restores the layout's transparent default background.
    fn clear_background(&self) {
        self.set_background(None);
    }

    /// Matching WinUI Panel behavior, only a layout with an actual background participates in
    /// hit-testing. `None` is both visually transparent and hit-test transparent.
    #[overrides]
    fn hit_test_content(&self) -> bool {
        self.background.borrow().is_some()
    }

    /// A retained group's own commands precede all child groups, so this fill is guaranteed to be
    /// behind the layout's children.
    #[overrides]
    fn render(&self, context: &mut RenderContext<'_>) {
        if let Some(background) = self.background.borrow().as_ref() {
            context.fill_rect(
                Rect {
                    x: 0.0,
                    y: 0.0,
                    width: self.arranged_width().unwrap_or(0.0),
                    height: self.arranged_height().unwrap_or(0.0),
                },
                background,
            );
        }
    }

    fn construct() -> Self {
        let base = UIElement::construct();
        let children = UIElementCollection::new(__self_weak.clone());
        Self {
            base,
            children,
            background: RefCell::new(None),
        }
    }
}
