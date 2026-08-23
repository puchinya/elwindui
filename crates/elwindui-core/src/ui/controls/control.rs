//! `elwindui::ui::Control` — the self-drawn templated-control base, and its local text-style storage.

use super::*;

/// A composable, templated component base (WinUI3's `Control`). `Control` has no public collection
/// content slot: its visual presentation is owned by the private template-root path. A component
/// body that inherits `Control` may provide one authored visual root, which is attached through
/// that path; collection content belongs to `Layout`, while a single logical content slot belongs
/// to `ContentControl`. `padding` shrinks the area its visual root is arranged into, the
/// `Control`-level analog of `margin` on an individual element.
///
/// Scope note: this is intentionally minimal for now — `content_horizontal_alignment`/
/// `content_vertical_alignment` are stored but not yet consulted by `arrange_override` (each
/// child's *own* `horizontal_alignment`/`vertical_alignment`, applied generically by `arrange`
/// below, already governs its placement within the padded content area).
/// `Control`'s own class trait (docs/design/runtime/ui_tree_design.md) — exposes the fields a
/// DSL-level subclass composed via `base: Control` (e.g. `elwindui::ui::ContentControl`,
/// `elwindui-core::ui`) delegates to.
#[elwindui_macros::class(inherits = crate::ui::UIElement)]
#[text_style]
#[prop(padding: Option<f32>)]
pub struct Control {
    pub padding: Cell<f32>,
    pub content_horizontal_alignment: Cell<HorizontalAlignment>,
    pub content_vertical_alignment: Cell<VerticalAlignment>,
    template_root: RefCell<Option<Rc<dyn UIElementExt>>>,
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
    /// Lets `ContentControl` descendants detach direct content before a template root is built.
    #[doc(hidden)]
    #[overridable]
    fn __prepare_template_presentation(&self) {}
    /// Returns the selected template root, if this control is template-enabled and mounted.
    #[doc(hidden)]
    fn __template_root(&self) -> Option<Rc<dyn UIElementExt>> {
        self.template_root.borrow().clone()
    }
    /// Replaces the Visual template root without adding it to the logical tree.
    #[doc(hidden)]
    fn __set_template_root(&self, root: Rc<dyn UIElementExt>) {
        if let Some(old) = self.template_root.borrow_mut().take() {
            self.as_ui_element().visual_collection.remove(&old);
        }
        self.as_ui_element().visual_collection.add(root.clone());
        *self.template_root.borrow_mut() = Some(root);
        self.invalidate_measure();
    }
    fn construct() -> Self {
        Self {
            base: UIElement::construct(),
            padding: Cell::new(0.0),
            content_horizontal_alignment: Cell::new(HorizontalAlignment::Stretch),
            content_vertical_alignment: Cell::new(VerticalAlignment::Stretch),
            template_root: RefCell::new(None),
            text_style: crate::graphics::TextStyleStorage::new(),
        }
    }
}

impl TextStyleOwner for Control {
    fn text_style_storage(&self) -> &crate::graphics::TextStyleStorage {
        &self.text_style
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::testsupport::*;

    #[test]
    fn control_padding_shrinks_the_slot_its_children_are_arranged_into() {
        let control = ContentControl::new();
        control.set_padding(10.0);
        control.set_content(native("a", size(10.0, 20.0)));
        let tree: Rc<dyn UIElementExt> = control;
        let (natives, _) = split(layout_tree::<FakeHandle>(&tree, size(100.0, 100.0)));
        assert_eq!(
            natives[0].1,
            Rect {
                x: 10.0,
                y: 10.0,
                width: 80.0,
                height: 80.0
            }
        );
    }

    #[test]
    fn template_root_replacement_is_visual_only_and_invalidates_measure() {
        let control = Control::new();
        let owner: Rc<dyn UIElementExt> = control.clone();
        let first = TextBlock::new();
        let first_node: Rc<dyn UIElementExt> = first.clone();

        control.measure(Size {
            width: 100.0,
            height: 100.0,
        });
        assert!(control.measured_size().is_some());
        control.__set_template_root(first_node.clone());
        assert!(control.measured_size().is_none());
        assert!(first.as_ui_element().parent.borrow().is_none());
        assert!(
            first
                .visual_parent()
                .is_some_and(|parent| Rc::ptr_eq(&parent, &owner))
        );

        let second = TextBlock::new();
        let second_node: Rc<dyn UIElementExt> = second.clone();
        control.__set_template_root(second_node.clone());
        assert!(first.visual_parent().is_none());
        assert!(second.as_ui_element().parent.borrow().is_none());
        assert_eq!(control.visual_children().len(), 1);
        assert!(Rc::ptr_eq(
            &control.__template_root().expect("template root"),
            &second_node
        ));

        control.__set_template_root(second_node);
        assert_eq!(control.visual_children().len(), 1);
    }

    #[test]
    fn template_root_parent_links_do_not_form_a_strong_cycle() {
        let control = Control::new();
        let root = TextBlock::new();
        let weak_root = Rc::downgrade(&root);
        control.__set_template_root(root.clone());
        drop(root);
        drop(control);
        assert!(weak_root.upgrade().is_none());
    }
}
