use crate::{DockGroupId, DockItem};
use elwindui_custom_controls::TabStripPosition;
use std::rc::Rc;

/// An authored tab group. Its identity is stable across runtime moves and snapshots.
#[elwindui::component(inherits Control)]
#[content(children)]
pub struct DockGroup {
    #[prop(default = crate::DockGroupId::new(String::new()))]
    id: crate::DockGroupId,
    #[prop(default = 1.0)]
    weight: f32,
    #[prop(default = TabStripPosition::Top)]
    tab_strip_position: TabStripPosition,
    #[prop(default = Vec::new())]
    children: Vec<Rc<DockItem>>,
    template: template_view!(|_this: Self| { Grid {} }),
}

#[elwindui::component]
impl DockGroup {}

impl DockGroup {
    /// Creates an authored tab group in the Created lifecycle state.
    pub fn new_group() -> Rc<Self> {
        Self::new()
    }

    /// Returns this group's authored identity.
    pub fn id_value(&self) -> DockGroupId {
        self.id()
    }

    /// Returns this group's authored default weight.
    pub fn weight_value(&self) -> f32 {
        self.weight()
    }

    /// Returns this group's authored tab-strip position.
    pub fn tab_strip_position_value(&self) -> TabStripPosition {
        self.tab_strip_position()
    }

    pub(crate) fn authored_children(&self) -> Vec<Rc<DockItem>> {
        self.children()
    }
}
