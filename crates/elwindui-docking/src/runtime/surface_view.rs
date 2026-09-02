//! Retained visual host for one Dock surface.

use crate::DockItemId;
use crate::DockingControl;
use crate::core::graphics::IconSource;
use crate::core::layout::GridLength;
use crate::core::theme::BrushStyle;
use crate::core::ui::{ContentControlExt, Grid, GridExt, LayoutExt, UIElementExt};
use crate::model::RootKind;
use crate::runtime::auto_hide::AutoHideOverlay;
use crate::runtime::overlay::DropPreview;
use crate::runtime::themed_brush;
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
        root.set_background(themed_brush(BrushStyle::Background));
        surface.set_content(root);
        surface
    }

    pub(crate) fn content_root(&self) -> Rc<Grid> {
        self.surface_content_root()
    }
}

/// All retained chrome and transient visuals belonging to one discoverable Dock surface.
pub(crate) struct SurfaceRuntime {
    pub(crate) root: RootKind,
    pub(crate) surface: Rc<DockSurfaceView>,
    pub(crate) auto_hide: AutoHideOverlay,
    pub(crate) preview: DropPreview,
}

impl SurfaceRuntime {
    pub(crate) fn new(
        root: RootKind,
        surface: Rc<DockSurfaceView>,
        owner: &std::rc::Weak<DockingControl>,
    ) -> Self {
        let auto_hide = AutoHideOverlay::new();
        auto_hide.bind_pin_handler(owner, root.clone());
        let preview = DropPreview::new();
        let runtime = Self {
            root,
            surface,
            auto_hide,
            preview,
        };
        runtime.reset_visual_children();
        runtime
    }

    pub(crate) fn set_root(&mut self, root: RootKind) {
        self.root = root.clone();
        self.auto_hide.set_root(root);
    }

    pub(crate) fn reset_visual_children(&self) {
        let root = self.surface.content_root();
        root.children().clear();
        root.children().add(self.auto_hide.visual());
        root.children().add(self.preview.visual());
    }

    pub(crate) fn add_main_child(&self, child: Rc<dyn UIElementExt>) {
        self.surface.content_root().children().insert(0, child);
    }

    pub(crate) fn render_strips(
        &self,
        titles: impl Iterator<Item = (usize, DockItemId, String, Option<IconSource>)>,
        owner: &std::rc::Weak<DockingControl>,
    ) {
        self.auto_hide
            .render_strips(titles, owner, self.root.clone());
    }
}
