# Docking runtime design

## Three layers

Docking keeps authored declarations, a value model, and runtime presentation separate:

    DockingControl content
        -> DefaultDockDefinition + stable DockItem registry
        -> DockLayoutModel
        -> private CustomTabView / CustomSplitter / overlay / floating realization

The declaration tree remains the source of registrations and reset state. The model is the only
owner of mutable placement. Runtime controls are a projection and are never used to infer the
desired layout.

## Stable ownership

StableItemRegistry creates one CustomTabViewItem wrapper for each authored item ID and retains it
across selection, move, close, auto-hide, floating, and re-dock. A reconciliation updates the
ordered parent list after a complete model transaction; it does not reconstruct page content.
Removal first detaches the wrapper from its current presentation and only then drops the
registration. Coordinator callbacks use weak links so a surface or floating host cannot keep its
owner alive.

## Transactions

Drag and splitter sessions keep an original value and a transient preview. Overlay state is
cleared on cancel, capture loss, and owner teardown. The latest-only source queue coalesces
reentrant incoming values; equal source values do not echo. A committed user operation performs one
model writeback and one layout-change notification.

Group views are realized with CustomTabView; split nodes retain CustomSplitter instances.
Floating hosts are tracked separately from the model but are created and removed from the model's
floating-root set. Empty hosts close after their last item is re-docked.

## Coordinate and lifecycle boundaries

Same-window hit testing uses root-relative points. Cross-window discovery uses only the Core
root_to_screen/screen_to_root coordinate capability and normalized logical screen points. Missing
conversion excludes a surface; docking never estimates window chrome, DPI, or native left/top
offsets.

Owner unmount cancels gestures, clears previews, disconnects weak callbacks, closes floating hosts,
and releases presentations. Native window close is admitted only when every contained item permits
closure; each item is closed once before the host is removed.
