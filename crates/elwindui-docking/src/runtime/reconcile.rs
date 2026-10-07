//! Reconciliation boundary between the value model and stable runtime item wrappers.

use crate::core::base::{Point, Rect, Size};
use crate::core::graphics::{Brush, Color, FontWeight, ImageSource};
use crate::core::input::PointerEventArgs;
use crate::core::layout::{GridLength, GridTrackConstraint, VerticalAlignment, Visibility};
use crate::core::theme::BrushStyle;
use crate::core::ui::{
    ControlExt, Grid, GridExt, LayoutExt, TextBlock, TextBlockExt, TextStyleOwner, UIElementExt,
};
#[cfg(all(not(test), any(target_os = "macos", target_os = "windows")))]
use crate::core::ui::{MenuExt, MenuItemExt};
use crate::model::{RootKind, SplitAddress};
use crate::snapshot::{SnapshotAutoHideEntry, SnapshotGroupKey, SnapshotNode, SnapshotOrientation};
use crate::{
    DockGroup, DockGroupId, DockItem, DockItemId, DockLayoutError, DockLayoutModel, DockSide,
    DockSplitPanel,
};
use elwindui_custom_controls::{
    CloseButtonPresentation, CustomGridSplitter, CustomGridSplitterExt, CustomTabView,
    CustomTabViewExt, CustomTabViewItem, CustomTabViewItemExt, GridResizeBehavior,
    GridResizeDirection, GridSplitterResizeCompletedEventArgs, GridSplitterResizeStartedEventArgs,
    TabItemPointerEvent, TabStripPosition,
};
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use std::rc::Weak;

use super::drag::{DragSession, DragSourceGeometry, ResolvedDockTarget};
#[cfg(test)]
use super::floating_window::FloatingHostFactory;
use super::floating_window::{FloatingHostId, FloatingHostRegistry, PreparedFloatingHostSync};
use super::group_view::replace_group_items;
use super::metrics::{
    CONTENT_HEADER_HEIGHT, SPLITTER_HIT_SIZE, TAB_INSERTION_MARKER_WIDTH, TAB_STRIP_HEIGHT,
    TITLE_BUTTON_SIZE,
};
use super::overlay::{group_target_rect, root_target_rect};
use super::split_layout::DockSplitView;
use super::split_view::SplitterSession;
use super::surface_registry::SurfaceRegistry;
use super::surface_view::{DockSurfaceView, SurfaceRuntime};
use super::{accent_brush, themed_brush};
use elwindui_custom_controls::{ChromeIcon, chrome_icon};

/// Stable registration-to-presentation map for one authored docking surface.
///
/// The map owns one `CustomTabViewItem` per authored `DockItem`. A layout reconciliation only
/// changes the parent tab view; it never reconstructs the page wrapper. This is the ownership
/// boundary that prevents a selected tab, floating host, and auto-hide overlay from each creating
/// a second logical page owner.
#[derive(Default)]
pub(crate) struct StableItemRegistry {
    items: BTreeMap<DockItemId, Rc<DockItem>>,
    wrappers: BTreeMap<DockItemId, Rc<CustomTabViewItem>>,
    group_positions: BTreeMap<DockGroupId, TabStripPosition>,
    group_compact_tabs: BTreeMap<DockGroupId, bool>,
    group_show_when_empty: BTreeMap<DockGroupId, bool>,
    group_extents: BTreeMap<DockGroupId, crate::DockSize>,
}

/// Inherits a split panel's cross-axis extent into a child that does not set one.
fn inherit_cross_extent(own: crate::DockSize, parent: crate::DockSize) -> crate::DockSize {
    crate::DockSize {
        width: own.width.or(parent.width),
        height: own.height.or(parent.height),
        ..own
    }
}

impl StableItemRegistry {
    pub(crate) fn from_authored(root: &dyn UIElementExt) -> Result<Self, DockLayoutError> {
        let mut registry = Self::default();
        registry.refresh_authored(root)?;
        Ok(registry)
    }

    /// Reconciles registration changes while retaining wrappers for IDs that still exist.
    pub(crate) fn refresh_authored(
        &mut self,
        root: &dyn UIElementExt,
    ) -> Result<(), DockLayoutError> {
        let mut items = BTreeMap::new();
        let mut groups = BTreeMap::new();
        let mut compact_tabs = BTreeMap::new();
        let mut show_when_empty = BTreeMap::new();
        let mut extents = BTreeMap::new();
        collect_authored(
            root,
            &mut items,
            &mut groups,
            &mut compact_tabs,
            &mut show_when_empty,
            &mut extents,
            crate::DockSize::default(),
        )?;

        let removed = self
            .items
            .keys()
            .filter(|id| !items.contains_key(*id))
            .cloned()
            .collect::<Vec<_>>();
        for id in removed {
            self.items.remove(&id);
            self.wrappers.remove(&id);
        }

        for (id, item) in items {
            let wrapper = self
                .wrappers
                .entry(id.clone())
                .or_insert_with(|| item.to_tab_item());
            // Metadata may be reactive even when the identity is not. Updating the existing
            // wrapper is safe because its ContentControl content remains the same logical slot.
            wrapper.set_header(item.title_value());
            wrapper.set_icon(item.icon_value());
            wrapper.set_closable(item.can_close_value());
            // Page replacement is deliberately not a registration-time operation. Keeping the
            // existing wrapper content untouched preserves page identity and avoids an implicit
            // unmount/remount during metadata-only authored refreshes.
            self.items.insert(id, item);
        }
        self.group_positions = groups;
        self.group_compact_tabs = compact_tabs;
        self.group_show_when_empty = show_when_empty;
        self.group_extents = extents;
        Ok(())
    }

    pub(crate) fn group_extent(&self, id: &DockGroupId) -> crate::DockSize {
        self.group_extents.get(id).copied().unwrap_or_default()
    }

    pub(crate) fn wrapper(&self, id: &DockItemId) -> Option<Rc<CustomTabViewItem>> {
        self.wrappers.get(id).cloned()
    }

    pub(crate) fn group_position(&self, id: &DockGroupId) -> TabStripPosition {
        self.group_positions
            .get(id)
            .copied()
            .unwrap_or(TabStripPosition::Top)
    }

    pub(crate) fn group_compact_tabs(&self, id: &DockGroupId) -> bool {
        self.group_compact_tabs.get(id).copied().unwrap_or(false)
    }

    pub(crate) fn group_show_when_empty(&self, id: &DockGroupId) -> bool {
        self.group_show_when_empty.get(id).copied().unwrap_or(false)
    }
}

fn collect_authored(
    node: &dyn UIElementExt,
    items: &mut BTreeMap<DockItemId, Rc<DockItem>>,
    groups: &mut BTreeMap<DockGroupId, TabStripPosition>,
    compact_tabs: &mut BTreeMap<DockGroupId, bool>,
    show_when_empty: &mut BTreeMap<DockGroupId, bool>,
    extents: &mut BTreeMap<DockGroupId, crate::DockSize>,
    inherited: crate::DockSize,
) -> Result<(), DockLayoutError> {
    if let Some(group) = node.as_any().downcast_ref::<DockGroup>() {
        let id = group.id_value();
        if id.as_ref().is_empty()
            || groups
                .insert(id.clone(), group.tab_strip_position_value())
                .is_some()
        {
            return Err(DockLayoutError::InvalidSnapshot {
                reason: format!("duplicate or empty authored DockGroupId: {id}"),
            });
        }
        compact_tabs.insert(id.clone(), group.compact_tabs_value());
        show_when_empty.insert(id.clone(), group.show_when_empty_value());
        extents.insert(
            id.clone(),
            inherit_cross_extent(group.dock_size_value().sanitized(), inherited),
        );
        for item in group.authored_children() {
            let item_id = item.id_value();
            if item_id.as_ref().is_empty() || items.insert(item_id.clone(), item).is_some() {
                return Err(DockLayoutError::InvalidSnapshot {
                    reason: format!("duplicate or empty authored DockItemId: {item_id}"),
                });
            }
        }
        return Ok(());
    }
    if let Some(panel) = node.as_any().downcast_ref::<DockSplitPanel>() {
        // Children of a split span its cross axis, so they inherit only that cross-axis size:
        // a horizontal panel's height, or a vertical panel's width.
        let panel_extent = inherit_cross_extent(panel.dock_size_value().sanitized(), inherited);
        let inherited = match panel.orientation_value() {
            crate::Orientation::Horizontal => crate::DockSize {
                height: panel_extent.height,
                ..crate::DockSize::default()
            },
            crate::Orientation::Vertical => crate::DockSize {
                width: panel_extent.width,
                ..crate::DockSize::default()
            },
        };
        for child in panel.authored_children() {
            collect_authored(
                child.as_ref(),
                items,
                groups,
                compact_tabs,
                show_when_empty,
                extents,
                inherited,
            )?;
        }
        return Ok(());
    }
    Err(DockLayoutError::InvalidSnapshot {
        reason: "authored docking root contains an unsupported element".to_owned(),
    })
}

/// The private realization tree kept by `DockingControl`.
pub(crate) enum RuntimeNode {
    Group {
        host: Rc<Grid>,
    },
    Split {
        children: Vec<RuntimeNode>,
        grid: Rc<Grid>,
        splitters: Vec<Rc<CustomGridSplitter>>,
        view: Rc<DockSplitView>,
        orientation: SnapshotOrientation,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RuntimePresentationOwner {
    Group(SnapshotGroupKey),
    AutoHide {
        root: RootKind,
    },
    FloatingGroup {
        root: usize,
        group: SnapshotGroupKey,
    },
    None,
}

struct FloatingRuntime {
    node: RuntimeNode,
    identity: Vec<SnapshotGroupKey>,
    surface: SurfaceRuntime,
}

impl RuntimeNode {
    fn element(&self) -> Rc<dyn UIElementExt> {
        match self {
            Self::Group { host, .. } => host.clone(),
            Self::Split { view, .. } => view.clone(),
        }
    }
}

#[derive(Clone)]
struct GroupRuntimeHost {
    container: Rc<Grid>,
    /// Header row above the tab view. The active frame is a sibling of this body inside the
    /// container, so it can enclose the header like WinUI.Dock's active group border.
    body: Rc<Grid>,
    content_header: Rc<Grid>,
    active_chrome: Rc<GroupChromeOverlay>,
    title: Rc<TextBlock>,
    pin_button: Rc<Grid>,
    close_button: Rc<Grid>,
    header_drag_handlers_bound: Rc<Cell<bool>>,
}

impl GroupRuntimeHost {
    fn refresh_theme(&self) {
        self.content_header
            .set_background(themed_brush(BrushStyle::Secondary));
        self.title
            .set_foreground(themed_brush(BrushStyle::Foreground));
        self.active_chrome.refresh_theme();

        self.pin_button.children().clear();
        self.pin_button
            .children()
            .add(RuntimeRealization::private_icon(true));
        self.close_button.children().clear();
        self.close_button
            .children()
            .add(RuntimeRealization::private_icon(false));
    }
}

fn active_frame_image(
    size: Size,
    gap: Option<(f32, f32)>,
    position: TabStripPosition,
    brush: Brush,
) -> Option<ImageSource> {
    use crate::core::base::AffineTransform;
    use crate::core::graphics::{
        PathBuilder, StrokeStyle, VectorGroup, VectorImageBuilder, VectorNode, VectorPaint,
        VectorPaintOrder, VectorPathNode, VectorShapeRendering, VectorStroke,
    };
    if !size.width.is_finite()
        || !size.height.is_finite()
        || size.width <= 1.0
        || size.height <= 1.0
    {
        return None;
    }
    let radius = 4.0_f32
        .min((size.width - 1.0) * 0.5)
        .min((size.height - 1.0) * 0.5);
    let left = 0.5;
    let right = size.width - 0.5;
    let bottom = size.height - 0.5;
    let gap = gap.and_then(|(start, end)| {
        let start = start.clamp(left + radius, right - radius);
        let end = end.clamp(left + radius, right - radius);
        (start.is_finite() && end.is_finite() && end > start).then_some((start, end))
    });
    let map = |x, y| Point {
        x,
        y: if position == TabStripPosition::Bottom {
            size.height - y
        } else {
            y
        },
    };
    let mut path = PathBuilder::new();
    path.move_to(map(gap.map_or(left + radius, |(_, end)| end), 0.5))
        .line_to(map(right - radius, 0.5))
        .quad_to(map(right, 0.5), map(right, 0.5 + radius))
        .line_to(map(right, bottom - radius))
        .quad_to(map(right, bottom), map(right - radius, bottom))
        .line_to(map(left + radius, bottom))
        .quad_to(map(left, bottom), map(left, bottom - radius))
        .line_to(map(left, 0.5 + radius))
        .quad_to(map(left, 0.5), map(left + radius, 0.5));
    if let Some((start, _)) = gap {
        path.line_to(map(start, 0.5));
    } else {
        path.close();
    }
    let node = VectorNode::Path(VectorPathNode {
        path: path.build().ok()?,
        transform: AffineTransform::IDENTITY,
        fill: None,
        stroke: Some(VectorStroke {
            paint: VectorPaint::Brush(brush),
            opacity: 1.0,
            style: StrokeStyle {
                width: 1.0,
                ..StrokeStyle::default()
            },
        }),
        paint_order: VectorPaintOrder::default(),
        rendering: VectorShapeRendering::GeometricPrecision,
        visibility: true,
    });
    Some(ImageSource::Vector(
        VectorImageBuilder::new(
            size,
            Rect {
                x: 0.0,
                y: 0.0,
                width: size.width,
                height: size.height,
            },
        )
        .ok()?
        .root(VectorGroup {
            children: std::sync::Arc::from([node]),
            ..VectorGroup::default()
        })
        .finish()
        .ok()?,
    ))
}

#[cfg(test)]
mod chrome_tests {
    use super::*;
    use crate::core::graphics::{PathCommand, VectorNode};

    #[test]
    fn group_chrome_arrange_paints_the_frame_without_invalidating_measure() {
        let overlay = GroupChromeOverlay::new();
        let element: Rc<dyn UIElementExt> = overlay.clone();
        let size = Size {
            width: 300.0,
            height: 200.0,
        };
        overlay.set_presentation(true, None, TabStripPosition::Top, 32.0);
        crate::core::ui::layout_root(&element, size);
        assert!(overlay.frame_source().borrow().is_some());
        assert_eq!(
            overlay.frame_rect(),
            Some(Rect {
                x: 0.0,
                y: 32.0,
                width: 300.0,
                height: 168.0,
            })
        );

        // Arrange draws the contour from final geometry only: repeated, resized and
        // active-toggled Arrange passes leave the retained measurement valid.
        let arrange = |width: f32| {
            overlay.arrange(Rect {
                x: 0.0,
                y: 0.0,
                width,
                height: size.height,
            })
        };
        for width in [300.0, 240.0, 300.0] {
            arrange(width);
            assert!(overlay.measured_size().is_some());
        }
        overlay.set_presentation(false, None, TabStripPosition::Top, 32.0);
        arrange(300.0);
        assert_eq!(overlay.frame_rect(), None);
        overlay.set_presentation(true, None, TabStripPosition::Top, 32.0);
        arrange(300.0);
        assert!(overlay.measured_size().is_some());

        // A tab-position change is presentation input; the frame follows on the next Arrange.
        overlay.set_presentation(true, None, TabStripPosition::Bottom, 32.0);
        arrange(300.0);
        assert!(overlay.measured_size().is_some());
        assert_eq!(
            overlay.frame_rect(),
            Some(Rect {
                x: 0.0,
                y: 0.0,
                width: 300.0,
                height: 168.0,
            })
        );
    }
    #[test]
    fn active_frame_contour_opens_at_the_selected_tab_on_either_edge() {
        let size = Size {
            width: 300.0,
            height: 180.0,
        };
        for (position, y) in [
            (TabStripPosition::Top, 0.5),
            (TabStripPosition::Bottom, 179.5),
        ] {
            let Some(ImageSource::Vector(image)) =
                active_frame_image(size, Some((20.0, 120.0)), position, accent_brush())
            else {
                panic!("valid frame should produce vector geometry")
            };
            let VectorNode::Path(node) = &image.root().children[0] else {
                panic!("frame should be a path")
            };
            let commands = node.path.commands();
            assert_eq!(
                commands.first(),
                Some(&PathCommand::MoveTo(Point { x: 120.0, y }))
            );
            assert_eq!(
                commands.last(),
                Some(&PathCommand::LineTo(Point { x: 20.0, y }))
            );
            assert!(
                !commands
                    .iter()
                    .any(|command| matches!(command, PathCommand::Close))
            );
            assert!(node.fill.is_none());
            assert_eq!(node.stroke.as_ref().unwrap().style.width, 1.0);
        }
        let Some(ImageSource::Vector(image)) =
            active_frame_image(size, None, TabStripPosition::Bottom, accent_brush())
        else {
            panic!("hidden strip should still produce a frame")
        };
        let VectorNode::Path(node) = &image.root().children[0] else {
            panic!("frame should be a path")
        };
        assert_eq!(node.path.commands().last(), Some(&PathCommand::Close));
        assert!(
            active_frame_image(
                Size {
                    width: 0.0,
                    height: 180.0
                },
                None,
                TabStripPosition::Top,
                accent_brush()
            )
            .is_none()
        );
    }
}

/// Docking-private active frame and document marker. The marker is placed from the retained tab's
/// arranged bounds, keeping generic CustomTabView free of Docking-specific public state.
///
/// The frame is painted by this element itself rather than by an `Image` child: its contour
/// depends on arranged geometry, and an `Image` source change would invalidate Measure from Arrange.
#[elwindui::component(inherits Control)]
struct GroupChromeOverlay {
    #[prop(default = false)]
    is_active: bool,
    #[prop(default = None)]
    active_tab: Option<Rc<CustomTabViewItem>>,
    #[prop(default = TabStripPosition::Top)]
    tab_position: TabStripPosition,
    #[prop(default = 32.0)]
    strip_height: f32,
    #[state(default = None)]
    frame_key: Option<(Size, Option<(f32, f32)>, TabStripPosition, Brush)>,
    #[state(default = Rc::new(std::cell::RefCell::new(None)))]
    frame_source: Rc<std::cell::RefCell<Option<ImageSource>>>,
    template: template_view!(|this: Self| {
        Grid {
            hit_test_visible: false,
        }
    }),
}

#[elwindui::component]
impl GroupChromeOverlay {
    #[overrides]
    fn measure_override(&self, available: Size) -> Size {
        // The overlay fills whatever its host arranges and asks for no space itself.
        if let Some(root) = self.__template_root() {
            root.measure(available);
        }
        Size {
            width: 0.0,
            height: 0.0,
        }
    }

    /// Builds the open contour from final geometry. Only paint state changes here; nothing that
    /// Measure reads is written.
    #[overrides]
    fn arrange_override(&self, final_size: Size) -> Size {
        if let Some(root) = self.__template_root() {
            root.arrange(Rect {
                x: 0.0,
                y: 0.0,
                width: final_size.width.max(0.0),
                height: final_size.height.max(0.0),
            });
        }
        let strip_height = self.strip_height().max(0.0);
        // Do not paint a border across the selected header's connection to the page.
        // An open contour works even when the page background is transparent.
        let size = Size {
            width: final_size.width.max(0.0),
            height: (final_size.height - strip_height).max(0.0),
        };
        let gap = self
            .active_tab()
            .filter(|_| strip_height > 0.0)
            .and_then(|tab| {
                let node: Rc<dyn UIElementExt> = tab;
                let bounds = SurfaceRegistry::bounds_in_host_root(&node)?;
                let origin = self.origin_in_host_root()?;
                let left = bounds.x - origin.x;
                Some((left, left + bounds.width))
            });
        let key = (size, gap, self.tab_position(), accent_brush());
        if self.is_active() && self.frame_key().as_ref() != Some(&key) {
            *self.frame_source().borrow_mut() =
                active_frame_image(size, gap, self.tab_position(), key.3.clone());
            self.set_frame_key(Some(key));
            self.invalidate_render();
        }
        final_size
    }

    #[overrides]
    fn render(&self, context: &mut crate::core::graphics::RenderContext<'_>) {
        let Some(rect) = self.frame_rect() else {
            return;
        };
        if let Some(ImageSource::Vector(image)) = self.frame_source().borrow().as_ref() {
            context.draw_vector_image(
                image,
                rect,
                None,
                crate::core::graphics::VectorImageDrawOptions {
                    fit: crate::core::graphics::ImageFit::Contain,
                    ..Default::default()
                },
            );
        }
    }
}

impl GroupChromeOverlay {
    /// Where the active frame paints: the page area beside the strip, or nothing while inactive.
    fn frame_rect(&self) -> Option<Rect> {
        if !self.is_active() {
            return None;
        }
        let width = self.arranged_width()?;
        let height = self.arranged_height()?;
        let strip_height = self.strip_height().max(0.0);
        let y = if self.tab_position() == TabStripPosition::Top {
            strip_height
        } else {
            0.0
        };
        Some(Rect {
            x: 0.0,
            y,
            width: width.max(0.0),
            height: (height - strip_height).max(0.0),
        })
    }

    /// This overlay's top-left in the hosted root space, from its own arranged offset and its
    /// parent's bounds (the overlay is arranged before its contour is built).
    fn origin_in_host_root(&self) -> Option<Point> {
        let offset = self.arranged_offset()?;
        let parent = match self.visual_parent() {
            Some(parent) => SurfaceRegistry::bounds_in_host_root(&parent)?,
            None => Rect {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 0.0,
            },
        };
        Some(Point {
            x: parent.x + offset.x,
            y: parent.y + offset.y,
        })
    }

    fn set_presentation(
        &self,
        is_active: bool,
        active_tab: Option<Rc<CustomTabViewItem>>,
        tab_position: TabStripPosition,
        strip_height: f32,
    ) {
        let mut changed = false;
        if self.is_active() != is_active {
            self.set_is_active(is_active);
            // An inactive overlay leaves the render tree, so its last frame cannot linger.
            self.set_visibility(if is_active {
                Visibility::Visible
            } else {
                Visibility::Collapsed
            });
            changed = true;
        }
        let same_tab = match (self.active_tab(), active_tab.as_ref()) {
            (Some(current), Some(next)) => Rc::ptr_eq(&current, next),
            (None, None) => true,
            _ => false,
        };
        if !same_tab {
            if let Some(previous) = self.active_tab() {
                previous.set_active_document_marker_visible(false);
            }
            self.set_active_tab(active_tab.clone());
            changed = true;
        }
        if let Some(tab) = active_tab.as_ref() {
            tab.set_active_document_marker_visible(is_active);
        }
        if self.tab_position() != tab_position {
            self.set_tab_position(tab_position);
            changed = true;
        }
        if self.strip_height() != strip_height {
            self.set_strip_height(strip_height);
            changed = true;
        }
        if changed {
            // Arrange rebuilds the contour for the new geometry; the paint is re-recorded by a
            // render invalidation.
            self.invalidate_arrange();
            self.invalidate_render();
        }
    }

    fn refresh_theme(&self) {
        self.set_frame_key(None);
        self.invalidate_arrange();
        self.invalidate_render();
    }
}
struct PlannedGroup {
    view: Rc<CustomTabView>,
    host: GroupRuntimeHost,
    tabs: Vec<Rc<CustomTabViewItem>>,
    items: Vec<DockItemId>,
    selected: Option<DockItemId>,
    tab_position: TabStripPosition,
    compact_tabs: bool,
    title: String,
    visibility: Visibility,
    title_visibility: Visibility,
    pin_visibility: Visibility,
    close_visibility: Visibility,
    is_active: bool,
    active_tab: Option<Rc<CustomTabViewItem>>,
}

struct PlannedSplit {
    grid: Rc<Grid>,
    splitters: Vec<Rc<CustomGridSplitter>>,
    view: Rc<DockSplitView>,
    orientation: SnapshotOrientation,
    weights: Vec<f32>,
}

struct PlannedFloatingRuntime {
    bounds: crate::Rect,
    node: RuntimeNode,
    identity: Vec<SnapshotGroupKey>,
    surface: Rc<DockSurfaceView>,
}

/// A complete candidate realization. Planning only derives facts and constructs unattached
/// candidate controls; `commit_reconcile` is the sole place that changes visual ownership.
struct ReconcilePlan {
    snapshot: crate::snapshot::DockLayoutSnapshot,
    desired_owners: BTreeMap<DockItemId, RuntimePresentationOwner>,
    planned_groups: BTreeMap<SnapshotGroupKey, PlannedGroup>,
    planned_splits: BTreeMap<SplitAddress, PlannedSplit>,
    group_items: BTreeMap<SnapshotGroupKey, Vec<DockItemId>>,
    group_roots: BTreeMap<SnapshotGroupKey, RootKind>,
    group_selected: BTreeMap<SnapshotGroupKey, Option<DockItemId>>,
    auto_hide_roots: BTreeMap<DockItemId, (RootKind, DockSide)>,
    root: Option<RuntimeNode>,
    floating: Vec<PlannedFloatingRuntime>,
    surfaces: SurfaceRegistry,
    main_surface_child: Option<Rc<dyn UIElementExt>>,
    open_auto_hide: Vec<(RootKind, DockSide, DockItemId)>,
    host_sync: PreparedFloatingHostSync,
}

pub struct RuntimeRealization {
    registry: StableItemRegistry,
    groups: BTreeMap<SnapshotGroupKey, Rc<CustomTabView>>,
    group_hosts: BTreeMap<SnapshotGroupKey, GroupRuntimeHost>,
    group_items: BTreeMap<SnapshotGroupKey, Vec<DockItemId>>,
    group_roots: BTreeMap<SnapshotGroupKey, RootKind>,
    group_selected: BTreeMap<SnapshotGroupKey, Option<DockItemId>>,
    auto_hide_roots: BTreeMap<DockItemId, (RootKind, DockSide)>,
    auto_hide_extents: super::auto_hide::AutoHideExtentCache,
    owners: BTreeMap<DockItemId, RuntimePresentationOwner>,
    root: Option<RuntimeNode>,
    floating: Vec<FloatingRuntime>,
    split_views: BTreeMap<SplitAddress, (Rc<Grid>, Vec<Rc<CustomGridSplitter>>, Rc<DockSplitView>)>,
    drag: Option<DragSession>,
    splitter: Option<SplitterSession>,
    floating_hosts: FloatingHostRegistry,
    surfaces: SurfaceRegistry,
    main_surface: SurfaceRuntime,
    surface_root: Rc<Grid>,
    main_surface_child: Option<Rc<dyn UIElementExt>>,
    /// Runtime-only preferred auto-hide side per item: the side of its last root-edge drop or of
    /// the strip it was last unpinned from (`docking_spec.md`). Not persisted in snapshots.
    preferred_sides: BTreeMap<DockItemId, DockSide>,
    /// Runtime-only fixed extents after a splitter drag resized a fixed-size track, keyed by
    /// authored group and axis (`true` = width). Never persisted in snapshots.
    fixed_overrides: BTreeMap<(DockGroupId, bool), f32>,
    /// Runtime-only presentation of generated groups, chosen once by the owner's
    /// `set_on_group_created` hook from the group's first Document. Never persisted in snapshots.
    generated_group_options: std::cell::RefCell<BTreeMap<u64, crate::DockGroupOptions>>,
    /// The dragged Document's tab header, hidden while its drag is active (the reference detaches
    /// the tab at drag start) and restored before the drag commits or cancels.
    drag_hidden_tab: Option<Rc<CustomTabViewItem>>,
    /// Presentation-only selection moved to a neighbour while the selected tab is dragged, with
    /// the index to restore. The model's selection is never changed by this.
    drag_shown_selection: Option<(Rc<CustomTabView>, usize)>,
    owner: Weak<crate::DockingControl>,
    reconciling: Rc<Cell<bool>>,
    native_bounds_syncing: Cell<bool>,
    #[cfg(test)]
    fail_after_reconcile_plan: bool,
    #[cfg(test)]
    full_reconcile_count: usize,
    #[cfg(test)]
    theme_refresh_count: Cell<usize>,
}

/// Source updates use a latest-only queue while a realization is being applied.
#[cfg(test)]
pub(crate) struct LatestOnlyQueue {
    applying: bool,
    pending: Option<DockLayoutModel>,
}

#[cfg(test)]
impl LatestOnlyQueue {
    pub(crate) fn new() -> Self {
        Self {
            applying: false,
            pending: None,
        }
    }

    pub(crate) fn request(
        &mut self,
        current: &DockLayoutModel,
        next: DockLayoutModel,
    ) -> Option<DockLayoutModel> {
        if !self.applying && current == &next {
            return None;
        }
        if self.applying {
            self.pending = Some(next);
            None
        } else {
            self.applying = true;
            Some(next)
        }
    }

    pub(crate) fn finish(&mut self) -> Option<DockLayoutModel> {
        if let Some(next) = self.pending.take() {
            Some(next)
        } else {
            self.applying = false;
            None
        }
    }
}

impl RuntimeRealization {
    pub(crate) fn from_authored(
        root: &dyn UIElementExt,
        surface: Rc<DockSurfaceView>,
        owner: Weak<crate::DockingControl>,
    ) -> Result<Self, DockLayoutError> {
        let auto_hide_extents = super::auto_hide::AutoHideExtentCache::default();
        let main_surface = SurfaceRuntime::new(
            RootKind::Main,
            surface.clone(),
            &owner,
            auto_hide_extents.clone(),
        );
        let surface_root = surface.content_root();
        let mut surfaces = SurfaceRegistry::default();
        let surface_node: Rc<dyn UIElementExt> = surface.clone();
        surfaces.register(RootKind::Main, &surface_node);
        Ok(Self {
            registry: StableItemRegistry::from_authored(root)?,
            groups: BTreeMap::new(),
            group_hosts: BTreeMap::new(),
            group_items: BTreeMap::new(),
            group_roots: BTreeMap::new(),
            group_selected: BTreeMap::new(),
            auto_hide_roots: BTreeMap::new(),
            auto_hide_extents,
            owners: BTreeMap::new(),
            root: None,
            floating: Vec::new(),
            split_views: BTreeMap::new(),
            drag: None,
            splitter: None,
            floating_hosts: FloatingHostRegistry::default(),
            surfaces,
            main_surface,
            surface_root,
            main_surface_child: None,
            preferred_sides: BTreeMap::new(),
            fixed_overrides: BTreeMap::new(),
            generated_group_options: std::cell::RefCell::new(BTreeMap::new()),
            drag_hidden_tab: None,
            drag_shown_selection: None,
            owner,
            reconciling: Rc::new(Cell::new(false)),
            native_bounds_syncing: Cell::new(false),
            #[cfg(test)]
            fail_after_reconcile_plan: false,
            #[cfg(test)]
            full_reconcile_count: 0,
            #[cfg(test)]
            theme_refresh_count: Cell::new(0),
        })
    }

    pub(crate) fn refresh_authored(
        &mut self,
        root: &dyn UIElementExt,
    ) -> Result<(), DockLayoutError> {
        self.registry.refresh_authored(root)?;
        // An authored registration change can invalidate the item or splitter captured by a
        // native gesture. Cancel both transient sessions before the next model reconciliation;
        // this is safer than allowing a stale wrapper to commit into the new registry.
        self.drag = None;
        self.restore_drag_hidden_tab();
        if let Some(mut splitter) = self.splitter.take() {
            splitter.cancel();
        }
        self.clear_previews();
        Ok(())
    }

    pub(crate) fn cancel_transient(&mut self) {
        self.drag = None;
        self.restore_drag_hidden_tab();
        if let Some(mut splitter) = self.splitter.take() {
            splitter.cancel();
        }
        self.clear_previews();
    }

    /// Test-only convenience that exercises the complete staged protocol. Production callers
    /// must let `DockingControl` finalize the prepared native-host sync after owner publication.
    #[cfg(test)]
    pub(crate) fn reconcile_for_test(
        &mut self,
        model: &DockLayoutModel,
    ) -> Result<(), DockLayoutError> {
        let host_sync = self.apply_staged(model)?;
        self.commit_floating_host_sync(host_sync);
        Ok(())
    }

    pub(crate) fn apply_staged(
        &mut self,
        model: &DockLayoutModel,
    ) -> Result<PreparedFloatingHostSync, DockLayoutError> {
        let plan = self.prepare_reconcile(model)?;
        Ok(self.commit_reconcile(plan))
    }

    pub(crate) fn commit_floating_host_sync(&mut self, host_sync: PreparedFloatingHostSync) {
        self.native_bounds_syncing.set(true);
        self.floating_hosts.commit_sync(host_sync);
        self.native_bounds_syncing.set(false);
        for (index, runtime) in self.floating.iter().enumerate() {
            let title = runtime
                .identity
                .iter()
                .find_map(|group| self.group_selected.get(group).and_then(Clone::clone))
                .and_then(|item| self.registry.items.get(&item))
                .map(|item| item.title_value())
                .unwrap_or_else(|| "Floating".to_owned());
            self.floating_hosts.set_title(index, &title);
        }
    }

    pub(crate) fn native_bounds_syncing(&self) -> bool {
        self.native_bounds_syncing.get()
    }

    pub(crate) fn refresh_theme(&self) {
        #[cfg(test)]
        self.theme_refresh_count
            .set(self.theme_refresh_count.get() + 1);

        self.main_surface.refresh_theme();
        for floating in &self.floating {
            floating.surface.refresh_theme();
        }
        for (group, view) in &self.groups {
            // Theme/template refreshes can recreate the visible tab presentation. Restore the
            // retained model selection before refreshing its chrome so a theme switch cannot
            // briefly or permanently show index zero for a different active item.
            view.set_selected_index(self.selected_group_index(group).unwrap_or(0));
            view.refresh_theme();
        }
        for host in self.group_hosts.values() {
            host.refresh_theme();
        }
    }

    fn prepare_reconcile(
        &mut self,
        model: &DockLayoutModel,
    ) -> Result<ReconcilePlan, DockLayoutError> {
        let snapshot = model.snapshot();
        crate::snapshot::validate_snapshot(&snapshot)?;
        let active_item = model.active_item();
        let desired_owners = desired_owners(&snapshot);
        let floating_count = snapshot.floating_roots.len();
        let mut auto_hide_roots = BTreeMap::new();
        for (side_index, entries) in snapshot.auto_hide.iter().enumerate() {
            let Some(side) = DockSide::ALL.get(side_index).copied() else {
                continue;
            };
            for entry in entries {
                auto_hide_roots.insert(
                    entry.item.clone(),
                    (auto_hide_root(entry, floating_count), side),
                );
            }
        }

        let mut groups = self.groups.clone();
        let mut group_hosts = self.group_hosts.clone();
        let mut planned_groups = BTreeMap::new();
        let mut planned_splits = BTreeMap::new();
        let mut group_items = BTreeMap::new();
        let mut group_roots = BTreeMap::new();
        let mut group_selected = BTreeMap::new();
        let mut used_groups = BTreeSet::new();
        let mut used_splits = BTreeSet::new();

        let root = snapshot
            .main_root
            .as_ref()
            .map(|node| {
                self.plan_node(
                    node,
                    &mut groups,
                    &mut group_hosts,
                    &mut planned_groups,
                    &mut planned_splits,
                    &mut group_items,
                    &mut group_roots,
                    &mut group_selected,
                    &mut used_groups,
                    &mut used_splits,
                    RootKind::Main,
                    &[],
                )
            })
            .transpose()?;

        let mut floating = Vec::with_capacity(snapshot.floating_roots.len());
        for (floating_index, floating_root) in snapshot.floating_roots.iter().enumerate() {
            let root_kind = RootKind::Floating(floating_index);
            let node = self.plan_node(
                &floating_root.root,
                &mut groups,
                &mut group_hosts,
                &mut planned_groups,
                &mut planned_splits,
                &mut group_items,
                &mut group_roots,
                &mut group_selected,
                &mut used_groups,
                &mut used_splits,
                root_kind.clone(),
                &[],
            )?;
            let identity = group_identity(&floating_root.root);
            let surface = self
                .floating
                .iter()
                .find(|runtime| runtime.identity == identity)
                .map(|runtime| runtime.surface.surface.clone())
                .unwrap_or_else(DockSurfaceView::empty_surface);
            floating.push(PlannedFloatingRuntime {
                bounds: floating_root.bounds.into(),
                node,
                identity,
                surface,
            });
        }

        for planned in planned_groups.values_mut() {
            if let Some(index) = active_item
                .as_ref()
                .and_then(|active| planned.items.iter().position(|item| item == active))
            {
                planned.is_active = true;
                planned.active_tab = planned.tabs.get(index).cloned();
            }
        }

        groups.retain(|key, _| used_groups.contains(key));
        self.generated_group_options
            .borrow_mut()
            .retain(|id, _| used_groups.contains(&SnapshotGroupKey::Generated(*id)));
        group_hosts.retain(|key, _| used_groups.contains(key));
        planned_groups.retain(|key, _| used_groups.contains(key));
        planned_splits.retain(|key, _| used_splits.contains(key));

        let mut surfaces = SurfaceRegistry::default();
        for (index, runtime) in floating.iter().enumerate() {
            let surface_node: Rc<dyn UIElementExt> = runtime.surface.clone();
            surfaces.register(RootKind::Floating(index), &surface_node);
        }
        let main_surface_node: Rc<dyn UIElementExt> = self.main_surface.surface.clone();
        surfaces.register(RootKind::Main, &main_surface_node);

        let open_auto_hide = snapshot
            .auto_hide
            .iter()
            .enumerate()
            .flat_map(|(side_index, entries)| {
                let side = DockSide::ALL.get(side_index).copied();
                entries.iter().filter_map(move |entry| {
                    side.filter(|_| entry.open).map(|side| {
                        (
                            auto_hide_root(entry, floating_count),
                            side,
                            entry.item.clone(),
                        )
                    })
                })
            })
            .collect::<Vec<_>>();
        let floating_specs = floating
            .iter()
            .map(|runtime| (runtime.bounds, runtime.surface.clone()))
            .collect::<Vec<_>>();
        let host_sync = self
            .floating_hosts
            .prepare_sync(&floating_specs, &self.owner)?;

        #[cfg(test)]
        if self.fail_after_reconcile_plan {
            self.fail_after_reconcile_plan = false;
            host_sync.abort();
            return Err(DockLayoutError::InvalidSnapshot {
                reason: "injected late planning failure".to_owned(),
            });
        }

        Ok(ReconcilePlan {
            snapshot,
            desired_owners,
            planned_groups,
            planned_splits,
            group_items,
            group_roots,
            group_selected,
            auto_hide_roots,
            main_surface_child: root.as_ref().map(RuntimeNode::element),
            root,
            floating,
            surfaces,
            open_auto_hide,
            host_sync,
        })
    }

    fn commit_reconcile(&mut self, plan: ReconcilePlan) -> PreparedFloatingHostSync {
        let ReconcilePlan {
            snapshot,
            desired_owners,
            planned_groups,
            planned_splits,
            group_items,
            group_roots,
            group_selected,
            auto_hide_roots,
            root,
            floating: planned_floating,
            surfaces,
            main_surface_child,
            open_auto_hide,
            host_sync,
        } = plan;

        self.reconciling.set(true);
        let _reconciling_guard = ReconcilingGuard(self.reconciling.clone());
        // Update retained titles while their old trees still route invalidation to the host.
        // A detached title cannot mark its retained render group dirty; unchanged bounds would
        // then leave the old text commands visible after the tree is reattached.
        for planned in planned_groups.values() {
            planned.host.title.set_text(&planned.title);
        }
        self.detach_existing_tree();
        self.detach_before_attach(&desired_owners);
        let mut previous_floating = std::mem::take(&mut self.floating);

        for planned in planned_groups.values() {
            self.apply_planned_group(planned);
        }
        // Split track sizing resolves each child's authored group through its host, so the
        // planned hosts must be visible before the split tree is applied.
        self.group_hosts = planned_groups
            .iter()
            .map(|(key, planned)| (key.clone(), planned.host.clone()))
            .collect();

        if let Some(root_node) = root.as_ref() {
            let _ = self.apply_planned_node(root_node, RootKind::Main, &[], &planned_splits);
        }
        self.main_surface.auto_hide.close();
        self.main_surface.preview.clear();
        self.main_surface.targets.clear();
        self.main_surface.insertion_marker.clear();
        self.main_surface.reset_visual_children();
        if let Some(element) = main_surface_child.clone() {
            self.main_surface.add_main_child(element);
        }

        let mut floating = Vec::with_capacity(planned_floating.len());
        for (index, planned) in planned_floating.into_iter().enumerate() {
            let root = RootKind::Floating(index);
            let mut surface = previous_floating
                .iter()
                .position(|runtime| runtime.identity == planned.identity)
                .map(|position| previous_floating.swap_remove(position).surface)
                .unwrap_or_else(|| {
                    SurfaceRuntime::new(
                        root.clone(),
                        planned.surface.clone(),
                        &self.owner,
                        self.auto_hide_extents.clone(),
                    )
                });
            surface.set_root(root.clone());
            surface.auto_hide.close();
            surface.preview.clear();
            surface.targets.clear();
            surface.insertion_marker.clear();
            surface.reset_visual_children();
            let element =
                self.apply_planned_node(&planned.node, root.clone(), &[], &planned_splits);
            surface.add_main_child(element);
            floating.push(FloatingRuntime {
                node: planned.node,
                identity: planned.identity,
                surface,
            });
        }

        let registry = &self.registry;
        let floating_count = snapshot.floating_roots.len();
        let strip_titles = |root: &RootKind| {
            snapshot
                .auto_hide
                .iter()
                .enumerate()
                .flat_map(|(side, entries)| {
                    entries.iter().filter_map(move |entry| {
                        (auto_hide_root(entry, floating_count) == *root).then(|| {
                            registry.items.get(&entry.item).map(|item| {
                                (
                                    side,
                                    entry.item.clone(),
                                    item.title_value(),
                                    item.icon_value(),
                                )
                            })
                        })?
                    })
                })
                .collect::<Vec<_>>()
        };
        self.main_surface
            .render_strips(strip_titles(&RootKind::Main).into_iter(), &self.owner);
        for (index, runtime) in floating.iter().enumerate() {
            runtime.surface.render_strips(
                strip_titles(&RootKind::Floating(index)).into_iter(),
                &self.owner,
            );
        }
        for (root, side, item) in open_auto_hide {
            let Some(authored) = self.registry.items.get(&item) else {
                continue;
            };
            let wrapper = self.registry.wrapper(&item);
            let title = authored.title_value();
            let can_pin = authored.can_pin_value();
            let can_close = authored.can_close_value();
            if let Some(surface) = match &root {
                RootKind::Main => Some(&mut self.main_surface),
                RootKind::Floating(index) => {
                    floating.get_mut(*index).map(|runtime| &mut runtime.surface)
                }
            } {
                let size = surface_extent(&surface.surface);
                surface.auto_hide.open_from_model(item.clone(), side, size);
                if surface.auto_hide.current().as_ref() == Some(&item) {
                    surface
                        .auto_hide
                        .present_open_item(wrapper, &title, can_pin, can_close);
                }
            }
        }

        self.groups = planned_groups
            .iter()
            .map(|(key, planned)| (key.clone(), planned.view.clone()))
            .collect();
        self.group_hosts = planned_groups
            .into_iter()
            .map(|(key, planned)| (key, planned.host))
            .collect();
        self.group_items = group_items;
        self.group_roots = group_roots;
        self.group_selected = group_selected;
        self.auto_hide_roots = auto_hide_roots;
        self.owners = desired_owners;
        self.root = root;
        self.floating = floating;
        self.main_surface_child = main_surface_child;
        self.surfaces = surfaces;
        self.split_views = planned_splits
            .into_iter()
            .map(|(address, planned)| (address, (planned.grid, planned.splitters, planned.view)))
            .collect();
        #[cfg(test)]
        {
            self.full_reconcile_count = self.full_reconcile_count.saturating_add(1);
        }
        host_sync
    }

    pub(crate) fn begin_drag(
        &mut self,
        model: &DockLayoutModel,
        item: DockItemId,
        host_root_position: Point,
    ) -> Result<(), DockLayoutError> {
        let Some(authored) = self.registry.items.get(&item) else {
            return Err(DockLayoutError::UnknownItem(item));
        };
        if !authored.can_dock_value() {
            return Err(DockLayoutError::InvalidSnapshot {
                reason: "dock item does not permit docking".to_owned(),
            });
        }
        let source_root =
            self.item_root(&item)
                .ok_or_else(|| DockLayoutError::InvalidSnapshot {
                    reason: "dock item has no current runtime surface".to_owned(),
                })?;
        let source_node: Rc<dyn UIElementExt> = match self.owners.get(&item) {
            Some(RuntimePresentationOwner::AutoHide { root }) => self
                .surface_runtime(root)
                .map(|surface| surface.auto_hide.drag_source_element())
                .ok_or_else(|| DockLayoutError::InvalidSnapshot {
                    reason: "auto-hide Document has no runtime popup geometry".to_owned(),
                })?,
            _ => {
                let group = self
                    .group_items
                    .iter()
                    .find(|(_, items)| items.iter().any(|candidate| candidate == &item))
                    .map(|(group, _)| group)
                    .ok_or_else(|| DockLayoutError::InvalidSnapshot {
                        reason: "dock item has no current runtime group geometry".to_owned(),
                    })?;
                let group_view = self.groups.get(group).cloned().ok_or_else(|| {
                    DockLayoutError::InvalidSnapshot {
                        reason: "dock item runtime group is unavailable".to_owned(),
                    }
                })?;
                group_view
            }
        };
        let source_bounds_host =
            SurfaceRegistry::bounds_in_host_root(&source_node).ok_or_else(|| {
                DockLayoutError::InvalidSnapshot {
                    reason: "dock item drag source has no arranged geometry".to_owned(),
                }
            })?;
        let pointer_offset = Point {
            x: host_root_position.x - source_bounds_host.x,
            y: host_root_position.y - source_bounds_host.y,
        };
        if !pointer_offset.x.is_finite() || !pointer_offset.y.is_finite() {
            return Err(DockLayoutError::InvalidSnapshot {
                reason: "dock drag pointer offset is not finite".to_owned(),
            });
        }
        let source_geometry = DragSourceGeometry {
            source_root: source_root.clone(),
            source_bounds_host,
            pointer_offset,
        };
        let from_group = !matches!(
            self.owners.get(&item),
            Some(RuntimePresentationOwner::AutoHide { .. })
        );
        let wrapper = self.registry.wrapper(&item);
        let source_tabs = self.group_items.iter().find_map(|(group, items)| {
            let index = items.iter().position(|candidate| candidate == &item)?;
            Some((self.groups.get(group)?.clone(), index, items.len()))
        });
        self.drag = Some(DragSession::begin(
            model,
            item,
            source_root,
            source_geometry,
        )?);
        self.clear_previews();
        // Like the reference, the dragged tab leaves its strip while the drag is active. This is
        // presentation only: the model and the page wrapper's ownership are unchanged.
        if from_group
            && let Some(wrapper) = wrapper
            && wrapper.visibility() == Visibility::Visible
        {
            wrapper.set_visibility(Visibility::Collapsed);
            self.drag_hidden_tab = Some(wrapper);
            // The reference shows the next Document while the selected one is dragged away.
            if let Some((view, index, count)) = source_tabs
                && count > 1
                && view.selected_index() == index
            {
                let neighbour = if index + 1 < count {
                    index + 1
                } else {
                    index - 1
                };
                view.set_selected_index(neighbour);
                self.drag_shown_selection = Some((view, index));
            }
        }
        Ok(())
    }

    fn restore_drag_hidden_tab(&mut self) {
        if let Some(wrapper) = self.drag_hidden_tab.take() {
            wrapper.set_visibility(Visibility::Visible);
        }
        if let Some((view, index)) = self.drag_shown_selection.take() {
            view.set_selected_index(index);
        }
    }

    pub(crate) fn request_close(
        &mut self,
        model: &DockLayoutModel,
        item: &DockItemId,
    ) -> Result<DockLayoutModel, DockLayoutError> {
        let authored = self
            .registry
            .items
            .get(item)
            .ok_or_else(|| DockLayoutError::UnknownItem(item.clone()))?;
        if !authored.can_close_value() {
            return Err(DockLayoutError::InvalidSnapshot {
                reason: "dock item does not permit closing".to_owned(),
            });
        }
        model.with_item_closed(item)
    }

    pub(crate) fn request_pin(
        &mut self,
        model: &DockLayoutModel,
        item: &DockItemId,
        side: crate::DockSide,
    ) -> Result<DockLayoutModel, DockLayoutError> {
        let authored = self
            .registry
            .items
            .get(item)
            .ok_or_else(|| DockLayoutError::UnknownItem(item.clone()))?;
        if !authored.can_pin_value() {
            return Err(DockLayoutError::InvalidSnapshot {
                reason: "dock item does not permit pinning".to_owned(),
            });
        }
        model.with_item_moved(item, crate::DockPlacement::AutoHide { side })
    }

    pub(crate) fn preview_drag(
        &mut self,
        target: &ResolvedDockTarget,
        weight: f32,
    ) -> Result<(), DockLayoutError> {
        let insertion_marker = self.insertion_marker_rect(target);
        if let Some(drag) = self.drag.as_mut() {
            drag.preview(target, weight)?;
        } else {
            return Err(DockLayoutError::InvalidSnapshot {
                reason: "dock preview requested without an active drag".to_owned(),
            });
        }
        self.clear_previews_except(&target.root);
        let Some(surface) = self.surface_runtime_mut(&target.root) else {
            return Err(DockLayoutError::InvalidFloatingRoot {
                index: match target.root {
                    RootKind::Floating(index) => index,
                    RootKind::Main => 0,
                },
            });
        };
        surface.preview.show(target);
        surface
            .targets
            .show(Some(target.target), target.group_bounds);
        surface.insertion_marker.show(insertion_marker);
        Ok(())
    }

    pub(crate) fn clear_drag_target(&mut self) {
        self.clear_previews();
    }

    /// Keeps the hovered group's compass and the root targets visible on `root`, with `target`
    /// as the resolved cell when the pointer is over one. Without a resolved target the drop
    /// preview stays cleared.
    pub(crate) fn show_drag_targets(
        &mut self,
        root: &RootKind,
        hovered_group: Option<Rect>,
        target: Option<crate::DockTarget>,
    ) {
        self.clear_previews_except(root);
        if let Some(surface) = self.surface_runtime_mut(root) {
            if target.is_none() {
                surface.preview.clear();
                surface.insertion_marker.clear();
            }
            surface.targets.show(target, hovered_group);
        }
    }

    pub(crate) fn drag_source_geometry(&self) -> Option<DragSourceGeometry> {
        self.drag
            .as_ref()
            .map(|drag| drag.source_geometry().clone())
    }

    pub(crate) fn floating_candidate(
        &mut self,
        bounds: crate::Rect,
    ) -> Result<DockLayoutModel, DockLayoutError> {
        let Some(drag) = self.drag.as_mut() else {
            return Err(DockLayoutError::InvalidSnapshot {
                reason: "floating candidate requested without an active drag".to_owned(),
            });
        };
        drag.set_floating_candidate(bounds)
    }

    #[cfg(test)]
    pub(crate) fn prepare_floating_host(
        &mut self,
        bounds: crate::Rect,
    ) -> Result<super::floating_window::PreparedFloatingHost, DockLayoutError> {
        self.floating_hosts
            .prepare_new(DockSurfaceView::empty_surface(), bounds, &self.owner)
    }

    pub(crate) fn floating_root_index(&self, id: FloatingHostId) -> Option<usize> {
        self.floating_hosts.root_index_for_host(id)
    }

    pub(crate) fn note_native_bounds_changed(
        &mut self,
        id: FloatingHostId,
        bounds: crate::Rect,
    ) -> bool {
        self.floating_hosts.note_native_bounds_changed(id, bounds)
    }

    pub(crate) fn begin_native_floating_close(&mut self, id: FloatingHostId) -> bool {
        self.floating_hosts.begin_native_close(id)
    }

    pub(crate) fn cancel_native_floating_close(&mut self, id: FloatingHostId) {
        self.floating_hosts.cancel_native_close(id);
    }

    #[cfg(test)]
    pub(crate) fn set_floating_host_factory_for_test(&mut self, factory: FloatingHostFactory) {
        self.floating_hosts = FloatingHostRegistry::with_factory(factory);
    }

    #[cfg(test)]
    pub(crate) fn fail_after_reconcile_plan_for_test(&mut self) {
        self.fail_after_reconcile_plan = true;
    }

    #[cfg(test)]
    pub(crate) fn full_reconcile_count_for_test(&self) -> usize {
        self.full_reconcile_count
    }

    #[cfg(test)]
    pub(crate) fn theme_refresh_count_for_test(&self) -> usize {
        self.theme_refresh_count.get()
    }

    #[cfg(test)]
    /// Visible active-group frames and their stroke brushes.
    pub(crate) fn visible_active_frames_for_test(
        &self,
    ) -> Vec<Option<crate::core::graphics::Brush>> {
        self.group_hosts
            .values()
            .filter_map(|host| {
                host.active_chrome.frame_rect()?;
                let Some(ImageSource::Vector(source)) =
                    host.active_chrome.frame_source().borrow().clone()
                else {
                    return None;
                };
                let crate::core::graphics::VectorNode::Path(path) = &source.root().children[0]
                else {
                    return None;
                };
                let crate::core::graphics::VectorPaint::Brush(brush) = &path.stroke.as_ref()?.paint
                else {
                    return None;
                };
                Some(Some(brush.clone()))
            })
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn active_group_chrome_count_for_test(&self) -> usize {
        self.group_hosts
            .values()
            .filter(|host| host.active_chrome.is_active())
            .count()
    }

    #[cfg(test)]
    pub(crate) fn active_group_seams_for_test(&self) -> Vec<Rect> {
        self.group_hosts
            .values()
            .filter_map(|host| {
                let chrome = &host.active_chrome;
                if !chrome.is_active() {
                    return None;
                }
                let (size, gap, position, _) = chrome.frame_key()?;
                let (start, end) = gap?;
                Some(Rect {
                    x: start,
                    y: if position == TabStripPosition::Top {
                        chrome.strip_height() + 0.5
                    } else {
                        size.height - 0.5
                    },
                    width: end - start,
                    height: 0.0,
                })
            })
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn active_drag_for_test(&self) -> bool {
        self.drag.is_some()
    }

    #[cfg(test)]
    pub(crate) fn active_splitter_for_test(&self) -> bool {
        self.splitter.is_some()
    }

    #[cfg(test)]
    pub(crate) fn surface_for_test(&self, root: &RootKind) -> Option<Rc<DockSurfaceView>> {
        self.surface_runtime(root)
            .map(|runtime| runtime.surface.clone())
    }

    #[cfg(test)]
    pub(crate) fn group_for_test(&self, group: &SnapshotGroupKey) -> Option<Rc<CustomTabView>> {
        self.groups.get(group).cloned()
    }

    #[cfg(test)]
    pub(crate) fn group_content_header_for_test(
        &self,
        group: &SnapshotGroupKey,
    ) -> Option<Rc<Grid>> {
        self.group_hosts
            .get(group)
            .map(|host| host.content_header.clone())
    }

    #[cfg(test)]
    pub(crate) fn main_runtime_root_for_test(&self) -> Option<Rc<dyn UIElementExt>> {
        self.root.as_ref().map(RuntimeNode::element)
    }

    #[cfg(test)]
    pub(crate) fn owner_for_test(&self, item: &DockItemId) -> Option<RuntimePresentationOwner> {
        self.owners.get(item).cloned()
    }

    #[cfg(test)]
    pub(crate) fn surface_chrome_for_test(
        &self,
        root: &RootKind,
    ) -> Option<(Rc<dyn UIElementExt>, Rc<dyn UIElementExt>)> {
        self.surface_runtime(root)
            .map(|runtime| (runtime.auto_hide.visual(), runtime.preview.visual()))
    }

    #[cfg(test)]
    pub(crate) fn auto_hide_parts_for_test(&self, root: &RootKind) -> Option<(Rc<Grid>, Rc<Grid>)> {
        self.surface_runtime(root).map(|runtime| {
            (
                runtime.auto_hide.pane_for_test(),
                runtime.auto_hide.page_host_for_test(),
            )
        })
    }

    #[cfg(test)]
    pub(crate) fn wrapper_for_test(&self, item: &DockItemId) -> Option<Rc<CustomTabViewItem>> {
        self.registry.wrapper(item)
    }

    #[cfg(test)]
    pub(crate) fn preview_for_test(&self, root: &RootKind) -> Option<(crate::DockTarget, Rect)> {
        self.surface_runtime(root)
            .and_then(|runtime| runtime.preview.target().zip(runtime.preview.preview_rect()))
    }

    #[cfg(test)]
    pub(crate) fn insertion_marker_for_test(&self, root: &RootKind) -> Option<Rect> {
        self.surface_runtime(root)
            .and_then(|runtime| runtime.insertion_marker.marker_rect_for_test())
    }

    #[cfg(test)]
    pub(crate) fn show_preview_for_test(&mut self, target: ResolvedDockTarget) {
        let insertion_marker = self.insertion_marker_rect(&target);
        self.clear_previews();
        if let Some(surface) = self.surface_runtime_mut(&target.root) {
            surface.preview.show(&target);
            surface
                .targets
                .show(Some(target.target), target.group_bounds);
            surface.insertion_marker.show(insertion_marker);
        }
    }

    #[cfg(test)]
    pub(crate) fn floating_host_count_for_test(&self) -> usize {
        self.floating_hosts.host_count()
    }

    #[cfg(test)]
    pub(crate) fn surface_registry_count_for_test(&self) -> usize {
        self.surfaces.entries().len()
    }

    pub(crate) fn target_for_drop(
        &self,
        screen_position: Option<Point>,
        host_root_position: Point,
    ) -> Option<ResolvedDockTarget> {
        self.resolve_drop(screen_position, host_root_position)
            .and_then(|resolution| resolution.target)
    }

    /// Pointer-hover resolution for a Document drag: the surface under the pointer, the deepest
    /// hovered group (its compass is shown), and the target resolved where a target is drawn.
    pub(crate) fn resolve_drop(
        &self,
        screen_position: Option<Point>,
        host_root_position: Point,
    ) -> Option<DropResolution> {
        let source_root = self.drag.as_ref().map(DragSession::source_root)?;
        let (root, surface, surface_local_point) = if let Some(screen) = screen_position {
            self.surfaces
                .entries()
                .into_iter()
                .find_map(|(root, surface)| {
                    let host_root_point = surface.screen_to_root(screen)?;
                    let surface_local_point =
                        SurfaceRegistry::host_root_to_surface_local(&surface, host_root_point)?;
                    let bounds = SurfaceRegistry::surface_bounds(&surface)?;
                    contains(bounds, surface_local_point).then_some((
                        root,
                        surface,
                        surface_local_point,
                    ))
                })?
        } else {
            let surface = self.surfaces.surface_for_root(&source_root)?;
            let surface_local_point =
                SurfaceRegistry::host_root_to_surface_local(&surface, host_root_position)?;
            let bounds = SurfaceRegistry::surface_bounds(&surface)?;
            contains(bounds, surface_local_point).then_some((
                source_root,
                surface,
                surface_local_point,
            ))?
        };
        let surface_bounds = SurfaceRegistry::surface_bounds(&surface)?;

        let selected_root = root.clone();
        let groups: Vec<_> = self
            .groups
            .iter()
            .filter_map(|(key, group)| {
                if self.group_roots.get(key) != Some(&selected_root) {
                    return None;
                }
                let group_node: Rc<dyn UIElementExt> = group.clone();
                SurfaceRegistry::bounds_in_surface_local(&group_node, &surface)
                    .map(|bounds| (key.clone(), bounds))
            })
            .collect();
        // The compass and Center/Split previews use the whole group frame, including a bottom-tab
        // content header, so they center on the group as drawn (the WinUI.Dock reference does
        // the same). Tab-header resolution below stays in tab-view coordinates.
        let frames: Vec<_> = groups
            .iter()
            .map(|(key, bounds)| {
                let frame = self
                    .group_hosts
                    .get(key)
                    .and_then(|host| {
                        let container: Rc<dyn UIElementExt> = host.container.clone();
                        SurfaceRegistry::bounds_in_surface_local(&container, &surface)
                    })
                    .unwrap_or(*bounds);
                (key.clone(), frame)
            })
            .collect();
        let frame_of = |key: &SnapshotGroupKey, fallback: Rect| {
            frames
                .iter()
                .find(|(candidate, _)| candidate == key)
                .map(|(_, frame)| *frame)
                .unwrap_or(fallback)
        };
        // A root outer band would otherwise cover the first 40 px of the top strip (and the last
        // 40 px of the bottom strip), leaving individual Documents unable to reorder from the
        // strip that actually contains their pointer. While a Document drag is active, a point
        // in an arranged target tab strip resolves to that group before surface edge targets.
        if self.drag.is_some()
            && let Some((group, bounds, index)) = groups
                .iter()
                .filter_map(|(group, bounds)| {
                    let group_view = self.groups.get(group)?;
                    let local = Point {
                        x: surface_local_point.x - bounds.x,
                        y: surface_local_point.y - bounds.y,
                    };
                    let header = group_view.tab_insertion_boundary(0)?;
                    (local.y >= header.y
                        && local.y <= header.y + header.height
                        && local.x >= 0.0
                        && local.x <= bounds.width)
                        .then(|| {
                            group_view
                                .tab_insertion_index_at(local)
                                .map(|index| (group.clone(), *bounds, index))
                        })?
                })
                .min_by(|(_, left, _), (_, right, _)| {
                    (left.width * left.height).total_cmp(&(right.width * right.height))
                })
        {
            let frame = frame_of(&group, bounds);
            return Some(DropResolution {
                root: root.clone(),
                hovered_group: Some(frame),
                target: Some(ResolvedDockTarget {
                    root,
                    target: crate::DockTarget::Center,
                    group: Some(group),
                    group_bounds: Some(frame),
                    preview_rect: group_preview(frame, crate::DockTarget::Center)?,
                    tab_insert_index: Some(index),
                }),
            });
        }
        let (hovered, target) = resolve_local_target(
            root.clone(),
            surface_bounds,
            surface_local_point,
            frames.iter().cloned(),
        );
        Some(DropResolution {
            root,
            hovered_group: hovered.map(|(_, bounds)| bounds),
            target,
        })
    }

    fn insertion_marker_rect(&self, target: &ResolvedDockTarget) -> Option<Rect> {
        if target.target != crate::DockTarget::Center {
            return None;
        }
        let group = target.group.as_ref()?;
        let index = target.tab_insert_index?;
        let group_view = self.groups.get(group)?;
        let surface_runtime = self.surface_runtime(&target.root)?;
        let surface: Rc<dyn UIElementExt> = surface_runtime.surface.clone();
        let group_node: Rc<dyn UIElementExt> = group_view.clone();
        let group_bounds = SurfaceRegistry::bounds_in_surface_local(&group_node, &surface)?;
        let boundary = group_view.tab_insertion_boundary(index)?;
        Some(Rect {
            x: group_bounds.x + boundary.x,
            y: group_bounds.y + boundary.y,
            width: TAB_INSERTION_MARKER_WIDTH,
            height: boundary.height,
        })
    }

    pub(crate) fn finish_drag(&mut self, commit: bool) -> Option<DockLayoutModel> {
        self.restore_drag_hidden_tab();
        let result = if let Some(mut drag) = self.drag.take() {
            if commit {
                drag.commit()
            } else {
                Some(drag.cancel())
            }
        } else {
            None
        };
        self.clear_previews();
        result
    }

    pub(crate) fn begin_splitter(
        &mut self,
        model: &DockLayoutModel,
        address: SplitAddress,
        boundary: usize,
        grid: Rc<Grid>,
        orientation: crate::Orientation,
    ) -> bool {
        self.splitter = SplitterSession::begin(model, address, boundary, grid, orientation.into());
        self.splitter.is_some()
    }

    pub(crate) fn finish_splitter(
        &mut self,
        canceled: bool,
        cumulative_delta: f32,
    ) -> Option<DockLayoutModel> {
        let mut splitter = self.splitter.take()?;
        if canceled {
            splitter.cancel();
            return None;
        }
        self.remember_fixed_tracks(&splitter.address().clone());
        splitter.commit(cumulative_delta)
    }

    /// Authored groups contained by a runtime node.
    fn node_group_ids(&self, node: &RuntimeNode) -> Vec<DockGroupId> {
        match node {
            RuntimeNode::Group { host } => self
                .group_hosts
                .iter()
                .find(|(_, candidate)| Rc::ptr_eq(&candidate.container, host))
                .and_then(|(key, _)| match key {
                    SnapshotGroupKey::Authored(id) => Some(vec![id.clone()]),
                    SnapshotGroupKey::Generated(_) => None,
                })
                .unwrap_or_default(),
            RuntimeNode::Split { children, .. } => children
                .iter()
                .flat_map(|child| self.node_group_ids(child))
                .collect(),
        }
    }

    /// Fixed extent of `node` along an axis (`width` = true), following the reference: a group
    /// with an authored (or runtime-resized) size is fixed; a split perpendicular to the axis is
    /// fixed when every child is, at the largest child extent. Anything else is star-sized.
    fn node_fixed_extent(&self, node: &RuntimeNode, width: bool) -> Option<f32> {
        match node {
            RuntimeNode::Group { .. } => {
                let id = self.node_group_ids(node).into_iter().next()?;
                self.fixed_overrides.get(&(id.clone(), width)).copied().or({
                    let extent = self.registry.group_extent(&id);
                    if width { extent.width } else { extent.height }
                })
            }
            RuntimeNode::Split {
                children,
                orientation,
                ..
            } => {
                let perpendicular = match orientation {
                    SnapshotOrientation::Horizontal => !width,
                    SnapshotOrientation::Vertical => width,
                };
                if !perpendicular || children.is_empty() {
                    return None;
                }
                children
                    .iter()
                    .map(|child| self.node_fixed_extent(child, width))
                    .try_fold(0.0_f32, |acc, extent| extent.map(|extent| acc.max(extent)))
            }
        }
    }

    /// Authored minimum/maximum of `node` along an axis; a split perpendicular to the axis uses
    /// the largest child minimum and the smallest child maximum.
    fn node_extent_limits(&self, node: &RuntimeNode, width: bool) -> (Option<f32>, Option<f32>) {
        match node {
            RuntimeNode::Group { .. } => {
                let Some(id) = self.node_group_ids(node).into_iter().next() else {
                    return (None, None);
                };
                let extent = self.registry.group_extent(&id);
                if width {
                    (extent.min_width, extent.max_width)
                } else {
                    (extent.min_height, extent.max_height)
                }
            }
            RuntimeNode::Split {
                children,
                orientation,
                ..
            } => {
                let perpendicular = match orientation {
                    SnapshotOrientation::Horizontal => !width,
                    SnapshotOrientation::Vertical => width,
                };
                if !perpendicular {
                    return (None, None);
                }
                let limits: Vec<_> = children
                    .iter()
                    .map(|child| self.node_extent_limits(child, width))
                    .collect();
                let min = limits.iter().filter_map(|(min, _)| *min).reduce(f32::max);
                let max = limits.iter().filter_map(|(_, max)| *max).reduce(f32::min);
                (min, max)
            }
        }
    }

    fn runtime_node_at(&self, address: &SplitAddress) -> Option<&RuntimeNode> {
        let mut node = match &address.root {
            RootKind::Main => self.root.as_ref()?,
            RootKind::Floating(index) => &self.floating.get(*index)?.node,
        };
        for index in &address.path {
            let RuntimeNode::Split { children, .. } = node else {
                return None;
            };
            node = children.get(*index)?;
        }
        Some(node)
    }

    /// After a splitter drag, keeps each fixed-size child at its new pixel extent for this runtime.
    fn remember_fixed_tracks(&mut self, address: &SplitAddress) {
        let Some(RuntimeNode::Split {
            children,
            grid,
            orientation,
            ..
        }) = self.runtime_node_at(address)
        else {
            return;
        };
        let width = *orientation == SnapshotOrientation::Horizontal;
        let tracks = if width {
            grid.columns.borrow().clone()
        } else {
            grid.rows.borrow().clone()
        };
        let mut remembered = Vec::new();
        for (child, track) in children.iter().zip(tracks) {
            if let GridLength::Fixed(extent) = track
                && self.node_fixed_extent(child, width).is_some()
            {
                for id in self.node_group_ids(child) {
                    remembered.push(((id, width), extent));
                }
            }
        }
        self.fixed_overrides.extend(remembered);
    }

    #[allow(dead_code)]
    pub(crate) fn open_auto_hide(&mut self, item: DockItemId) -> Option<DockItemId> {
        let root = self.auto_hide_roots.get(&item)?.0.clone();
        self.open_auto_hide_on(root, item)
    }

    pub(crate) fn present_auto_hide(&self, item: &DockItemId) {
        let Some((root, _side)) = self.auto_hide_roots.get(item) else {
            return;
        };
        if let Some(surface) = self.surface_runtime(root) {
            self.present_auto_hide_item(surface, item);
        }
    }

    pub(crate) fn dismiss_auto_hide_for_drag(&self, item: &DockItemId) {
        let Some(RuntimePresentationOwner::AutoHide { root }) = self.owners.get(item) else {
            return;
        };
        if let Some(surface) = self.surface_runtime(root) {
            surface.auto_hide.dismiss_for_drag(item);
        }
    }

    fn present_auto_hide_item(&self, surface: &SurfaceRuntime, item: &DockItemId) {
        let Some(authored) = self.registry.items.get(item) else {
            return;
        };
        surface.auto_hide.present_open_item(
            self.registry.wrapper(item),
            &authored.title_value(),
            authored.can_pin_value(),
            authored.can_close_value(),
        );
    }

    pub(crate) fn open_auto_hide_on(
        &mut self,
        root: RootKind,
        item: DockItemId,
    ) -> Option<DockItemId> {
        self.clear_auto_hide_presentations();
        let wrapper = self.registry.wrapper(&item);
        let side = self.auto_hide_roots.get(&item)?.1;
        let size = surface_extent(&self.surface_runtime(&root)?.surface);
        let authored = self.registry.items.get(&item)?;
        let title = authored.title_value();
        let can_pin = authored.can_pin_value();
        let can_close = authored.can_close_value();
        let previous = self
            .surface_runtime_mut(&root)?
            .auto_hide
            .open(item, side, size);
        if let Some(surface) = self.surface_runtime(&root) {
            surface
                .auto_hide
                .present_open_item(wrapper, &title, can_pin, can_close);
        }
        previous
    }

    pub(crate) fn selected_group_item(&self, group: &SnapshotGroupKey) -> Option<DockItemId> {
        self.group_selected.get(group).and_then(Clone::clone)
    }

    pub(crate) fn selected_group_index(&self, group: &SnapshotGroupKey) -> Option<usize> {
        let selected = self.selected_group_item(group)?;
        self.group_items
            .get(group)?
            .iter()
            .position(|item| item == &selected)
    }

    /// Applies the non-structural part of a live tab selection. The caller has already changed
    /// the retained `CustomTabView` selected index, so this method only accepts a selection when
    /// the model transformation preserves every item/group/root relationship.
    /// The selected index of a realized group view, if the group is realized and non-empty.
    pub(crate) fn group_selected_index(&self, group: &SnapshotGroupKey) -> Option<usize> {
        self.groups
            .get(group)
            .filter(|view| !view.children().is_empty())
            .map(|view| view.selected_index())
    }

    pub(crate) fn apply_selection_fast_path(
        &mut self,
        model: &DockLayoutModel,
        group: &SnapshotGroupKey,
        index: usize,
        item: &DockItemId,
    ) -> bool {
        if self.group_item(group, index).as_ref() != Some(item)
            || !self
                .groups
                .get(group)
                .is_some_and(|view| view.selected_index() == index)
            || !model.activation_is_selection_only(item)
        {
            return false;
        }
        let owner_matches = match self.owners.get(item) {
            Some(RuntimePresentationOwner::Group(current)) => current == group,
            Some(RuntimePresentationOwner::FloatingGroup { group: current, .. }) => {
                current == group
            }
            _ => false,
        };
        if !owner_matches {
            return false;
        }

        self.group_selected
            .insert(group.clone(), Some(item.clone()));
        // The retained bottom header belongs to the selected document just as the page does.
        // Update this group's metadata without rebuilding any group or page wrappers.
        if self
            .groups
            .get(group)
            .is_some_and(|view| view.tab_strip_position() == TabStripPosition::Bottom)
            && let Some(host) = self.group_hosts.get(group)
            && let Some(authored) = self.registry.items.get(item)
        {
            host.title.set_text(&authored.title_value());
            host.pin_button.set_visibility(if authored.can_pin_value() {
                Visibility::Visible
            } else {
                Visibility::Collapsed
            });
            host.close_button
                .set_visibility(if authored.can_close_value() {
                    Visibility::Visible
                } else {
                    Visibility::Collapsed
                });
        }
        self.sync_active_group_chrome(model);
        for owner in self.owners.values_mut() {
            if matches!(owner, RuntimePresentationOwner::AutoHide { .. }) {
                *owner = RuntimePresentationOwner::None;
            }
        }
        self.clear_auto_hide_presentations();
        true
    }

    fn sync_active_group_chrome(&self, model: &DockLayoutModel) {
        let active_item = model.active_item();
        for (group, host) in &self.group_hosts {
            let view = self.groups.get(group);
            let items = self.group_items.get(group);
            let active_index = active_item.as_ref().and_then(|active| {
                items.and_then(|items| items.iter().position(|item| item == active))
            });
            let active_tab = active_item
                .as_ref()
                .filter(|_| active_index.is_some())
                .and_then(|active| self.registry.wrapper(active));
            let tab_position = view
                .map(|view| view.tab_strip_position())
                .unwrap_or(TabStripPosition::Top);
            let strip_height = if tab_position == TabStripPosition::Bottom
                && items.is_some_and(|items| items.len() == 1)
            {
                0.0
            } else {
                TAB_STRIP_HEIGHT
            };
            host.active_chrome.set_presentation(
                active_tab.is_some(),
                active_tab,
                tab_position,
                strip_height,
            );
        }
    }

    pub(crate) fn open_auto_hide_item_on(&self, root: &RootKind) -> Option<DockItemId> {
        self.surface_runtime(root)
            .and_then(|surface| surface.auto_hide.current())
    }

    pub(crate) fn can_pin(&self, item: &DockItemId) -> bool {
        self.registry
            .items
            .get(item)
            .is_some_and(|item| item.can_pin_value())
    }

    pub(crate) fn can_float(&self, item: &DockItemId) -> bool {
        self.registry
            .items
            .get(item)
            .is_some_and(|item| item.can_float_value())
    }

    pub(crate) fn dispose(&mut self) {
        self.detach_existing_tree();
        self.surface_root.children().clear();
        self.drag = None;
        self.restore_drag_hidden_tab();
        self.splitter = None;
        self.clear_previews();
        self.main_surface.auto_hide.close();
        self.floating_hosts.close_empty();
        self.surfaces.unregister(RootKind::Main);
        self.surfaces = SurfaceRegistry::default();
        self.groups.clear();
        self.group_hosts.clear();
        self.owners.clear();
        self.root = None;
        self.floating.clear();
        self.split_views.clear();
        self.group_items.clear();
        self.group_roots.clear();
        self.group_selected.clear();
        self.auto_hide_roots.clear();
        self.auto_hide_extents.borrow_mut().clear();
        self.generated_group_options.borrow_mut().clear();
    }

    /// The presentation of generated group `id`, asking the owner's creation hook the first time
    /// the group is realized (with its first Document) and reusing that answer afterwards.
    fn generated_options(&self, id: u64, items: &[DockItemId]) -> crate::DockGroupOptions {
        if let Some(options) = self.generated_group_options.borrow().get(&id) {
            return *options;
        }
        let Some(first) = items.first() else {
            return crate::DockGroupOptions::default();
        };
        let owner: Option<Rc<crate::DockingControl>> = self.owner.upgrade();
        let options = owner
            .map(|owner| owner.generated_group_options(first))
            .unwrap_or_default();
        self.generated_group_options
            .borrow_mut()
            .insert(id, options);
        options
    }

    pub(crate) fn drag_item(&self) -> Option<DockItemId> {
        self.drag.as_ref().map(|drag| drag.item().clone())
    }

    fn plan_node(
        &self,
        node: &SnapshotNode,
        groups: &mut BTreeMap<SnapshotGroupKey, Rc<CustomTabView>>,
        group_hosts: &mut BTreeMap<SnapshotGroupKey, GroupRuntimeHost>,
        planned_groups: &mut BTreeMap<SnapshotGroupKey, PlannedGroup>,
        planned_splits: &mut BTreeMap<SplitAddress, PlannedSplit>,
        group_items: &mut BTreeMap<SnapshotGroupKey, Vec<DockItemId>>,
        group_roots: &mut BTreeMap<SnapshotGroupKey, RootKind>,
        group_selected: &mut BTreeMap<SnapshotGroupKey, Option<DockItemId>>,
        used_groups: &mut BTreeSet<SnapshotGroupKey>,
        used_splits: &mut BTreeSet<SplitAddress>,
        root_kind: RootKind,
        path: &[usize],
    ) -> Result<RuntimeNode, DockLayoutError> {
        match node {
            SnapshotNode::Group {
                group,
                items,
                selected,
            } => {
                used_groups.insert(group.clone());
                group_roots.insert(group.clone(), root_kind.clone());
                let view = if let Some(view) = groups.get(group) {
                    view.clone()
                } else {
                    let view = CustomTabView::new_view();
                    view.set_connected_chrome(true);
                    self.wire_group_callbacks(&view, group.clone());
                    groups.insert(group.clone(), view.clone());
                    view
                };
                group_items.insert(group.clone(), items.clone());
                group_selected.insert(group.clone(), selected.clone());
                let tabs = items
                    .iter()
                    .map(|id| {
                        self.registry
                            .wrapper(id)
                            .ok_or_else(|| DockLayoutError::UnknownItem(id.clone()))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let generated = match &group {
                    SnapshotGroupKey::Generated(id) => Some(self.generated_options(*id, items)),
                    SnapshotGroupKey::Authored(_) => None,
                };
                let tab_position = match &group {
                    SnapshotGroupKey::Authored(id) => self.registry.group_position(id),
                    SnapshotGroupKey::Generated(_) => {
                        generated.unwrap_or_default().tab_strip_position
                    }
                };
                let compact_tabs = match &group {
                    SnapshotGroupKey::Authored(id) => self.registry.group_compact_tabs(id),
                    SnapshotGroupKey::Generated(_) => generated.unwrap_or_default().compact_tabs,
                };
                let show_when_empty = match &group {
                    SnapshotGroupKey::Authored(id) => self.registry.group_show_when_empty(id),
                    SnapshotGroupKey::Generated(_) => false,
                };
                let visibility = if items.is_empty() && !show_when_empty {
                    Visibility::Collapsed
                } else {
                    Visibility::Visible
                };
                let selected_item = selected
                    .as_ref()
                    .and_then(|selected| self.registry.items.get(selected));
                let has_pin = selected_item.is_some_and(|item| item.can_pin_value());
                let has_bottom_header =
                    tab_position == TabStripPosition::Bottom && selected_item.is_some();
                let title_visibility = if has_bottom_header {
                    Visibility::Visible
                } else {
                    Visibility::Collapsed
                };
                let pin_visibility = if has_pin {
                    Visibility::Visible
                } else {
                    Visibility::Collapsed
                };
                let close_visibility = if selected_item.is_some_and(|item| item.can_close_value()) {
                    Visibility::Visible
                } else {
                    Visibility::Collapsed
                };
                let host = group_hosts
                    .entry(group.clone())
                    .or_insert_with(|| self.new_group_host(group))
                    .clone();
                planned_groups.insert(
                    group.clone(),
                    PlannedGroup {
                        view,
                        host: host.clone(),
                        tabs,
                        items: items.clone(),
                        selected: selected.clone(),
                        tab_position,
                        compact_tabs,
                        title: selected_item
                            .map(|item| item.title_value())
                            .unwrap_or_default(),
                        visibility,
                        title_visibility,
                        pin_visibility,
                        close_visibility,
                        is_active: false,
                        active_tab: None,
                    },
                );
                Ok(RuntimeNode::Group {
                    host: host.container.clone(),
                })
            }
            SnapshotNode::Split {
                orientation,
                children: snapshot_children,
            } => {
                let split_address = SplitAddress {
                    root: root_kind.clone(),
                    path: path.to_vec(),
                };
                used_splits.insert(split_address.clone());
                let weights = snapshot_children
                    .iter()
                    .map(|child| child.weight)
                    .collect::<Vec<_>>();
                let children = snapshot_children
                    .iter()
                    .enumerate()
                    .map(|(index, child)| {
                        let mut child_path = path.to_vec();
                        child_path.push(index);
                        self.plan_node(
                            &child.node,
                            groups,
                            group_hosts,
                            planned_groups,
                            planned_splits,
                            group_items,
                            group_roots,
                            group_selected,
                            used_groups,
                            used_splits,
                            root_kind.clone(),
                            &child_path,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let (grid, mut splitters, view) = self
                    .split_views
                    .get(&split_address)
                    .cloned()
                    .unwrap_or_else(|| (Grid::new(), Vec::new(), DockSplitView::new_view()));
                while splitters.len() < children.len().saturating_sub(1) {
                    let index = splitters.len();
                    let splitter = CustomGridSplitter::new_splitter();
                    self.wire_splitter(&splitter, grid.clone(), split_address.clone(), index);
                    splitters.push(splitter);
                }
                splitters.truncate(children.len().saturating_sub(1));
                planned_splits.insert(
                    split_address,
                    PlannedSplit {
                        grid: grid.clone(),
                        splitters: splitters.clone(),
                        view: view.clone(),
                        orientation: *orientation,
                        weights,
                    },
                );
                Ok(RuntimeNode::Split {
                    children,
                    grid,
                    splitters,
                    view,
                    orientation: *orientation,
                })
            }
        }
    }

    fn new_group_host(&self, group: &SnapshotGroupKey) -> GroupRuntimeHost {
        let container = Grid::new();
        container.set_rows(vec![GridLength::Star(1.0)]);
        container.set_columns(vec![GridLength::Star(1.0)]);
        let body = Grid::new();
        body.set_rows(vec![GridLength::Auto, GridLength::Star(1.0)]);
        body.set_columns(vec![GridLength::Star(1.0)]);

        let active_chrome = GroupChromeOverlay::new();
        active_chrome.set_visibility(Visibility::Collapsed);

        let content_header = Grid::new();
        content_header.set_rows(vec![
            GridLength::Fixed(6.0),
            GridLength::Auto,
            GridLength::Fixed(6.0),
        ]);
        // Action columns are Auto: a collapsed action takes no space, so the visible actions stay
        // packed at the trailing edge like the reference's horizontal StackPanel.
        content_header.set_columns(vec![
            GridLength::Fixed(8.0),
            GridLength::Star(1.0),
            GridLength::Fixed(8.0),
            GridLength::Auto,
            GridLength::Auto,
            GridLength::Fixed(8.0),
        ]);
        content_header.set_min_height(CONTENT_HEADER_HEIGHT);
        content_header.set_background(themed_brush(BrushStyle::Secondary));
        content_header.set_visibility(Visibility::Collapsed);

        let title = TextBlock::new();
        title.set_foreground(themed_brush(BrushStyle::Foreground));
        title.set_font_weight(FontWeight(600));
        title.set_font_size(14.0);
        title.set_text_wrapping(crate::core::graphics::TextWrapping::Wrap);
        title.set_vertical_alignment(VerticalAlignment::Center);
        title.set_attached("Grid", "row", 1i32);
        title.set_attached("Grid", "column", 1i32);
        content_header.children().add(title.clone());

        let pin_button = Grid::new();
        // Keep the full button hit-testable without painting a surface over the title bar.
        pin_button.set_background(Some(Color::TRANSPARENT.into()));
        pin_button.set_width(TITLE_BUTTON_SIZE);
        pin_button.set_height(TITLE_BUTTON_SIZE);
        // Star tracks center the 16-pixel glyph in the 24-pixel button (reference padding 6).
        pin_button.set_rows(vec![GridLength::Star(1.0)]);
        pin_button.set_columns(vec![GridLength::Star(1.0)]);
        pin_button.set_vertical_alignment(VerticalAlignment::Center);
        pin_button.set_attached("Grid", "row", 1i32);
        pin_button.set_attached("Grid", "column", 3i32);
        pin_button.children().add(Self::private_icon(true));
        let weak_owner: Weak<crate::DockingControl> = self.owner.clone();
        let pin_group = group.clone();
        pin_button.register_routed_handler::<PointerEventArgs>(
            "on_pointer_released",
            Box::new(move |event, args| {
                if event.button != Some(crate::core::input::MouseButton::Left) {
                    return;
                }
                args.handled.set(true);
                let owner: Option<Rc<crate::DockingControl>> = weak_owner.upgrade();
                if let Some(owner) = owner {
                    owner.handle_selected_item_pin(pin_group.clone());
                }
            }),
        );
        pin_button.register_routed_handler::<PointerEventArgs>(
            "on_pointer_pressed",
            Box::new(|event, args| {
                if event.button == Some(crate::core::input::MouseButton::Left) {
                    args.handled.set(true);
                }
            }),
        );
        content_header.children().add(pin_button.clone());

        let close_button = Grid::new();
        // Keep the full button hit-testable without painting a surface over the content header.
        close_button.set_background(Some(Color::TRANSPARENT.into()));
        close_button.set_width(TITLE_BUTTON_SIZE);
        close_button.set_height(TITLE_BUTTON_SIZE);
        close_button.set_rows(vec![GridLength::Star(1.0)]);
        close_button.set_columns(vec![GridLength::Star(1.0)]);
        close_button.set_vertical_alignment(VerticalAlignment::Center);
        close_button.set_attached("Grid", "row", 1i32);
        close_button.set_attached("Grid", "column", 4i32);
        close_button.children().add(Self::private_icon(false));
        let weak_owner: Weak<crate::DockingControl> = self.owner.clone();
        let close_group = group.clone();
        close_button.register_routed_handler::<PointerEventArgs>(
            "on_pointer_released",
            Box::new(move |event, args| {
                if event.button != Some(crate::core::input::MouseButton::Left) {
                    return;
                }
                args.handled.set(true);
                let owner: Option<Rc<crate::DockingControl>> = weak_owner.upgrade();
                if let Some(owner) = owner {
                    owner.handle_selected_item_close(close_group.clone());
                }
            }),
        );
        close_button.register_routed_handler::<PointerEventArgs>(
            "on_pointer_pressed",
            Box::new(|event, args| {
                if event.button == Some(crate::core::input::MouseButton::Left) {
                    args.handled.set(true);
                }
            }),
        );
        content_header.children().add(close_button.clone());
        content_header.set_attached("Grid", "row", 0i32);
        content_header.set_attached("Grid", "column", 0i32);
        // WinUI.Dock activates a Document on any press inside it, even one its content handles
        // (`Document` registers `PointerPressed` with handledEventsToo). A press anywhere in the
        // group activates its selected Document; activating the active one is a no-op.
        let weak_owner: Weak<crate::DockingControl> = self.owner.clone();
        let reconciling = self.reconciling.clone();
        let pressed_group = group.clone();
        container.register_routed_handler_handled_too::<PointerEventArgs>(
            "on_pointer_pressed",
            Box::new(move |event, _| {
                if reconciling.get() || event.button != Some(crate::core::input::MouseButton::Left)
                {
                    return;
                }
                let owner: Option<Rc<crate::DockingControl>> = weak_owner.upgrade();
                if let Some(owner) = owner {
                    owner.handle_group_content_pressed(pressed_group.clone());
                }
            }),
        );

        GroupRuntimeHost {
            container,
            body,
            content_header,
            active_chrome,
            title,
            pin_button,
            close_button,
            header_drag_handlers_bound: Rc::new(Cell::new(false)),
        }
    }

    fn private_icon(pin: bool) -> Rc<dyn UIElementExt> {
        chrome_icon(
            if pin {
                ChromeIcon::Pin
            } else {
                ChromeIcon::Close
            },
            themed_brush(BrushStyle::Foreground),
        )
    }

    fn apply_planned_group(&self, planned: &PlannedGroup) {
        for (id, tab) in planned.items.iter().zip(&planned.tabs) {
            let weak_owner = self.owner.clone();
            let pin_item = id.clone();
            tab.set_document_pin_action(
                self.can_pin(id),
                Rc::new(move || {
                    let owner: Option<Rc<crate::DockingControl>> = weak_owner.upgrade();
                    if let Some(owner) = owner {
                        owner.handle_tab_context_action(
                            pin_item.clone(),
                            crate::docking_control::DockTabContextAction::Pin,
                        );
                    }
                }),
            );
        }
        replace_group_items(&planned.view, planned.tabs.clone());
        planned.view.set_tab_strip_position(planned.tab_position);
        planned.view.set_compact(planned.compact_tabs);
        planned.view.set_close_button_presentation(
            if planned.tab_position == TabStripPosition::Top {
                CloseButtonPresentation::OnPointerOver
            } else {
                CloseButtonPresentation::Never
            },
        );
        if let Some(selected) = planned.selected.as_ref() {
            if let Some(index) = planned.items.iter().position(|item| item == selected) {
                planned.view.select_index(index);
            }
        }
        let strip_height =
            if planned.tab_position == TabStripPosition::Bottom && planned.items.len() == 1 {
                0.0
            } else {
                TAB_STRIP_HEIGHT
            };
        planned.host.active_chrome.set_presentation(
            planned.is_active,
            planned.active_tab.clone(),
            planned.tab_position,
            strip_height,
        );
        planned.host.container.children().clear();
        planned.host.body.children().clear();
        planned.host.container.set_visibility(planned.visibility);
        planned
            .host
            .content_header
            .set_visibility(planned.title_visibility);
        planned
            .host
            .pin_button
            .set_visibility(planned.pin_visibility);
        planned
            .host
            .close_button
            .set_visibility(planned.close_visibility);
        planned.view.set_attached_if_changed("Grid", "row", 1i32);
        planned.view.set_attached_if_changed("Grid", "column", 0i32);
        planned
            .host
            .body
            .children()
            .add(planned.host.content_header.clone());
        self.bind_content_header_drag(&planned.host, &planned.view);
        planned.host.body.children().add(planned.view.clone());
        planned
            .host
            .container
            .children()
            .add(planned.host.body.clone());
        planned
            .host
            .container
            .children()
            .add(planned.host.active_chrome.clone());
        #[cfg(all(not(test), any(target_os = "macos", target_os = "windows")))]
        self.install_tab_context_menus(&planned.items);
    }

    fn bind_content_header_drag(&self, host: &GroupRuntimeHost, view: &Rc<CustomTabView>) {
        if host.header_drag_handlers_bound.replace(true) {
            return;
        }
        let weak_view: Weak<CustomTabView> = Rc::downgrade(view);
        host.content_header
            .register_routed_handler::<PointerEventArgs>(
                "on_pointer_pressed",
                Box::new(
                    move |event: &PointerEventArgs, args: &crate::core::input::RoutedEventArgs| {
                        if args.handled.get()
                            || event.button != Some(crate::core::input::MouseButton::Left)
                        {
                            return;
                        }
                        let view: Option<Rc<CustomTabView>> = weak_view.upgrade();
                        if let Some(view) = view {
                            view.forward_selected_item_pointer_event(TabItemPointerEvent::Pressed(
                                *event,
                            ));
                            args.handled.set(true);
                        }
                    },
                ),
            );

        let weak_view: Weak<CustomTabView> = Rc::downgrade(view);
        host.content_header
            .register_routed_handler::<PointerEventArgs>(
                "on_pointer_moved",
                Box::new(
                    move |event: &PointerEventArgs, args: &crate::core::input::RoutedEventArgs| {
                        if args.handled.get() {
                            return;
                        }
                        let view: Option<Rc<CustomTabView>> = weak_view.upgrade();
                        if let Some(view) = view {
                            view.forward_selected_item_pointer_event(TabItemPointerEvent::Moved(
                                *event,
                            ));
                            args.handled.set(true);
                        }
                    },
                ),
            );

        let weak_view: Weak<CustomTabView> = Rc::downgrade(view);
        host.content_header
            .register_routed_handler::<PointerEventArgs>(
                "on_pointer_released",
                Box::new(
                    move |event: &PointerEventArgs, args: &crate::core::input::RoutedEventArgs| {
                        if args.handled.get() {
                            return;
                        }
                        let view: Option<Rc<CustomTabView>> = weak_view.upgrade();
                        if let Some(view) = view {
                            view.forward_selected_item_pointer_event(
                                TabItemPointerEvent::Released(*event),
                            );
                            args.handled.set(true);
                        }
                    },
                ),
            );

        let weak_view: Weak<CustomTabView> = Rc::downgrade(view);
        host.content_header
            .register_routed_handler::<PointerEventArgs>(
                "on_pointer_canceled",
                Box::new(
                    move |event: &PointerEventArgs, args: &crate::core::input::RoutedEventArgs| {
                        if args.handled.get() {
                            return;
                        }
                        let view: Option<Rc<CustomTabView>> = weak_view.upgrade();
                        if let Some(view) = view {
                            view.forward_selected_item_pointer_event(
                                TabItemPointerEvent::Canceled(*event),
                            );
                            args.handled.set(true);
                        }
                    },
                ),
            );
    }

    #[cfg(all(not(test), any(target_os = "macos", target_os = "windows")))]
    fn install_tab_context_menus(&self, items: &[DockItemId]) {
        let closeable = items
            .iter()
            .map(|item| {
                self.registry
                    .items
                    .get(item)
                    .is_some_and(|item| item.can_close_value())
            })
            .collect::<Vec<_>>();
        for item in items {
            let Some(wrapper) = self.registry.wrapper(item) else {
                continue;
            };
            let Some(menu) = build_tab_context_menu(
                self.owner.clone(),
                item.clone(),
                self.registry
                    .items
                    .get(item)
                    .is_some_and(|item| item.can_close_value()),
                self.registry
                    .items
                    .get(item)
                    .is_some_and(|item| item.can_pin_value()),
                self.registry
                    .items
                    .get(item)
                    .is_some_and(|item| item.can_float_value()),
                items,
                &closeable,
            ) else {
                continue;
            };
            wrapper.set_context_menu(Some(menu));
        }
    }

    fn apply_planned_node(
        &self,
        node: &RuntimeNode,
        root_kind: RootKind,
        path: &[usize],
        splits: &BTreeMap<SplitAddress, PlannedSplit>,
    ) -> Rc<dyn UIElementExt> {
        match node {
            RuntimeNode::Group { host } => host.clone(),
            RuntimeNode::Split {
                children,
                grid,
                splitters,
                view,
                orientation,
            } => {
                let address = SplitAddress {
                    root: root_kind.clone(),
                    path: path.to_vec(),
                };
                let planned = splits
                    .get(&address)
                    .expect("planned split must exist during commit");
                grid.children().clear();
                match planned.orientation {
                    SnapshotOrientation::Horizontal => {
                        grid.set_row_spacing(0.0);
                        grid.set_column_spacing(SPLITTER_HIT_SIZE);
                        grid.set_rows(vec![GridLength::Star(1.0)]);
                        let mut columns = Vec::new();
                        let mut column_constraints = Vec::new();
                        for (index, child) in children.iter().enumerate() {
                            columns.push(
                                self.node_fixed_extent(child, true)
                                    .map(GridLength::Fixed)
                                    .unwrap_or(GridLength::Star(snapshot_split_weight(
                                        planned.weights[index],
                                    ))),
                            );
                            let (authored_min, authored_max) = self.node_extent_limits(child, true);
                            let mut child_path = path.to_vec();
                            child_path.push(index);
                            let element = self.apply_planned_node(
                                child,
                                root_kind.clone(),
                                &child_path,
                                splits,
                            );
                            column_constraints.push(GridTrackConstraint {
                                min: element.min_width().or(authored_min),
                                max: element.max_width().or(authored_max),
                            });
                            element.as_ui_element().set_attached_if_changed(
                                "Grid",
                                "column",
                                index as i32,
                            );
                            grid.children().add(element);
                        }
                        grid.set_columns(columns);
                        grid.set_column_constraints(column_constraints);
                        for index in 0..children.len().saturating_sub(1) {
                            let splitter = planned.splitters[index].clone();
                            if splitter.resize_direction() != GridResizeDirection::Columns {
                                splitter.set_resize_direction(GridResizeDirection::Columns);
                            }
                            if splitter.resize_behavior() != GridResizeBehavior::PreviousAndCurrent
                            {
                                splitter
                                    .set_resize_behavior(GridResizeBehavior::PreviousAndCurrent);
                            }
                            if splitter.width() != Some(SPLITTER_HIT_SIZE) {
                                splitter.set_width(SPLITTER_HIT_SIZE);
                            }
                            let base = splitter.as_ui_element();
                            if base.height.get().is_some()
                                || base.presentation_height.get().is_some()
                            {
                                base.height.set(None);
                                base.presentation_height.set(None);
                                splitter.invalidate_measure();
                            }
                            self.wire_splitter(&splitter, grid.clone(), address.clone(), index);
                        }
                    }
                    SnapshotOrientation::Vertical => {
                        grid.set_column_spacing(0.0);
                        grid.set_row_spacing(SPLITTER_HIT_SIZE);
                        grid.set_columns(vec![GridLength::Star(1.0)]);
                        let mut rows = Vec::new();
                        let mut row_constraints = Vec::new();
                        for (index, child) in children.iter().enumerate() {
                            rows.push(
                                self.node_fixed_extent(child, false)
                                    .map(GridLength::Fixed)
                                    .unwrap_or(GridLength::Star(snapshot_split_weight(
                                        planned.weights[index],
                                    ))),
                            );
                            let (authored_min, authored_max) =
                                self.node_extent_limits(child, false);
                            let mut child_path = path.to_vec();
                            child_path.push(index);
                            let element = self.apply_planned_node(
                                child,
                                root_kind.clone(),
                                &child_path,
                                splits,
                            );
                            row_constraints.push(GridTrackConstraint {
                                min: element.min_height().or(authored_min),
                                max: element.max_height().or(authored_max),
                            });
                            element.as_ui_element().set_attached_if_changed(
                                "Grid",
                                "row",
                                index as i32,
                            );
                            grid.children().add(element);
                        }
                        grid.set_rows(rows);
                        grid.set_row_constraints(row_constraints);
                        for index in 0..children.len().saturating_sub(1) {
                            let splitter = planned.splitters[index].clone();
                            if splitter.resize_direction() != GridResizeDirection::Rows {
                                splitter.set_resize_direction(GridResizeDirection::Rows);
                            }
                            if splitter.resize_behavior() != GridResizeBehavior::PreviousAndCurrent
                            {
                                splitter
                                    .set_resize_behavior(GridResizeBehavior::PreviousAndCurrent);
                            }
                            if splitter.height() != Some(SPLITTER_HIT_SIZE) {
                                splitter.set_height(SPLITTER_HIT_SIZE);
                            }
                            let base = splitter.as_ui_element();
                            if base.width.get().is_some() || base.presentation_width.get().is_some()
                            {
                                base.width.set(None);
                                base.presentation_width.set(None);
                                splitter.invalidate_measure();
                            }
                            self.wire_splitter(&splitter, grid.clone(), address.clone(), index);
                        }
                    }
                }
                view.configure(grid.clone(), splitters.clone(), *orientation);
                view.clone()
            }
        }
    }

    fn wire_group_callbacks(&self, view: &Rc<CustomTabView>, group: SnapshotGroupKey) {
        let weak_owner: Weak<crate::DockingControl> = self.owner.clone();
        let reconciling = self.reconciling.clone();
        let selected_group = group.clone();
        view.set_on_selected_index_change(Box::new(move |index| {
            if reconciling.get() {
                return;
            }
            let owner: Option<Rc<crate::DockingControl>> = weak_owner.upgrade();
            if let Some(owner) = owner {
                owner.handle_group_selected(selected_group.clone(), index);
            }
        }));
        // Pressing the tab that is already selected changes no selection but must still activate
        // its Document, so the active marker can move to another group (WinUI.Dock activates on
        // every tab press). Activation of an already active item is a no-op.
        let weak_owner: Weak<crate::DockingControl> = self.owner.clone();
        let reconciling = self.reconciling.clone();
        let pressed_group = group.clone();
        view.set_on_tab_pressed(Some(Box::new(move |index| {
            if reconciling.get() {
                return;
            }
            let owner: Option<Rc<crate::DockingControl>> = weak_owner.upgrade();
            if let Some(owner) = owner {
                owner.handle_group_selected(pressed_group.clone(), index);
            }
        })));

        let weak_owner: Weak<crate::DockingControl> = self.owner.clone();
        let reconciling = self.reconciling.clone();
        let close_group = group.clone();
        view.set_on_close_request(Box::new(move |index| {
            if reconciling.get() {
                return;
            }
            let owner: Option<Rc<crate::DockingControl>> = weak_owner.upgrade();
            if let Some(owner) = owner {
                owner.handle_group_close(close_group.clone(), index);
            }
        }));

        let weak_owner: Weak<crate::DockingControl> = self.owner.clone();
        let reconciling = self.reconciling.clone();
        let start_group = group.clone();
        view.set_on_tab_drag_started(Box::new(move |args| {
            if reconciling.get() {
                return;
            }
            let owner: Option<Rc<crate::DockingControl>> = weak_owner.upgrade();
            if let Some(owner) = owner {
                owner.handle_tab_drag_started(start_group.clone(), args);
            }
        }));

        let weak_owner: Weak<crate::DockingControl> = self.owner.clone();
        let reconciling = self.reconciling.clone();
        let moved_group = group.clone();
        view.set_on_tab_drag_moved(Box::new(move |args| {
            if reconciling.get() {
                return;
            }
            let owner: Option<Rc<crate::DockingControl>> = weak_owner.upgrade();
            if let Some(owner) = owner {
                owner.handle_tab_drag_moved(moved_group.clone(), args);
            }
        }));

        let weak_owner: Weak<crate::DockingControl> = self.owner.clone();
        let reconciling = self.reconciling.clone();
        view.set_on_tab_drag_completed(Box::new(move |args| {
            if reconciling.get() {
                return;
            }
            let owner: Option<Rc<crate::DockingControl>> = weak_owner.upgrade();
            if let Some(owner) = owner {
                owner.handle_tab_drag_completed(group.clone(), args);
            }
        }));
    }

    fn wire_splitter(
        &self,
        splitter: &Rc<CustomGridSplitter>,
        grid: Rc<Grid>,
        address: SplitAddress,
        boundary: usize,
    ) {
        let weak_owner: Weak<crate::DockingControl> = self.owner.clone();
        let reconciling = self.reconciling.clone();
        let start_grid: Weak<Grid> = Rc::downgrade(&grid);
        let start_address = address.clone();
        splitter.set_on_resize_started(Box::new(
            move |args: GridSplitterResizeStartedEventArgs| {
                if reconciling.get() {
                    return;
                }
                let owner: Option<Rc<crate::DockingControl>> = weak_owner.upgrade();
                let start_grid: Option<Rc<Grid>> = start_grid.upgrade();
                if let (Some(owner), Some(start_grid)) = (owner, start_grid) {
                    owner.handle_splitter_started(
                        start_address.clone(),
                        boundary,
                        start_grid.clone(),
                        args,
                    );
                }
            },
        ));

        let weak_owner: Weak<crate::DockingControl> = self.owner.clone();
        let reconciling = self.reconciling.clone();
        splitter.set_on_resize_completed(Box::new(
            move |args: GridSplitterResizeCompletedEventArgs| {
                if reconciling.get() {
                    return;
                }
                let owner: Option<Rc<crate::DockingControl>> = weak_owner.upgrade();
                if let Some(owner) = owner {
                    owner.handle_splitter_completed(args);
                }
            },
        ));
    }

    pub(crate) fn group_item(&self, group: &SnapshotGroupKey, index: usize) -> Option<DockItemId> {
        self.group_items
            .get(group)
            .and_then(|items| items.get(index))
            .cloned()
    }

    pub(crate) fn group_for_item(&self, item: &DockItemId) -> Option<(SnapshotGroupKey, usize)> {
        self.group_items.iter().find_map(|(group, items)| {
            items
                .iter()
                .position(|candidate| candidate == item)
                .map(|index| (group.clone(), index))
        })
    }

    pub(crate) fn group_items_for(&self, group: &SnapshotGroupKey) -> Option<Vec<DockItemId>> {
        self.group_items.get(group).cloned()
    }

    pub(crate) fn can_close(&self, item: &DockItemId) -> bool {
        self.registry
            .items
            .get(item)
            .is_some_and(|item| item.can_close_value())
    }

    pub(crate) fn all_closeable(&self, items: &[DockItemId]) -> bool {
        items.iter().all(|item| {
            self.registry
                .items
                .get(item)
                .is_some_and(|item| item.can_close_value())
        })
    }

    pub(crate) fn context_floating_bounds(&self, item: &DockItemId) -> Option<crate::Rect> {
        let (group, _) = self.group_for_item(item)?;
        self.context_group_floating_bounds(&group)
    }

    pub(crate) fn context_group_floating_bounds(
        &self,
        group: &SnapshotGroupKey,
    ) -> Option<crate::Rect> {
        let group = self.groups.get(group).cloned()?;
        let group_node: Rc<dyn UIElementExt> = group;
        let arranged = SurfaceRegistry::bounds_in_host_root(&group_node)?;
        Some(crate::Rect {
            x: 120.0,
            y: 120.0,
            width: arranged.width.max(super::metrics::FLOATING_MIN_WIDTH),
            height: arranged.height.max(super::metrics::FLOATING_MIN_HEIGHT),
        })
    }

    /// Side for pinning `item`: its runtime preferred side, otherwise the reference shape rule —
    /// a group narrower than tall picks the nearer of Left/Right, any other the nearer of
    /// Top/Bottom, measured from the group's surface-local origin.
    pub(crate) fn pin_side(&self, item: &DockItemId) -> Option<crate::DockSide> {
        if let Some(side) = self.preferred_sides.get(item) {
            return Some(*side);
        }
        let key = self
            .group_items
            .iter()
            .find(|(_, items)| items.iter().any(|candidate| candidate == item))
            .map(|(key, _)| key)?;
        let group_view = self.groups.get(key)?;
        let group_node: Rc<dyn UIElementExt> = group_view.clone();
        let root = self.group_roots.get(key)?;
        let surface = self.surfaces.surface_for_root(root)?;
        let bounds = SurfaceRegistry::surface_bounds(&surface)?;
        let group_bounds = SurfaceRegistry::bounds_in_surface_local(&group_node, &surface)?;
        if !group_bounds.x.is_finite() || !group_bounds.y.is_finite() {
            return None;
        }
        Some(if group_bounds.width < group_bounds.height {
            if group_bounds.x < bounds.width - group_bounds.x {
                crate::DockSide::Left
            } else {
                crate::DockSide::Right
            }
        } else if group_bounds.y < bounds.height - group_bounds.y {
            crate::DockSide::Top
        } else {
            crate::DockSide::Bottom
        })
    }

    pub(crate) fn set_preferred_side(&mut self, item: DockItemId, side: crate::DockSide) {
        self.preferred_sides.insert(item, side);
    }

    /// Root and side of an auto-hidden item, for docking it back to the same root edge.
    pub(crate) fn auto_hide_location(&self, item: &DockItemId) -> Option<(RootKind, DockSide)> {
        self.auto_hide_roots.get(item).cloned()
    }

    fn item_root(&self, item: &DockItemId) -> Option<RootKind> {
        match self.owners.get(item)? {
            RuntimePresentationOwner::Group(group) => self.group_roots.get(group).cloned(),
            RuntimePresentationOwner::FloatingGroup { root, .. } => Some(RootKind::Floating(*root)),
            RuntimePresentationOwner::AutoHide { root } => Some(root.clone()),
            RuntimePresentationOwner::None => None,
        }
    }

    fn surface_runtime(&self, root: &RootKind) -> Option<&SurfaceRuntime> {
        match root {
            RootKind::Main => Some(&self.main_surface),
            RootKind::Floating(index) => self.floating.get(*index).map(|runtime| &runtime.surface),
        }
    }

    fn surface_runtime_mut(&mut self, root: &RootKind) -> Option<&mut SurfaceRuntime> {
        match root {
            RootKind::Main => Some(&mut self.main_surface),
            RootKind::Floating(index) => self
                .floating
                .get_mut(*index)
                .map(|runtime| &mut runtime.surface),
        }
    }

    /// Clears transient drag visuals on every surface except `root`, whose visuals the caller
    /// updates in place (no hide/show churn while the pointer stays on one surface).
    fn clear_previews_except(&mut self, root: &RootKind) {
        if root != &RootKind::Main {
            self.main_surface.preview.clear();
            self.main_surface.targets.clear();
            self.main_surface.insertion_marker.clear();
        }
        for (index, runtime) in self.floating.iter_mut().enumerate() {
            if root == &RootKind::Floating(index) {
                continue;
            }
            runtime.surface.preview.clear();
            runtime.surface.targets.clear();
            runtime.surface.insertion_marker.clear();
        }
    }

    fn clear_previews(&mut self) {
        self.main_surface.preview.clear();
        self.main_surface.targets.clear();
        self.main_surface.insertion_marker.clear();
        for runtime in &mut self.floating {
            runtime.surface.preview.clear();
            runtime.surface.targets.clear();
            runtime.surface.insertion_marker.clear();
        }
    }

    fn clear_auto_hide_presentations(&mut self) {
        self.main_surface.auto_hide.close();
        for runtime in &mut self.floating {
            runtime.surface.auto_hide.close();
        }
    }
}

impl RuntimeRealization {
    fn detach_existing_tree(&mut self) {
        if let Some(old_main) = self.main_surface_child.take() {
            self.surface_root.children().remove(&old_main);
        }
        if let Some(root) = self.root.as_ref() {
            detach_runtime_node(root);
        }
        self.main_surface.auto_hide.close();
        self.main_surface.preview.clear();
        self.main_surface.targets.clear();
        self.main_surface.insertion_marker.clear();
        for runtime in &mut self.floating {
            runtime.surface.auto_hide.close();
            runtime.surface.preview.clear();
            runtime.surface.targets.clear();
            runtime.surface.insertion_marker.clear();
            runtime.surface.surface.content_root().children().clear();
            detach_runtime_node(&runtime.node);
        }
    }

    fn detach_before_attach(&mut self, desired: &BTreeMap<DockItemId, RuntimePresentationOwner>) {
        // Clear every old tab parent before any desired parent is built. This makes the ownership
        // transition explicit and prevents CustomTabView's duplicate-parent guard from seeing a
        // wrapper in two visual collections during reconciliation.
        for (item, current) in &self.owners {
            if desired.get(item) == Some(current) {
                continue;
            }
            match current {
                RuntimePresentationOwner::Group(group)
                | RuntimePresentationOwner::FloatingGroup { group, .. } => {
                    if let Some(view) = self.groups.get(group) {
                        view.replace_children(Vec::new());
                    }
                }
                RuntimePresentationOwner::AutoHide { .. } | RuntimePresentationOwner::None => {}
            }
        }
    }
}

#[cfg(all(not(test), any(target_os = "macos", target_os = "windows")))]
fn build_tab_context_menu(
    owner: Weak<crate::DockingControl>,
    item: DockItemId,
    can_close: bool,
    can_pin: bool,
    can_float: bool,
    group_items: &[DockItemId],
    closeable: &[bool],
) -> Option<Rc<dyn MenuExt>> {
    let menu = new_docking_menu()?;
    let index = group_items
        .iter()
        .position(|candidate| candidate == &item)?;
    add_tab_context_item(
        &menu,
        "Close",
        can_close,
        owner.clone(),
        item.clone(),
        crate::docking_control::DockTabContextAction::Close,
    );
    add_tab_context_item(
        &menu,
        "Close Others",
        closeable
            .iter()
            .enumerate()
            .any(|(candidate, closeable)| candidate != index && *closeable),
        owner.clone(),
        item.clone(),
        crate::docking_control::DockTabContextAction::CloseOthers,
    );
    add_tab_context_item(
        &menu,
        "Close Tabs to Left",
        closeable[..index].iter().any(|closeable| *closeable),
        owner.clone(),
        item.clone(),
        crate::docking_control::DockTabContextAction::CloseTabsToLeft,
    );
    add_tab_context_item(
        &menu,
        "Close Tabs to Right",
        closeable[index + 1..].iter().any(|closeable| *closeable),
        owner.clone(),
        item.clone(),
        crate::docking_control::DockTabContextAction::CloseTabsToRight,
    );
    add_tab_context_item(
        &menu,
        "Float",
        can_float,
        owner.clone(),
        item.clone(),
        crate::docking_control::DockTabContextAction::Float,
    );
    add_tab_context_item(
        &menu,
        "Auto Hide / Pin",
        can_pin,
        owner,
        item,
        crate::docking_control::DockTabContextAction::Pin,
    );
    Some(menu)
}

#[cfg(all(not(test), any(target_os = "macos", target_os = "windows")))]
fn add_tab_context_item(
    menu: &Rc<dyn MenuExt>,
    text: &str,
    enabled: bool,
    owner: Weak<crate::DockingControl>,
    item: DockItemId,
    action: crate::docking_control::DockTabContextAction,
) {
    let menu_item = new_docking_menu_item();
    menu_item.set_text(text);
    menu_item.set_enabled(enabled);
    menu_item.set_on_select(Box::new(move || {
        // Explicitly typed (issue #239): rust-analyzer, unlike rustc, cannot infer `owner`'s
        // type from an inline `if let Some(owner) = owner.upgrade() { owner.method(...) }` here.
        let owner: Option<Rc<crate::DockingControl>> = owner.upgrade();
        if let Some(owner) = owner {
            owner.handle_tab_context_action(item.clone(), action);
        }
    }));
    // Keep the Rust menu-item wrapper alive for as long as the native NSMenu retains the
    // NSMenuItem. AppKit does not retain an NSMenuItem target, so using the backend-only
    // `add_item` helper here drops `MenuItemTarget` immediately after construction; the menu
    // remains visible, but Close/Float actions become inert when selected.
    menu.items().add(menu_item);
}

#[cfg(all(not(test), target_os = "macos"))]
fn new_docking_menu() -> Option<Rc<dyn MenuExt>> {
    Some(elwindui_backend_appkit::Menu::new() as Rc<dyn MenuExt>)
}

#[cfg(all(not(test), target_os = "windows"))]
fn new_docking_menu() -> Option<Rc<dyn MenuExt>> {
    Some(elwindui_backend_winui3::Menu::new() as Rc<dyn MenuExt>)
}

#[cfg(all(not(test), target_os = "macos"))]
fn new_docking_menu_item() -> Rc<dyn MenuItemExt> {
    elwindui_backend_appkit::MenuItem::new() as Rc<dyn MenuItemExt>
}

#[cfg(all(not(test), target_os = "windows"))]
fn new_docking_menu_item() -> Rc<dyn MenuItemExt> {
    elwindui_backend_winui3::MenuItem::new() as Rc<dyn MenuItemExt>
}

fn detach_runtime_node(node: &RuntimeNode) {
    match node {
        RuntimeNode::Group { host } => {
            host.children().clear();
        }
        RuntimeNode::Split { children, grid, .. } => {
            for child in children {
                detach_runtime_node(child);
            }
            grid.children().clear();
        }
    }
}

fn snapshot_split_weight(weight: f32) -> f32 {
    if weight.is_finite() && weight > 0.0 {
        weight
    } else {
        1.0
    }
}

/// Result of hovering a Document drag over a surface; see `RuntimeRealization::resolve_drop`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DropResolution {
    pub(crate) root: RootKind,
    pub(crate) hovered_group: Option<Rect>,
    pub(crate) target: Option<ResolvedDockTarget>,
}

/// Last arranged size of a surface for sizing its auto-hide pane. The retained surface view keeps
/// its arrangement while reconciliation rebuilds (and re-invalidates) its content root.
fn surface_extent(surface: &Rc<DockSurfaceView>) -> Size {
    let content = surface.content_root();
    Size {
        width: surface
            .arranged_width()
            .or_else(|| content.arranged_width())
            .unwrap_or(1.0),
        height: surface
            .arranged_height()
            .or_else(|| content.arranged_height())
            .unwrap_or(1.0),
    }
}

fn contains(bounds: Rect, point: Point) -> bool {
    point.x >= bounds.x
        && point.y >= bounds.y
        && point.x <= bounds.x + bounds.width
        && point.y <= bounds.y + bounds.height
}

/// Resolves a pointer already expressed in one surface's local coordinate space. Keeping this
/// separate from the screen/root conversion makes the target facts and preview geometry one
/// operation: the exact object returned here is also the object used by the drag commit path.
/// Resolves a drop only where its target is drawn: a root-edge target rect, or one of the five
/// compass cells of the deepest group under the pointer. Also returns that hovered group, which
/// keeps its compass visible while no cell is resolved.
fn resolve_local_target(
    root: RootKind,
    surface_bounds: Rect,
    surface_local_point: Point,
    groups: impl IntoIterator<Item = (SnapshotGroupKey, Rect)>,
) -> (Option<(SnapshotGroupKey, Rect)>, Option<ResolvedDockTarget>) {
    if !valid_preview_rect(surface_bounds)
        || !surface_local_point.x.is_finite()
        || !surface_local_point.y.is_finite()
        || !contains(surface_bounds, surface_local_point)
    {
        return (None, None);
    }
    let mut deepest = None;
    let mut smallest_area = f32::INFINITY;
    for (key, bounds) in groups {
        if !valid_preview_rect(bounds) || !contains(bounds, surface_local_point) {
            continue;
        }
        let area = bounds.width * bounds.height;
        if area < smallest_area {
            smallest_area = area;
            deepest = Some((key, bounds));
        }
    }

    let surface_size = Size {
        width: surface_bounds.width,
        height: surface_bounds.height,
    };
    // Floating surfaces have no root-edge targets, like the WinUI.Dock reference.
    let root_target = [
        crate::DockTarget::DockLeft,
        crate::DockTarget::DockTop,
        crate::DockTarget::DockRight,
        crate::DockTarget::DockBottom,
    ]
    .into_iter()
    .filter(|_| root == RootKind::Main)
    .find(|target| {
        root_target_rect(*target, surface_size).is_some_and(|rect| {
            contains(
                Rect {
                    x: surface_bounds.x + rect.x,
                    y: surface_bounds.y + rect.y,
                    ..rect
                },
                surface_local_point,
            )
        })
    });
    if let Some(target) = root_target {
        let resolved =
            outer_preview(surface_bounds, target).map(|preview_rect| ResolvedDockTarget {
                root,
                target,
                group: None,
                group_bounds: None,
                preview_rect,
                tab_insert_index: None,
            });
        return (deepest, resolved);
    }

    let Some((group, bounds)) = deepest.clone() else {
        return (None, None);
    };
    let target = [
        crate::DockTarget::Center,
        crate::DockTarget::SplitLeft,
        crate::DockTarget::SplitTop,
        crate::DockTarget::SplitRight,
        crate::DockTarget::SplitBottom,
    ]
    .into_iter()
    .find(|target| {
        group_target_rect(*target, bounds).is_some_and(|rect| contains(rect, surface_local_point))
    });
    let resolved = target.and_then(|target| {
        Some(ResolvedDockTarget {
            root,
            target,
            group: Some(group),
            group_bounds: Some(bounds),
            preview_rect: group_preview(bounds, target)?,
            tab_insert_index: None,
        })
    });
    (deepest, resolved)
}

#[cfg(test)]
pub(crate) fn resolve_local_target_for_test(
    root: RootKind,
    surface_bounds: Rect,
    surface_local_point: Point,
    groups: Vec<(SnapshotGroupKey, Rect)>,
) -> Option<ResolvedDockTarget> {
    resolve_local_target(root, surface_bounds, surface_local_point, groups).1
}

fn auto_hide_root(entry: &SnapshotAutoHideEntry, floating_count: usize) -> RootKind {
    entry
        .return_state
        .floating_root
        .filter(|index| *index < floating_count)
        .map(RootKind::Floating)
        .unwrap_or(RootKind::Main)
}

fn group_identity(node: &SnapshotNode) -> Vec<SnapshotGroupKey> {
    fn visit(node: &SnapshotNode, groups: &mut BTreeSet<SnapshotGroupKey>) {
        match node {
            SnapshotNode::Group { group, .. } => {
                groups.insert(group.clone());
            }
            SnapshotNode::Split { children, .. } => {
                for child in children {
                    visit(&child.node, groups);
                }
            }
        }
    }
    let mut groups = BTreeSet::new();
    visit(node, &mut groups);
    groups.into_iter().collect()
}

fn outer_preview(surface: Rect, target: crate::DockTarget) -> Option<Rect> {
    let preview = match target {
        crate::DockTarget::DockLeft => Rect {
            x: surface.x,
            y: surface.y,
            width: surface.width * 0.5,
            height: surface.height,
        },
        crate::DockTarget::DockRight => Rect {
            x: surface.x + surface.width * 0.5,
            y: surface.y,
            width: surface.width * 0.5,
            height: surface.height,
        },
        crate::DockTarget::DockTop => Rect {
            x: surface.x,
            y: surface.y,
            width: surface.width,
            height: surface.height * 0.5,
        },
        crate::DockTarget::DockBottom => Rect {
            x: surface.x,
            y: surface.y + surface.height * 0.5,
            width: surface.width,
            height: surface.height * 0.5,
        },
        _ => return None,
    };
    valid_preview_rect(preview).then_some(preview)
}

fn group_preview(group: Rect, target: crate::DockTarget) -> Option<Rect> {
    let preview = match target {
        crate::DockTarget::Center => group,
        crate::DockTarget::SplitLeft => Rect {
            x: group.x,
            y: group.y,
            width: group.width * 0.5,
            height: group.height,
        },
        crate::DockTarget::SplitRight => Rect {
            x: group.x + group.width * 0.5,
            y: group.y,
            width: group.width * 0.5,
            height: group.height,
        },
        crate::DockTarget::SplitTop => Rect {
            x: group.x,
            y: group.y,
            width: group.width,
            height: group.height * 0.5,
        },
        crate::DockTarget::SplitBottom => Rect {
            x: group.x,
            y: group.y + group.height * 0.5,
            width: group.width,
            height: group.height * 0.5,
        },
        _ => return None,
    };
    valid_preview_rect(preview).then_some(preview)
}

fn valid_preview_rect(rect: Rect) -> bool {
    rect.x.is_finite()
        && rect.y.is_finite()
        && rect.width.is_finite()
        && rect.height.is_finite()
        && rect.width >= 0.0
        && rect.height >= 0.0
}

fn desired_owners(
    snapshot: &crate::snapshot::DockLayoutSnapshot,
) -> BTreeMap<DockItemId, RuntimePresentationOwner> {
    fn visit(
        node: &SnapshotNode,
        floating_root: Option<usize>,
        out: &mut BTreeMap<DockItemId, RuntimePresentationOwner>,
    ) {
        match node {
            SnapshotNode::Group { group, items, .. } => {
                let owner = floating_root
                    .map(|root| RuntimePresentationOwner::FloatingGroup {
                        root,
                        group: group.clone(),
                    })
                    .unwrap_or_else(|| RuntimePresentationOwner::Group(group.clone()));
                for item in items {
                    out.insert(item.clone(), owner.clone());
                }
            }
            SnapshotNode::Split { children, .. } => {
                for child in children {
                    visit(&child.node, floating_root, out);
                }
            }
        }
    }
    let mut owners = BTreeMap::new();
    if let Some(root) = &snapshot.main_root {
        visit(root, None, &mut owners);
    }
    for (index, floating) in snapshot.floating_roots.iter().enumerate() {
        visit(&floating.root, Some(index), &mut owners);
    }
    for entries in &snapshot.auto_hide {
        for entry in entries {
            owners.insert(
                entry.item.clone(),
                if entry.open {
                    RuntimePresentationOwner::AutoHide {
                        root: auto_hide_root(entry, snapshot.floating_roots.len()),
                    }
                } else {
                    RuntimePresentationOwner::None
                },
            );
        }
    }
    for entry in &snapshot.closed {
        owners.insert(entry.item.clone(), RuntimePresentationOwner::None);
    }
    owners
}

struct ReconcilingGuard(Rc<Cell<bool>>);

impl Drop for ReconcilingGuard {
    fn drop(&mut self) {
        self.0.set(false);
    }
}
