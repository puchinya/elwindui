//! Retained visual host for one Dock surface.

use crate::DockItemId;
use crate::DockingControl;
use crate::core::graphics::IconSource;
use crate::core::layout::GridLength;
use crate::core::theme::BrushStyle;
use crate::core::ui::{ContentControlExt, Grid, GridExt, LayoutExt, UIElementExt};
use crate::model::RootKind;
use crate::runtime::auto_hide::{AutoHideExtentCache, AutoHideOverlay};
use crate::runtime::overlay::{DockTargetOverlay, DropPreview, InsertionMarker};
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
    /// Hosts the main root in its center cell; edge tracks reserve the visible auto-hide strips.
    main_host: Rc<Grid>,
    pub(crate) auto_hide: AutoHideOverlay,
    pub(crate) preview: DropPreview,
    pub(crate) targets: DockTargetOverlay,
    pub(crate) insertion_marker: InsertionMarker,
}

impl SurfaceRuntime {
    pub(crate) fn new(
        root: RootKind,
        surface: Rc<DockSurfaceView>,
        owner: &std::rc::Weak<DockingControl>,
        extents: AutoHideExtentCache,
    ) -> Self {
        let auto_hide = AutoHideOverlay::with_extent_cache(extents);
        auto_hide.bind_handlers(owner, root.clone());
        let preview = DropPreview::new();
        let targets = DockTargetOverlay::new();
        let insertion_marker = InsertionMarker::new();
        let main_host = Grid::new();
        let runtime = Self {
            root,
            surface,
            main_host,
            auto_hide,
            preview,
            targets,
            insertion_marker,
        };
        runtime
            .targets
            .set_root_targets_hidden(runtime.root != RootKind::Main);
        runtime.reset_visual_children();
        runtime
            .auto_hide
            .bind_light_dismiss(&runtime.surface.content_root(), owner);
        runtime
    }

    pub(crate) fn set_root(&mut self, root: RootKind) {
        self.targets.set_root_targets_hidden(root != RootKind::Main);
        self.root = root.clone();
        self.auto_hide.set_root(root);
    }

    pub(crate) fn reset_visual_children(&self) {
        let root = self.surface.content_root();
        root.children().clear();
        self.main_host.children().clear();
        self.update_strip_reservation();
        root.children().add(self.main_host.clone());
        root.children().add(self.auto_hide.visual());
        root.children().add(self.preview.visual());
        root.children().add(self.targets.visual());
        root.children().add(self.insertion_marker.visual());
    }

    pub(crate) fn add_main_child(&self, child: Rc<dyn UIElementExt>) {
        child
            .as_ui_element()
            .set_attached_if_changed("Grid", "row", 1i32);
        child
            .as_ui_element()
            .set_attached_if_changed("Grid", "column", 1i32);
        self.main_host.children().add(child);
    }

    /// Shrinks the main root by the visible strip extents, matching the strips' own Auto tracks.
    fn update_strip_reservation(&self) {
        let [left, top, right, bottom] = self.auto_hide.strip_extents();
        // Grid track setters ignore unchanged values, so repeated renders do not invalidate.
        self.main_host.set_rows(vec![
            GridLength::Fixed(top),
            GridLength::Star(1.0),
            GridLength::Fixed(bottom),
        ]);
        self.main_host.set_columns(vec![
            GridLength::Fixed(left),
            GridLength::Star(1.0),
            GridLength::Fixed(right),
        ]);
    }

    pub(crate) fn refresh_theme(&self) {
        self.surface
            .content_root()
            .set_background(themed_brush(BrushStyle::Background));
        self.auto_hide.refresh_theme();
        self.preview.refresh_theme();
        self.targets.refresh_theme();
        self.insertion_marker.refresh_theme();
    }

    pub(crate) fn render_strips(
        &self,
        titles: impl Iterator<Item = (usize, DockItemId, String, Option<IconSource>)>,
        owner: &std::rc::Weak<DockingControl>,
    ) {
        self.auto_hide
            .render_strips(titles, owner, self.root.clone());
        self.update_strip_reservation();
    }
}
