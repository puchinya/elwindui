//! Font/text-style inheritance: which parent link the cascade follows
//! ([`InheritanceParentKind`]), the [`TextStyleOwner`] capability trait an element implements to
//! hold its own local overrides, and the two entry points that materialize an inherited style.
//!
//! Orthogonal to the class hierarchy on purpose — `TextStyleOwner` is implemented by both native
//! (`NativeControl`) and self-drawn (`TextBlock`/`Control`) elements, which have no common ancestor
//! below `UIElement`.

use super::*;

/// Which parent link `UIElementExt::inheritance_parent` should follow (指示書 §13/§14). WinUI3
/// doesn't keep a dedicated third "inheritance parent" pointer — it picks, at each inheritance walk,
/// between the two links every element already has (`GetInheritanceParentInternal()`):
///
/// ```text
/// Visual:  VisualTreeHelper.GetParent — the normal path. Font inheritance always uses this.
/// Logical: the logical parent, falling back to Visual if there is none — used by properties like
///          DataContext that prefer logical ownership (ContentControl.Content, Popup boundaries, ...).
/// ```
///
/// Font inheritance must not be hardcoded to the Logical tree (指示書 §13 forbids this explicitly).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InheritanceParentKind {
    Visual,
    Logical,
}

/// Capability trait for an element that can hold its own local font/text-style values — `Control`,
/// `TextBlock`, and each backend's own `NativeControl` all implement this (指示書 §5/§8). Not a
/// `#[class]`-managed class: those three are siblings on the single-inheritance chain (`Control`/
/// `TextBlock` both inherit `UIElement` directly; a backend's `NativeControl` also inherits
/// `UIElement`, not `Control`), so this is a plain, hand-written, orthogonal capability trait —
/// exactly the shape `AsAny`/`RelayoutHost`/`FocusHost` already use for "some but not all elements
/// need this", rather than threading it through the `inherits =` chain.
///
/// `TextStyleOwner` intentionally does *not* expose one `text_style` property to the DSL (指示書
/// §8 rules this out) — its only job is: hold a [`crate::graphics::TextStyleStorage`], forward each
/// of the seven individual setters to it with per-property change notification, and resolve this
/// element's own [`crate::graphics::ComputedTextStyle`] against its inherited value.
pub trait TextStyleOwner: UIElementExt {
    /// The single piece of real state an implementor provides.
    fn text_style_storage(&self) -> &crate::graphics::TextStyleStorage;

    fn font_family(&self) -> Option<crate::graphics::FontFamily> {
        self.text_style_storage().font_family()
    }
    // Bare (non-`Option`) setter argument, matching the house convention every other
    // `Option<T>`-declared-in-the-DSL/scalar-or-enum-typed common property already uses
    // (`UIElement::set_width(&self, width: f32)`, `set_margin`, ...): "unset" is expressed purely
    // by never calling the setter (or by calling `clear_font_family()`), never by passing an
    // explicit `None` — no DSL syntax spells that anyway, and this keeps `elwindui-codegen`'s
    // generic per-field setter emission (`build_component_setters`/`build_virtual_value`) applying
    // uniformly with no per-field-name special case. `set_foreground` below is the one exception,
    // matching `Shape::set_fill`/`set_stroke`'s own established `Option<Brush>` convention instead.
    fn set_font_family(&self, value: crate::graphics::FontFamily) {
        if self.text_style_storage().set_font_family(Some(value)) {
            self.on_text_style_property_changed(crate::graphics::TextStyleProperty::FontFamily);
        }
    }
    fn clear_font_family(&self) {
        if self
            .text_style_storage()
            .clear(crate::graphics::TextStyleProperty::FontFamily)
        {
            self.on_text_style_property_changed(crate::graphics::TextStyleProperty::FontFamily);
        }
    }

    fn font_size(&self) -> Option<f32> {
        self.text_style_storage().font_size()
    }
    fn set_font_size(&self, value: f32) {
        if self.text_style_storage().set_font_size(Some(value)) {
            self.on_text_style_property_changed(crate::graphics::TextStyleProperty::FontSize);
        }
    }
    fn clear_font_size(&self) {
        if self
            .text_style_storage()
            .clear(crate::graphics::TextStyleProperty::FontSize)
        {
            self.on_text_style_property_changed(crate::graphics::TextStyleProperty::FontSize);
        }
    }

    fn font_weight(&self) -> Option<crate::graphics::FontWeight> {
        self.text_style_storage().font_weight()
    }
    fn set_font_weight(&self, value: crate::graphics::FontWeight) {
        if self.text_style_storage().set_font_weight(Some(value)) {
            self.on_text_style_property_changed(crate::graphics::TextStyleProperty::FontWeight);
        }
    }
    fn clear_font_weight(&self) {
        if self
            .text_style_storage()
            .clear(crate::graphics::TextStyleProperty::FontWeight)
        {
            self.on_text_style_property_changed(crate::graphics::TextStyleProperty::FontWeight);
        }
    }

    fn font_style(&self) -> Option<crate::graphics::FontStyle> {
        self.text_style_storage().font_style()
    }
    fn set_font_style(&self, value: crate::graphics::FontStyle) {
        if self.text_style_storage().set_font_style(Some(value)) {
            self.on_text_style_property_changed(crate::graphics::TextStyleProperty::FontStyle);
        }
    }
    fn clear_font_style(&self) {
        if self
            .text_style_storage()
            .clear(crate::graphics::TextStyleProperty::FontStyle)
        {
            self.on_text_style_property_changed(crate::graphics::TextStyleProperty::FontStyle);
        }
    }

    fn font_stretch(&self) -> Option<crate::graphics::FontStretch> {
        self.text_style_storage().font_stretch()
    }
    fn set_font_stretch(&self, value: crate::graphics::FontStretch) {
        if self.text_style_storage().set_font_stretch(Some(value)) {
            self.on_text_style_property_changed(crate::graphics::TextStyleProperty::FontStretch);
        }
    }
    fn clear_font_stretch(&self) {
        if self
            .text_style_storage()
            .clear(crate::graphics::TextStyleProperty::FontStretch)
        {
            self.on_text_style_property_changed(crate::graphics::TextStyleProperty::FontStretch);
        }
    }

    fn character_spacing(&self) -> Option<i32> {
        self.text_style_storage().character_spacing()
    }
    fn set_character_spacing(&self, value: i32) {
        if self.text_style_storage().set_character_spacing(Some(value)) {
            self.on_text_style_property_changed(
                crate::graphics::TextStyleProperty::CharacterSpacing,
            );
        }
    }
    fn clear_character_spacing(&self) {
        if self
            .text_style_storage()
            .clear(crate::graphics::TextStyleProperty::CharacterSpacing)
        {
            self.on_text_style_property_changed(
                crate::graphics::TextStyleProperty::CharacterSpacing,
            );
        }
    }

    fn foreground(&self) -> Option<crate::graphics::Brush> {
        self.text_style_storage().foreground()
    }
    fn set_foreground(&self, value: Option<crate::graphics::Brush>) {
        if self.text_style_storage().set_foreground(value) {
            self.on_text_style_property_changed(crate::graphics::TextStyleProperty::Foreground);
        }
    }
    fn clear_foreground(&self) {
        self.set_foreground(None);
    }

    /// Clears exactly one property's local value.
    fn clear_text_style_property(&self, property: crate::graphics::TextStyleProperty) {
        if self.text_style_storage().clear(property) {
            self.on_text_style_property_changed(property);
        }
    }
    /// Clears every local text-style value on this element, reverting all seven to inheritance.
    fn clear_text_style(&self) {
        for property in crate::graphics::TextStyleProperty::ALL {
            self.clear_text_style_property(property);
        }
    }

    /// Change notification + invalidation hook (指示書 §23). The default routes each property to
    /// the weakest sufficient invalidation: `Foreground` only ever affects painting, everything
    /// else can affect measured size (a wider/heavier/larger glyph run).
    fn on_text_style_property_changed(&self, property: crate::graphics::TextStyleProperty) {
        match property {
            crate::graphics::TextStyleProperty::Foreground => self.invalidate_render(),
            _ => self.invalidate_measure(),
        }
    }

    /// This element's inherited/local cascade without backend defaults. Native adapters consume
    /// this form so an absent value can clear a platform property.
    fn cascaded_text_style(&self) -> crate::graphics::CascadedTextStyle {
        let inherited = inherited_cascaded_text_style(self.as_ui_element());
        self.text_style_storage().cascade_onto(&inherited)
    }

    /// This element's fully materialized style for framework-owned measurement and painting.
    fn resolved_text_style(&self) -> crate::graphics::ComputedTextStyle {
        self.cascaded_text_style()
            .materialize(&crate::graphics::text_backend().default_text_style())
    }
}

/// The style this element inherits before backend defaults are materialized. Always walks the
/// **Visual** tree: a
/// `Grid`/`Layout`/`Shape`/`Image` in between is transparent (its `as_text_style_owner()` is `None`,
/// so the walk simply continues past it) — inheritance is never blocked by a non-owning element
/// (指示書 §11).
pub fn inherited_cascaded_text_style(base: &UIElement) -> crate::graphics::CascadedTextStyle {
    // `base` is the bare `UIElement` struct, not a `dyn UIElementExt` — read its `visual_parent`
    // field directly for this first hop (mirroring `request_relayout`'s identical first step);
    // every subsequent hop is a real `Rc<dyn UIElementExt>`, which does implement the trait method.
    let mut current: Option<Rc<dyn UIElementExt>> = base
        .visual_parent
        .borrow()
        .as_ref()
        .and_then(|w| w.upgrade());
    while let Some(element) = current {
        if let Some(owner) = element.as_text_style_owner() {
            return owner.cascaded_text_style();
        }
        current = element.inheritance_parent(InheritanceParentKind::Visual);
    }
    crate::graphics::CascadedTextStyle::default()
}

/// Returns the inherited text style materialized for framework-owned drawing and measurement.
///
/// Native controls should use [`inherited_cascaded_text_style`] instead, preserving unset values
/// until their platform property adapter.
pub fn inherited_text_style(base: &UIElement) -> crate::graphics::ComputedTextStyle {
    inherited_cascaded_text_style(base)
        .materialize(&crate::graphics::text_backend().default_text_style())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::testsupport::*;

    #[derive(Default)]
    struct KindRecordingHost {
        kinds: std::cell::RefCell<Vec<crate::ui::InvalidationKind>>,
    }

    impl crate::ui::RelayoutHost for KindRecordingHost {
        fn request_relayout(&self, _dirty_group_id: u64, kind: crate::ui::InvalidationKind) {
            self.kinds.borrow_mut().push(kind);
        }
    }

    #[test]
    fn as_text_style_owner_is_none_for_non_owning_elements() {
        // Grid/Layout/Shape must stay transparent to inheritance (指示書 §11) — verified directly
        // via the downcast hook rather than only indirectly through an inheritance chain.
        let grid = Grid::new();
        assert!(grid.as_text_style_owner().is_none());
        let stack = VerticalLayout::new();
        assert!(stack.as_text_style_owner().is_none());
        let rect = Rectangle::new();
        assert!(rect.as_text_style_owner().is_none());
    }

    #[test]
    fn as_text_style_owner_is_some_for_control_and_text_block() {
        let control = Control::new();
        assert!(control.as_text_style_owner().is_some());
        let text_block = TextBlock::new();
        assert!(text_block.as_text_style_owner().is_some());
    }

    #[test]
    fn orphan_text_block_resolves_to_backend_default() {
        let text_block = TextBlock::new();
        let style = text_block.resolved_text_style();
        assert_eq!(style, crate::graphics::text_backend().default_text_style());
    }

    #[test]
    fn control_font_size_inherits_through_grid_to_nested_text_block() {
        // Control -(Visual)-> Grid -(Visual)-> TextBlock: Grid is not a TextStyleOwner, so it must
        // not block inheritance (指示書 §11's own worked example).
        let control = Control::new();
        control.set_font_size(24.0);
        let grid = Grid::new();
        let text_block = TextBlock::new();
        grid.children().add(text_block.clone());
        control.as_ui_element().visual_collection.add(grid.clone());

        let style = text_block.resolved_text_style();
        assert_eq!(style.font_size, 24.0);
    }

    #[test]
    fn child_local_value_wins_over_inherited() {
        let control = Control::new();
        control.set_font_size(24.0);
        let text_block = TextBlock::new();
        text_block.set_font_size(12.0);
        control
            .as_ui_element()
            .visual_collection
            .add(text_block.clone());

        assert_eq!(text_block.resolved_text_style().font_size, 12.0);
    }

    #[test]
    fn child_partial_override_leaves_other_properties_inherited() {
        // Setting only `font_size` locally must not disturb `font_family`/`font_weight`/etc. —
        // each of the seven properties resolves independently (指示書 §7/§19, never a wholesale
        // "inherit the whole struct" replacement).
        let control = Control::new();
        control.set_font_size(24.0);
        control.set_font_family(crate::graphics::FontFamily::new("Helvetica"));
        control.set_font_weight(crate::graphics::FontWeight::BOLD);
        let text_block = TextBlock::new();
        text_block.set_font_size(12.0);
        control
            .as_ui_element()
            .visual_collection
            .add(text_block.clone());

        let style = text_block.resolved_text_style();
        assert_eq!(style.font_size, 12.0); // local wins
        assert_eq!(
            style.font_family,
            crate::graphics::FontFamily::new("Helvetica")
        ); // inherited
        assert_eq!(style.font_weight, crate::graphics::FontWeight::BOLD); // inherited
    }

    #[test]
    fn clear_font_size_reverts_to_inherited_value() {
        let control = Control::new();
        control.set_font_size(24.0);
        let text_block = TextBlock::new();
        text_block.set_font_size(12.0);
        control
            .as_ui_element()
            .visual_collection
            .add(text_block.clone());
        assert_eq!(text_block.resolved_text_style().font_size, 12.0);

        text_block.clear_font_size();
        assert_eq!(text_block.resolved_text_style().font_size, 24.0);
    }

    #[test]
    fn setting_font_size_invalidates_measure_but_foreground_only_invalidates_render() {
        let text_block = TextBlock::new();
        let host = Rc::new(KindRecordingHost::default());
        text_block.set_invalidate_host(Some(host.clone() as Rc<dyn RelayoutHost>));
        layout_root(
            &(text_block.clone() as Rc<dyn UIElementExt>),
            size(100.0, 100.0),
        );
        assert!(text_block.measured_size().is_some());
        assert!(text_block.arranged_width().is_some());

        text_block.set_font_size(20.0);
        assert!(
            text_block.measured_size().is_none(),
            "a font-size change must invalidate measure"
        );

        layout_root(
            &(text_block.clone() as Rc<dyn UIElementExt>),
            size(100.0, 100.0),
        );
        assert!(text_block.measured_size().is_some());
        host.kinds.borrow_mut().clear();
        text_block.set_foreground(Some(crate::graphics::Brush::Solid(
            crate::graphics::Color::white(),
        )));
        assert!(
            text_block.measured_size().is_some(),
            "a foreground-only change must not invalidate measure"
        );
        assert!(
            text_block.arranged_width().is_some(),
            "a foreground-only change must not invalidate arrange"
        );
        assert!(text_block.arranged_height().is_some());
        assert!(text_block.arranged_offset().is_some());
        assert_eq!(
            &*host.kinds.borrow(),
            &[crate::ui::InvalidationKind::Render]
        );
    }

    #[test]
    fn reparenting_text_block_re_resolves_from_the_new_parent() {
        let old_parent = Control::new();
        old_parent.set_font_size(10.0);
        let new_parent = Control::new();
        new_parent.set_font_size(30.0);
        let text_block = TextBlock::new();
        old_parent
            .as_ui_element()
            .visual_collection
            .add(text_block.clone());
        assert_eq!(text_block.resolved_text_style().font_size, 10.0);

        old_parent
            .as_ui_element()
            .visual_collection
            .remove(&(text_block.clone() as Rc<dyn UIElementExt>));
        new_parent
            .as_ui_element()
            .visual_collection
            .add(text_block.clone());
        assert_eq!(text_block.resolved_text_style().font_size, 30.0);
    }

    #[test]
    fn removed_from_parent_falls_back_to_backend_default() {
        let parent = Control::new();
        parent.set_font_size(30.0);
        let text_block = TextBlock::new();
        parent
            .as_ui_element()
            .visual_collection
            .add(text_block.clone());
        assert_eq!(text_block.resolved_text_style().font_size, 30.0);

        parent
            .as_ui_element()
            .visual_collection
            .remove(&(text_block.clone() as Rc<dyn UIElementExt>));
        assert_eq!(
            text_block.resolved_text_style().font_size,
            crate::graphics::text_backend()
                .default_text_style()
                .font_size
        );
    }

    #[test]
    fn inheritance_parent_logical_falls_back_to_visual_when_no_logical_parent() {
        let root = VerticalLayout::new();
        let child = native("a", size(10.0, 10.0));
        root.as_ui_element().visual_collection.add(child.clone());
        // `child` has a Visual parent (`root`, via the raw visual collection) but no Logical
        // parent (never added through `UIElementCollection`) — `Logical` must still find `root`
        // by falling back to Visual (指示書 §14).
        assert!(child.parent().is_none());
        let via_logical = child
            .inheritance_parent(InheritanceParentKind::Logical)
            .expect("Logical must fall back to Visual when there is no logical parent");
        assert!(Rc::ptr_eq(
            &via_logical,
            &(root.clone() as Rc<dyn UIElementExt>)
        ));
        let via_visual = child
            .inheritance_parent(InheritanceParentKind::Visual)
            .expect("Visual parent must be reachable directly");
        assert!(Rc::ptr_eq(&via_visual, &(root as Rc<dyn UIElementExt>)));
    }

    #[test]
    fn content_control_inherits_text_style_from_its_base_control() {
        // Regression guard for the `Attr::TextStyle` exemption in `resolve_effective_fields`/
        // `resolve_field_declaring_types` (`elwindui-codegen`'s `codegen.rs`) — without it, a
        // `has_view` component like `ContentControl` would silently lose all seven text-style
        // setters (they'd never even compile-error; the DSL setter would just not exist). This
        // exercises the *runtime* half: `ContentControl::as_text_style_owner()` must resolve
        // through the `#[class]` ancestor-forwarding chain to its embedded `base: Control`, which
        // really implements `TextStyleOwner` — not `ContentControl` itself (see
        // `emit_field_setter_call`'s own doc comment on why `elwindui-codegen` always goes through
        // `as_text_style_owner()` rather than assuming `TextStyleOwner` is implemented directly).
        let content_control = ContentControl::new();
        let owner = content_control
            .as_text_style_owner()
            .expect("ContentControl must resolve a TextStyleOwner through its Control base");
        owner.set_font_size(18.0);
        assert_eq!(
            content_control
                .as_text_style_owner()
                .unwrap()
                .resolved_text_style()
                .font_size,
            18.0
        );

        let inner = TextBlock::new();
        content_control.set_content(inner.clone());
        assert_eq!(inner.resolved_text_style().font_size, 18.0);
    }
}
