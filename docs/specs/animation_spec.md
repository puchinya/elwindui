# Animation and transition specification

This document is the normative public contract for ElwindUI animation and
transition behavior. It is intentionally backend-neutral; backend documents
only describe frame delivery and projection.

## Public API

`elwindui_core::ui` and the facade `elwindui::ui` expose:

```rust
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Animation {
    Linear { duration: Duration },
    EaseIn { duration: Duration },
    EaseOut { duration: Duration },
    EaseInOut { duration: Duration },
    Spring { response: Duration, damping_ratio: f32 },
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Transaction {
    pub animation: Option<Animation>,
    pub disables_animations: bool,
}

pub fn with_transaction<R>(transaction: Transaction, body: impl FnOnce() -> R) -> R;
pub fn with_animation<R>(animation: Animation, body: impl FnOnce() -> R) -> R;
```

The constructors `linear`, `ease_in`, `ease_out`, `ease_in_out`, and `spring`
are convenience constructors. A zero duration or response is immediate.
Spring damping must be finite and strictly positive; invalid values panic as
programmer errors. Curves use monotonic elapsed time and are deterministic for
the same initial value, velocity, and sample times.

The common visual types are:

```rust
pub struct VisualTransform {
    pub translation: Vector,
    pub scale: f32,
    pub rotation: f32, // radians
}

pub struct UnitPoint { pub x: f32, pub y: f32 }
```

`UnitPoint::new` accepts finite values, including values outside `0..=1`;
`CENTER` and useful edge/corner constants are provided. Scale is uniform and
non-negative in v1. Translation and rotation must be finite.

## Target and presentation values

Application state and public property getters always expose target values.
Animation never mutates application state frame by frame. Measure, Arrange,
Render, hit testing, and native projection consume presentation values for the
animatable common properties. Without an active animation, presentation snaps
to target synchronously. An unhosted element cannot schedule frames and also
snaps synchronously. Initial mount never implicitly animates.

The common visual defaults are `opacity = 1.0`, identity
`visual_transform`, and `UnitPoint::CENTER` as `transform_origin`.
The v1 animatable layout properties are `margin`, `width`, `height`,
`min_width`, `min_height`, `max_width`, and `max_height`.

`Visible` participates normally; `Collapsed` participates in neither layout,
rendering nor input. Opacity zero is transparent but remains hit-testable when
otherwise enabled. `hit_test_visible = false` affects input only.

Transform origin is composed in local coordinates as
`T(pivot) * R(rotation) * S(scale) * T(-pivot) * T(translation)` where
`pivot = (width * origin.x, height * origin.y)`, using the existing Core
`AffineTransform` implementation.

## Implicit animation DSL

The modifier is metadata, not an ordinary property:

```rust
#[animation(
    animation = Animation::ease_in_out(Duration::from_millis(250)),
    value = expanded
)]
Button { width: if expanded { 280.0 } else { 140.0 } }
```

`animation` accepts an animation expression and `value` accepts only the
approved bare trigger field kinds. Initial mount does not trigger it. Scoped
transactions apply only to the generated dependent expression/dynamic region;
unrelated siblings are not animated. Nested scopes use the innermost active
scope, and dynamic regions inherit the transaction without duplicate refresh.
There is no generic user-property animation metadata in v1.

## Transitions

The public transition values are `identity`, `opacity`, `scale(f32)`,
`offset(Vector)`, `combined(self, other)`, and
`asymmetric(insertion, removal)`. Combined transforms use the same Core
composition helper as base transforms and multiply opacity contributions.

`#[transition(...)]` is valid only on a literal `UIElement` child structurally
owned by an `if`, `match`, or `for` dynamic region whose host is a visual
`UIElementCollection`. It is rejected for static children, menu/non-visual
collections, and other non-UIElement list entries. Validation is based on
resolved content capability, not concrete control names.

Transition overlay presentation is separate from base presentation:

```text
effective opacity   = base opacity * transition opacity
effective transform = base transform composed with transition transform
```

Transition timing comes from the active transaction. On insertion, the child is
mounted, inserted into logical and Visual ownership, initialized at the
insertion effect, made active for input, and animated to identity. If no
animation is active it is inserted immediately at identity.

On animated removal, logical ownership is dropped immediately while the child
remains in the Visual collection as `Exiting`. Pointer capture is cancelled,
focus is cleared, shortcuts and native input are suppressed, and the backend
is notified synchronously before the event loop can process another input.
The exiting Visual remains renderable until the overlay reaches its removal
effect, then it is removed and unmounted exactly once. It never participates in
layout, hit testing, focus traversal, or shortcuts.

An exiting child stays at its former Visual z-position. A replacement inserted
at the same logical position is placed after the outgoing Visual so it is
topmost in normal painter order. Reappearance after the old child leaves the
dynamic slot creates a new child; it never resurrects the old exiting child.
Owner teardown and `DynamicChildSlot::clear` are immediate and do not play
exit transitions.

## Input, clipping, and reduced motion

Hit testing follows the rendered presentation transform. The algorithm carries
the cumulative transform, maps the root point through its inverse into local
coordinates, tests local bounds, and checks each effective clipping ancestor in
that ancestor's local coordinates. Rotated clips are not approximated by an
axis-aligned rectangle. A singular transform makes that subtree non-hit-testable;
opacity zero does not. Exiting Visuals are never hit-testable.

`Environment` key `reduce_motion = true` makes common animations and
transitions snap immediately and schedules no frame. No automatic platform
accessibility bridge is implied by this key.

## Backend guarantees and v1 boundary

Backends provide frame delivery and project Core presentation. AppKit uses the
existing native-island container and CVDisplayLink, and WinUI 3 uses
`CompositionTarget.Rendering`. Neither backend defines curve or lifecycle
semantics. Native controls must match self-drawn transform/opacity behavior and
cannot remain interactive while exiting.

Keyframe/phase animation, generic user-property animation, non-uniform/skew
transforms, and Brush/Gradient interpolation are explicitly deferred.
