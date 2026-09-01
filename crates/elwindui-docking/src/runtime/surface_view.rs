//! Retained visual host for one Dock surface.

use crate::core::layout::GridLength;
use crate::core::ui::{ContentControlExt, Grid, GridExt};
use std::rc::Rc;

/// One retained surface root. Floating windows own another instance of this type; authored
/// DockGroup/DockSplitPanel objects are never registered as surfaces.
#[elwindui::component(inherits ContentControl)]
pub(crate) struct DockSurfaceView {
    #[state(default = crate::core::ui::Grid::new())]
    surface_content_root: Rc<Grid>,
    template: template_view!(|_this: Self| { ContentPresenter {} }),
}

#[elwindui::component]
impl DockSurfaceView {}

impl DockSurfaceView {
    pub(crate) fn empty_surface() -> Rc<Self> {
        let surface = Self::new();
        let root = surface.content_root();
        root.set_rows(vec![GridLength::Star(1.0)]);
        root.set_columns(vec![GridLength::Star(1.0)]);
        surface.set_content(root);
        surface
    }

    pub(crate) fn content_root(&self) -> Rc<Grid> {
        self.surface_content_root()
    }
}
