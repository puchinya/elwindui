# Layout design

Related specification: [`../../specs/ui_spec.md`](../../specs/ui_spec.md).

## Pipeline

Layout is a two-pass process:

1. `measure(available)` computes `DesiredSize` from local constraints and measured children.
2. `arrange(final_rect)` assigns the final bounds and child offsets.

Common width, height, alignment, margin, visibility, min/max, and container rules are applied around element-specific `measure_override` / `arrange_override` behavior. Rendering consumes arranged bounds and does not perform layout.

## Constraints

An unconstrained axis is represented explicitly rather than by an arbitrary large number. `ScrollView` measures content without the viewport constraint on scrollable axes while constraining the cross axis. Native measurement adapters translate this contract into each toolkit's natural-size mechanism.

Arrange may set explicit native width/height for positioning. A backend whose native measure caches those arranged values must clear them back to its `Auto` sentinel before the next natural measurement.

## Invalidation

Property changes declare whether they affect measure, arrange, or paint. `RelayoutHost` coalesces repeated requests and runs a new root pass; elements do not synchronously recurse into layout from a setter.

Inactive subtrees do not schedule independent layout. Reactivation invalidates from their host boundary.

## Host boundary

Every root or independently hosted subtree has one layout host that owns the current viewport, pending invalidation, and backend application of final rectangles. Container implementations such as `TabView` delegate page layout to those boundaries instead of mixing two root coordinate systems.
