//! Private runtime seams. The model remains backend-neutral; these modules own the eventual
//! presentation, drag transaction, surface registry, and floating-host integrations.

mod auto_hide;
mod drag;
mod floating_window;
mod group_view;
mod overlay;
mod reconcile;
mod split_view;
mod support;
mod surface_registry;
mod surface_view;

#[cfg(test)]
pub(crate) use auto_hide::AutoHideOverlay;
#[cfg(test)]
pub(crate) use drag::DragSession;
#[cfg(test)]
pub(crate) use overlay::DropPreview;
#[cfg(test)]
pub(crate) use reconcile::LatestOnlyQueue;
pub(crate) use reconcile::RuntimeRealization;
pub(crate) use support::weak_self_from_visual_owner;
pub(crate) use surface_view::DockSurfaceView;
