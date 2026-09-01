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
    #[state(default = None)]
    registration_callback: Option<Rc<dyn Fn()>>,
    template: template_view!(|this: Self| {
        on_update(children, orientation, weight) {
            this.notify_registration_changed();
        }
        Grid {}
    }),
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

    pub(crate) fn bind_registration_callback(&self, callback: Option<Rc<dyn Fn()>>) {
        self.set_registration_callback(callback);
    }

    fn notify_registration_changed(&self) {
        if let Some(callback) = self.registration_callback() {
            callback();
        }
    }
}
