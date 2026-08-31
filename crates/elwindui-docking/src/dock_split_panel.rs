use crate::Orientation;
use crate::core::ui::UIElementExt;
use std::rc::Rc;

/// An authored split container. The runtime owns the mutable pane weights.
#[elwindui::component(inherits Control)]
#[content(children)]
pub struct DockSplitPanel {
    #[prop(default = Orientation::Horizontal)]
    orientation: Orientation,
    #[prop(default = 1.0)]
    weight: f32,
    #[prop(default = Vec::new())]
    children: Vec<Rc<dyn UIElementExt>>,
    template: template_view!(|_this: Self| { Grid {} }),
}

#[elwindui::component]
impl DockSplitPanel {}

impl DockSplitPanel {
    /// Creates an authored split panel in the Created lifecycle state.
    pub fn new_panel() -> Rc<Self> {
        Self::new()
    }

    /// Returns the authored split orientation.
    pub fn orientation_value(&self) -> Orientation {
        self.orientation()
    }

    /// Returns the authored default weight of this panel.
    pub fn weight_value(&self) -> f32 {
        self.weight()
    }

    pub(crate) fn authored_children(&self) -> Vec<Rc<dyn UIElementExt>> {
        #[cfg(not(rust_analyzer))]
        {
            self.children().to_vec()
        }
        #[cfg(rust_analyzer)]
        {
            self.children()
        }
    }
}
