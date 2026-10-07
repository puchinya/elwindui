//! Private docking chrome metrics. Keeping these values together makes the interaction geometry
//! and the painted geometry share one source of truth without adding public styling API.

pub(crate) const CONTENT_HEADER_HEIGHT: f32 = 40.0;
pub(crate) const TITLE_BUTTON_SIZE: f32 = 24.0;

pub(crate) const SPLITTER_HIT_SIZE: f32 = 12.0;
/// Root-edge targets sit flush against the surface edges, like the reference's edge-aligned
/// DockTargetButtons.
pub(crate) const ROOT_TARGET_EDGE_INSET: f32 = 0.0;
pub(crate) const TAB_INSERTION_MARKER_WIDTH: f32 = 2.0;

pub(crate) const FLOATING_MIN_WIDTH: f32 = 160.0;
pub(crate) const FLOATING_MIN_HEIGHT: f32 = 120.0;
/// Size of a drag-created floating root, matching the reference default floating window.
pub(crate) const FLOATING_DEFAULT_EXTENT: f32 = 400.0;

pub(crate) const AUTO_HIDE_STRIP_SIZE: f32 = 28.0;
pub(crate) const AUTO_HIDE_ENTRY_HEIGHT: f32 = 24.0;
pub(crate) const AUTO_HIDE_ENTRY_SPACING: f32 = 16.0;
pub(crate) const AUTO_HIDE_MARKER_SIZE: f32 = 4.0;
pub(crate) const AUTO_HIDE_PANEL_HEADER_HEIGHT: f32 = 40.0;
pub(crate) const AUTO_HIDE_RESIZE_GRIP_SIZE: f32 = 6.0;

pub(crate) const COMPASS_SIZE: f32 = 124.0;
pub(crate) const COMPASS_BUTTON_SIZE: f32 = 36.0;
