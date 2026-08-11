# Rendering design

Related specifications: [`../../specs/graphics_spec.md`](../../specs/graphics_spec.md) and [`../../specs/ui_spec.md`](../../specs/ui_spec.md).

## Retained model

Each active element builds a `RenderGroup` containing its local commands and child groups. A `RenderTree` retains the previous group hierarchy and reconciles it with the next hierarchy so unchanged backend resources can be reused.

`RenderCommand` is an internal backend-neutral primitive set for fills, strokes, text, images, vector content, clip, transform, opacity, and supported composition operations. It is not a public graphics value contract.

## Reconciliation

Node identity follows the UI tree identity. Reconciliation updates changed groups, creates new groups, and removes groups that disappeared or became inactive. Removal prunes backend caches associated with the removed node; caches must not outlive the owning render node indefinitely.

Clip, transform, and opacity are applied in a deterministic parent-to-child stack. Backend replay must restore state after a group so sibling groups cannot inherit transient state.

## Backend seam

The common runtime produces retained groups and invalidation signals. AppKit replays them into Core Graphics / layer resources; WinUI 3 replays them into Win2D / Composition resources. Backend documents define those mappings and lifetime details.

Image and vector caches key by stable resource identity plus the rendering parameters that affect the raster result. Automatic raster sizes may shrink after demand falls; pruning is driven by tree ownership rather than global age alone.

## Activation

Deactivating a hosted subtree drops its `RenderTree` and backend drawing caches but retains the UI tree and native-control state. Reactivation creates a fresh tree after layout with the current viewport.
