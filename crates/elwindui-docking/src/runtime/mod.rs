//! Private runtime seams. The model remains backend-neutral; these modules own the eventual
//! presentation, drag transaction, surface registry, and floating-host integrations.

mod auto_hide;
mod drag;
mod floating_window;
mod group_view;
pub(crate) mod metrics;
mod overlay;
mod reconcile;
mod split_view;
mod support;
mod surface_registry;
mod surface_view;

#[cfg(test)]
pub(crate) use auto_hide::AutoHideOverlay;
pub(crate) use drag::DragSourceGeometry;
#[cfg(test)]
pub(crate) use drag::{DragSession, ResolvedDockTarget};
pub(crate) use floating_window::FloatingHostId;
pub(crate) use floating_window::PreparedFloatingHostSync;
#[cfg(test)]
pub(crate) use floating_window::{FloatingHostFactory, FloatingHostRegistry, FloatingWindowHost};
#[cfg(test)]
pub(crate) use overlay::DropPreview;
pub(crate) use reconcile::RuntimeRealization;
#[cfg(test)]
pub(crate) use reconcile::{LatestOnlyQueue, resolve_local_target_for_test};
pub(crate) use support::{themed_brush, weak_self_from_visual_owner};
#[cfg(test)]
pub(crate) use surface_registry::SurfaceRegistry;
pub(crate) use surface_view::DockSurfaceView;
