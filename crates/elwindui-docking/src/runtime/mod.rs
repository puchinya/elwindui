//! Private runtime seams. The model remains backend-neutral; these modules own the eventual
//! presentation, drag transaction, surface registry, and floating-host integrations.

mod auto_hide;
mod drag;
mod floating_window;
mod group_view;
mod overlay;
mod reconcile;
mod split_view;
mod surface_registry;

#[cfg(test)]
pub(crate) use auto_hide::AutoHideOverlay;
#[cfg(test)]
pub(crate) use drag::DragSession;
#[cfg(test)]
pub(crate) use overlay::DropPreview;
#[cfg(test)]
pub(crate) use reconcile::LatestOnlyQueue;
pub(crate) use reconcile::RuntimeRealization;
