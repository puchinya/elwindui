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
    #[prop(default = false)]
    compact_tabs: bool,
    #[prop(default = false)]
    show_when_empty: bool,
    #[prop(default = Vec::new())]
    children: Vec<Rc<DockItem>>,
    #[state(default = None)]
    registration_callback: Option<Rc<dyn Fn()>>,
    template: template_view!(|this: Self| {
        on_update(children, id, weight, tab_strip_position, compact_tabs, show_when_empty) {
            this.notify_registration_changed();
        }
        Grid {}
    }),
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

    /// Returns whether this group uses compact tab sizing.
    pub fn compact_tabs_value(&self) -> bool {
        self.compact_tabs()
    }

    /// Returns whether this authored group remains visible when empty.
    pub fn show_when_empty_value(&self) -> bool {
        self.show_when_empty()
    }

    pub(crate) fn authored_children(&self) -> Vec<Rc<DockItem>> {
        self.children()
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
