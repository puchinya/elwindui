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

## Layout reflow

Layout reflow implements the specification's structural position animation
without a public channel. `AnimationRuntime` keys its channels by a private
`RuntimeChannel::{Public(AnimationChannel), LayoutReflow}`; reflow uses an
`AnimatedValue::Transform` with translation only. Every channel insertion
receives a generation, so a channel cancelled or replaced from inside a `tick`
callback is never reinserted.

Intent is recorded when a mutation happens, never read from the transaction
during reconcile, because hosts may realize layout after the closure ends.
Effective Visual collection add/insert/remove/remove_at/clear, the
active-to-exiting move, and `Visible <-> Collapsed` record a per-host pending
intent, `Animate(Animation)` or `Snap`; the last one wins. `Animate` requires a
normalized finite non-immediate animation, an enabled transaction, and
`reduce_motion = false`. No-op mutations, exit completion, and mutations during
subtree teardown (`unmount_subtree`, which covers `DynamicChildSlot::clear` and
owner unmount) record nothing. Property setters never record intent.

`layout_root` arms a pending intent after Arrange. The first armed
`RenderTree::reconcile` consumes it; render-only passes and mutations after the
last layout leave it pending. `RenderTree::new` discards it, so a first mount or
host re-creation never reflows.

The previous position is the matched retained `RenderGroup.offset` for the
same stable `render_group_id`; `arranged_offset` is not a baseline because
invalidation may clear it. For an Active, laid-out child with a changed finite
offset, `Animate` samples any running reflow at the runtime's current time,
starts at `D_current + (old - new)`, preserves velocity, and targets zero.
`Snap` cancels that element's reflow and writes zero. Passes without an intent
leave running reflows alone.

The starting translation is written synchronously into
`UIElement::layout_reflow_translation` and the reconciled group transform, with
no reentrant invalidation while the RenderTree is borrowed. The frame wake is
requested once after reconcile; later ticks publish through Render
invalidation. Completion writes zero and removes the channel. A tick that
observes `reduce_motion` or a dropped element cancels the channel and writes
zero. Because those Render-only passes do not rebuild the semantic snapshot by
themselves, a tick that advanced any reflow requests one accessibility update
for the tree through the existing `request_accessibility_update` route.

One Core composer, `effective_local_presentation_transform`, returns
`T(layout_reflow_translation) * local_transform(base + transition)`. Render
build/reconcile, hit testing, and accessibility bounds compose it inside the
parent's arranged-offset translation, so native projection consumes the same
`RenderGroup.transform` as self-drawn content. Unregistering an animation frame
host discards pending intent and active reflows, and resets their translations.

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
