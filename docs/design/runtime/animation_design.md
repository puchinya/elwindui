# Animation runtime design

This design implements [`animation_spec.md`](../../specs/animation_spec.md)
without moving public semantics into a backend.

## State and ownership

Each animatable common property has target storage and presentation storage.
Setters update the target immediately and consult the current UI-thread-local
`Transaction`. A hosted element routes a target/presentation pair to the
per-host `AnimationRuntime`; an unhosted element snaps. Public getters read
targets, while layout, retained rendering, hit testing, and native projection
read presentation.

The transaction stack is scoped to a synchronous closure. It is thread-local,
panic-safe through an RAII guard, and never crosses an await boundary. Nested
transactions inherit `disables_animations`; an inner animation cannot clear an
inherited disable flag.

## AnimationRuntime

`AnimationRuntime` belongs to one hosted tree. It owns active channel state,
monotonic start/sample time, spring velocity, transition overlays, and the
strongest pending `InvalidationKind`. Its host route is weak or capability
based. Frame requests are coalesced, stop when no channels remain, and are
cancelled during host deactivation/teardown. Test hosts tick explicit elapsed
time instead of sleeping.

Retargeting samples the current presentation and starts the new trajectory from
that value. Spring retargeting preserves the sampled velocity. Width/height/
min/max/margin channels request Measure invalidation; opacity and transform-only
channels request Render invalidation. Coalescing retains the strongest kind.

## Transform and render flow

Core owns one helper for local transform-origin composition and every consumer
uses it. `RenderGroup` retains presentation transform/opacity instead of
baking parent transforms into leaf commands, so parent animation does not
rebuild unrelated command buffers. Reconciliation detects presentation changes
while preserving existing retained paint/native Z-order behavior.

Hit testing recursively carries cumulative transforms and tests local clips
before descending. It uses `AffineTransform::invert`, and a failed inverse
rejects the transformed subtree.

## Dynamic removal

`UIElementCollection` remains the logical/Visual ownership boundary. Logical
enumeration and layout use active children. Visual traversal may temporarily
include `Exiting` children. Removal drops the logical dynamic record and its
subscriptions, cancels capture, clears focus, invokes `TransitionHost`, and
only then removes/unmounts the Visual on completion. Completion is idempotent.

Centralized participation helpers prevent independent lifecycle checks from
drifting between layout, render, focus, shortcuts, and hit testing.

## Backend capabilities

`AnimationFrameHost` supplies a per-host runtime and frame wake request.
`TransitionHost` synchronously suppresses exiting native input and finalizes
exit resources. AppKit projects through its existing native-island container and
delivers main-thread ticks from CVDisplayLink. WinUI 3 projects to XAML child
transforms/opacity from `CompositionTarget.Rendering` and revokes the token when
idle. Backend failure falls back to safe immediate cleanup.

## Verification strategy

Pure curve, transaction, transform, hit-test, retarget, invalidation, and
collection lifecycle tests run in Core. Codegen tests cover AST metadata,
scoped refresh, nested scopes, trigger validation, and transition lowering.
AppKit tests cover island projection, suppression, and frame-thread safety.
WinUI tests compile and run on Windows when available; macOS cannot claim them.
