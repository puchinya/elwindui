//! Abstract base for backend-neutral, self-drawn icon elements.

use super::*;

/// Base class for UI elements that display an icon.
///
/// `IconElement` is intentionally abstract: use [`IconSourceElement`] to place a shareable
/// [`IconSource`] value in the Visual tree. Its optional `foreground` is inherited through the
/// existing Visual-tree foreground cascade by derived icon elements.
///
/// ```compile_fail
/// use elwindui_core::ui::IconElement;
///
/// // Abstract class bases have no public constructor.
/// let _ = IconElement::new();
/// ```
#[elwindui_macros::class(inherits = crate::ui::UIElement, abstract_class)]
#[prop(semantic_brush, foreground: Option<crate::graphics::Brush>)]
pub struct IconElement {
    /// Local monochrome foreground; `None` delegates color selection to the Visual ancestor.
    pub foreground: RefCell<Option<Brush>>,
}

#[elwindui_macros::class]
impl IconElement {
    /// Returns the locally assigned foreground, excluding inherited values.
    fn foreground(&self) -> Option<Brush> {
        self.foreground.borrow().clone()
    }

    /// Sets the local brush used to paint monochrome system icons.
    fn set_foreground(&self, foreground: Option<Brush>) {
        *self.foreground.borrow_mut() = foreground;
        self.invalidate();
    }

    /// Clears the local brush so a derived icon element inherits foreground from its Visual tree.
    fn clear_foreground(&self) {
        self.set_foreground(None);
    }

    fn construct() -> Self {
        Self {
            base: UIElement::construct(),
            foreground: RefCell::new(None),
        }
    }
}
