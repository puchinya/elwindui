//! The tree host: one XAML `Panel` that reflects an `elwindui_core` element tree into real XAML
//! children and Composition visuals, and feeds native events back into core's
//! pointer/keyboard/focus dispatchers.
//!
//! Depends downward on `render` for all drawing.

mod accessibility;
mod event;
mod replay;

use crate::ffi::{
    AnyView, UiCallbackRegistryOwner, invoke_ui_bool_event_callback,
    invoke_ui_context_event_callback, invoke_ui_event_callback, invoke_ui_key_event_callback,
    invoke_ui_pointer_event_callback, invoke_ui_right_tapped_callback,
    invoke_ui_text_event_callback,
};
use event::*;
use replay::*;

use crate::bindings::Microsoft;
use crate::bindings::Microsoft::UI::Input::PointerUpdateKind;
use crate::bindings::Microsoft::UI::Xaml::Controls::Canvas;
use crate::bindings::Microsoft::UI::Xaml::Input::{
    CharacterReceivedRoutedEventArgs, KeyEventHandler, PointerEventHandler, PointerRoutedEventArgs,
};
use crate::bindings::Microsoft::UI::Xaml::Media::CompositionTarget;
use crate::bindings::Microsoft::UI::Xaml::Shapes::Rectangle;
use crate::bindings::Microsoft::UI::Xaml::{FrameworkElement, UIElement};
use crate::render::composition::{
    CompositionClipSpec, CompositionPrimitive, CompositionRenderer, DesiredCompositionIsland,
    DesiredCompositionNode, IslandId,
};
use accessibility::{WinUI3AccessibilityHost, WinUI3AccessibilityState};
use elwindui_core::base::{Point, Rect};
use elwindui_core::input::{
    FocusState, KeyboardDispatcher, MouseButton, PointerDispatcher, RawKeyEvent, RawKeyEventKind,
    RawPointerEvent, RawPointerEventKind, RawTextInputEvent,
};
use elwindui_core::ui::{
    AnimationFrameHost, AnimationRuntime, CoordinateHost, FocusHost, PointerGestureHost,
    RelayoutHost, UIElementExt,
};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicU64, Ordering};
use windows::Foundation::{EventHandler, TypedEventHandler};
use windows::core::{IInspectable, Interface, Ref};

/// The single reusable "reflect an `Rc<dyn elwindui_core::ui::UIElement>` into real XAML
/// elements" host — the WinUI3 counterpart of `elwindui-backend-appkit`'s `TreeHost`. A
/// `Canvas` needs no custom `MeasureOverride`/`ArrangeOverride` subclass (unlike `TreeHost`'s
/// `NSView` subclass) since `Canvas`'s own built-in layout already just measures every child with
/// an unconstrained size and positions it from the `Canvas.Left`/`Canvas.Top` attached properties —
/// exactly the "trust `elwindui_core::ui::layout_root`'s own absolute-rect computation, don't
/// let the native layout system second-guess it" behavior this needs. `Rectangle`/`Ellipse`/
/// `TextBlock` paint nodes become real `Shapes::Rectangle`/`Shapes::Ellipse`/`Controls::TextBlock`
/// elements appended to `Canvas.Children` in traversal order (`Canvas` z-orders by collection
/// order — a parent's own paint is appended before its children's, so it stays behind them),
/// rather than AppKit's separate `CAShapeLayer`/`CATextLayer` sublayer mechanism.
/// Issue #235 review remediation: per-`TreeHost` relayout-reentrancy state. A thread-local
/// guard would suppress every *other* host's relayout while this one's pass is in progress, but
/// `docs/design/runtime/layout_design.md` treats each hosted subtree as owning its own layout
/// host/viewport/pending-invalidation state — one host's synchronous pass must never block a
/// sibling host (e.g. a `TabView` page host, a `ScrollView` content host) from relaying out
/// immediately. `in_progress` marks a pass as currently running *for this host*; a same-host
/// request arriving while it's set means a structural/measure change happened synchronously
/// during this host's own traversal (see `relayout_static`'s doc comment) — set `rerun_requested`
/// instead of recursing, so the still-running pass repeats once more after it finishes rather than
/// silently dropping the request or nesting a nont-tail call that can overflow the stack.
#[derive(Default)]
struct RelayoutCycleState {
    in_progress: Cell<bool>,
    rerun_requested: Cell<bool>,
}

impl RelayoutCycleState {
    /// Runs `pass` for this exact host, coalescing a same-host reentrant call — one `pass` itself
    /// triggers synchronously, on this same `RelayoutCycleState` — into one more run instead of
    /// either recursing (the stack-overflow bug this replaces) or dropping the request (unsafe:
    /// the subtree that just changed may already be behind the still-running pass). A call against
    /// a *different* `RelayoutCycleState` is unaffected no matter how deeply nested, since each
    /// host's in-progress/rerun state is its own `Cell` pair, never shared or thread-local.
    ///
    /// Returns `false` without running `pass` when a call for this same host is already in
    /// progress further up the call stack; that call's own loop (below) picks up this request via
    /// `rerun_requested` once it finishes its current iteration. Returns `true` once this call's
    /// own loop (the outermost one for this host) has completed, including every coalesced rerun.
    fn run_coalesced(&self, mut pass: impl FnMut()) -> bool {
        if self.in_progress.replace(true) {
            self.rerun_requested.set(true);
            return false;
        }
        struct InProgressGuard<'a>(&'a RelayoutCycleState);
        impl Drop for InProgressGuard<'_> {
            fn drop(&mut self) {
                self.0.in_progress.set(false);
            }
        }
        let _guard = InProgressGuard(self);
        loop {
            self.rerun_requested.set(false);
            pass();
            if !self.rerun_requested.get() {
                break;
            }
        }
        true
    }
}

/// WinUI's display-aligned frame source for one hosted tree.
///
/// `CompositionTarget.Rendering` is a process-wide XAML event. The generated WinRT delegate is
/// `Send`, while the hosted tree is intentionally UI-thread-local and uses `Rc`; the delegate
/// therefore captures only this state object's stable address. The `TreeHost` owns the
/// `Rc`, unregisters the event before clearing the host, and drops the delegate after revocation.
/// The callback itself only runs on the XAML UI thread, so no Core animation state is touched from
/// a worker thread.
struct WinUI3RenderingState {
    host: RefCell<Weak<WinUI3RelayoutHost>>,
    token: Cell<Option<i64>>,
    handler: RefCell<Option<EventHandler<IInspectable>>>,
}

impl WinUI3RenderingState {
    fn set_host(&self, host: Weak<WinUI3RelayoutHost>) {
        self.stop();
        *self.host.borrow_mut() = host;
    }

    fn clear_host(&self) {
        self.stop();
        *self.host.borrow_mut() = Weak::<WinUI3RelayoutHost>::new();
    }

    fn ensure_started(self: &Rc<Self>) {
        if self.token.get().is_some() {
            return;
        }
        let state_address = Rc::as_ptr(self) as usize;
        let handler = EventHandler::<IInspectable>::new(
            move |_: Ref<'_, IInspectable>, _: Ref<'_, IInspectable>| {
                // SAFETY: `TreeHost` unregisters the static event before its last strong
                // reference to this state is released. The delegate cannot run after that point.
                let state = unsafe { &*(state_address as *const WinUI3RenderingState) };
                state.on_rendering();
                Ok(())
            },
        );
        match CompositionTarget::Rendering(&handler) {
            Ok(token) => {
                *self.handler.borrow_mut() = Some(handler);
                self.token.set(Some(token));
            }
            Err(error) => {
                if std::env::var_os("ELWINDUI_WINUI3_DIAGNOSTICS").is_some() {
                    eprintln!(
                        "[elwindui-winui3] CompositionTarget.Rendering registration failed: {error:?}"
                    );
                }
            }
        }
    }

    fn on_rendering(&self) {
        let host: Option<Rc<WinUI3RelayoutHost>> = self.host.borrow().upgrade();
        let Some(host) = host else {
            self.stop();
            return;
        };
        let runtime: Option<Rc<AnimationRuntime>> = host.animation_runtime.upgrade();
        let Some(runtime) = runtime else {
            self.stop();
            return;
        };
        if runtime.tick_now() {
            host.flush_interactive_relayout();
        } else {
            self.stop();
        }
    }

    fn stop(&self) {
        if let Some(token) = self.token.take() {
            let _ = CompositionTarget::RemoveRendering(token);
        }
        self.handler.borrow_mut().take();
    }
}

impl Default for WinUI3RenderingState {
    fn default() -> Self {
        Self {
            host: RefCell::new(Weak::<WinUI3RelayoutHost>::new()),
            token: Cell::new(None::<i64>),
            handler: RefCell::new(None::<EventHandler<IInspectable>>),
        }
    }
}

/// The explicit viewport a `TreeHost` measures/arranges its logical tree against — the only
/// source of `layout_root`'s available size (Issue #261 review remediation §2.2). `Some(width)`/
/// `Some(height)` is a constrained axis (normalized finite, non-negative); `None` is unconstrained
/// (Core receives `f32::INFINITY` on that axis, and the axis grows to the tree's own natural size
/// after layout — see `relayout_static_pass`). This single value replaces the previous three-way
/// split model (a separate `unconstrained_axes` flag pair, a native `Canvas.Width`/`Height` used
/// as *both* a presentation property and a layout input, and a bit-exact `last_viewport_size`
/// dedup cache) — see `TreeHost::set_viewport`'s own doc comment for why that split model was
/// itself the root cause of a same-host self-feedback cascade.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TreeHostViewport {
    pub(crate) width: Option<f64>,
    pub(crate) height: Option<f64>,
}

impl TreeHostViewport {
    fn normalized(self) -> Self {
        fn normalize(value: Option<f64>) -> Option<f64> {
            value.map(|v| if v.is_finite() { v.max(0.0) } else { 0.0 })
        }
        Self {
            width: normalize(self.width),
            height: normalize(self.height),
        }
    }
}

/// Applies the native viewport before publishing the new Core layout input. A failed native
/// setter therefore leaves `stored` unchanged, so the next owner update retries both dimensions
/// instead of being suppressed as an unchanged viewport.
fn apply_native_viewport(
    stored: &Cell<Option<TreeHostViewport>>,
    viewport: TreeHostViewport,
    mut set_width: impl FnMut(f64) -> windows::core::Result<()>,
    mut set_height: impl FnMut(f64) -> windows::core::Result<()>,
) -> windows::core::Result<bool> {
    if stored.get() == Some(viewport) {
        return Ok(false);
    }
    set_width(viewport.width.unwrap_or(f64::NAN))?;
    set_height(viewport.height.unwrap_or(f64::NAN))?;
    stored.set(Some(viewport));
    Ok(true)
}

#[derive(Clone)]
pub struct TreeHost {
    canvas: Canvas,
    /// Permanent transparent, hit-testable source for blank self-drawn content. It is appended
    /// once at construction and is never part of dynamic child reconciliation.
    input_surface: Rectangle,
    /// See `RelayoutCycleState`'s own doc comment. Owned here (not a thread-local) so reentrancy
    /// coalescing is scoped to exactly this host.
    relayout_cycle: Rc<RelayoutCycleState>,
    composition: Rc<RefCell<CompositionRenderer>>,
    tree: Rc<RefCell<Option<Rc<dyn elwindui_core::ui::UIElementExt>>>>,
    render_tree: Rc<RefCell<Option<elwindui_core::graphics::RenderTree>>>,
    /// The `Text`/`NativeControl` children currently reflected into `canvas.Children()` — see
    /// `reconcile_native_children`'s own doc comment for why this exists (in short: so relayout
    /// never has to `Clear()`/rebuild `canvas.Children()` wholesale, which is what broke Win2D's
    /// device creation for whichever tab started out selected).
    native_children: Rc<RefCell<NativeChildMap>>,
    /// Turns `canvas`'s own raw `KeyDown`/`KeyUp`/`CharacterReceived` events into
    /// `elwindui_core::ui::dispatch_routed` calls against whichever element currently has focus,
    /// and owns the `FocusTracker`/`ShortcutRegistry` for whatever tree this panel hosts — mirrors
    /// `elwindui_backend_appkit::inner::TreeHostIvars::keyboard`'s own doc comment, including its
    /// caveat: self-drawn elements' virtual focus is real, but a native leaf (`Button`/`TextArea`/
    /// `TabView`) receives real OS keyboard focus/events directly and needs its own individual
    /// wiring (see `native_ui.rs`'s `Button`/`TextArea`) — `canvas`'s own `KeyDown`/`KeyUp` below
    /// never even fire while one is focused.
    keyboard: Rc<KeyboardDispatcher>,
    /// Core routed-pointer dispatcher for self-drawn content hosted directly by `canvas`.
    pointer: Rc<PointerDispatcher>,
    /// This host's current explicit viewport, supplied exclusively by its owner (Window/TabView/
    /// ScrollView/Popup — see `set_viewport`'s own doc comment) — `None` until the first call.
    /// `Rc<Cell<..>>`, not a plain `Cell<..>` field on `TreeHost` itself, because
    /// `relayout_static`'s own weakly-captured closures need their own handle to read it at fire
    /// time, the same pattern `render_tree`/`native_children` already use.
    viewport: Rc<Cell<Option<TreeHostViewport>>>,
    /// Gates all layout and rendering work while this host belongs to a non-selected tab.
    active: Rc<Cell<bool>>,
    /// Keeps track of an active standalone custom popup surface, if any, ensuring single-popup ownership.
    pub(crate) active_popup:
        Rc<RefCell<Option<Rc<dyn elwindui_core::ui::popup::PopupSurfaceHandle>>>>,
    /// Keeps every FFI callback registered by this host alive for exactly this host's lifetime.
    callback_owner: UiCallbackRegistryOwner,
    animation_runtime: Rc<AnimationRuntime>,
    rendering: Rc<WinUI3RenderingState>,
    accessibility: Rc<WinUI3AccessibilityState>,
    /// Weak route to whichever `WinUI3RelayoutHost` `set_tree()` most recently installed — lets
    /// `force_relayout_with_source` schedule through that host's own pending/queue-ticket state
    /// machine instead of calling `relayout_static` directly and bypassing it (Issue #261 review
    /// remediation §2.5). `Rc<RefCell<..>>`, not a plain `RefCell<..>` field, so it can be
    /// independently downgraded and captured weakly, the same pattern every other field here
    /// already uses. Replaced wholesale (not mutated in place) each time `set_tree()` installs a
    /// new host; the old `Weak` simply stops upgrading once nothing else holds its target's `Rc` —
    /// no explicit teardown needed.
    relayout_host: Rc<RefCell<Weak<WinUI3RelayoutHost>>>,
}

#[cfg(test)]
thread_local! {
    static RELAYOUT_STATIC_PASS_COUNT: Cell<u32> = const { Cell::new(0) };
    static RELAYOUT_REALIZATION_HISTORY: RefCell<Vec<RelayoutRealizationRecord>> = const { RefCell::new(Vec::new()) };
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RelayoutRealizationRecord {
    pub(crate) diagnostic_id: u64,
    pub(crate) source: RelayoutSource,
    pub(crate) kind: elwindui_core::ui::InvalidationKind,
}

#[cfg(test)]
pub(crate) fn relayout_static_pass_count_for_test() -> u32 {
    RELAYOUT_STATIC_PASS_COUNT.with(|count| count.get())
}

#[cfg(test)]
pub(crate) fn reset_relayout_static_pass_count_for_test() {
    RELAYOUT_STATIC_PASS_COUNT.with(|count| count.set(0));
}

#[cfg(test)]
pub(crate) fn reset_relayout_realization_history_for_test() {
    RELAYOUT_REALIZATION_HISTORY.with(|history| history.borrow_mut().clear());
}

#[cfg(test)]
pub(crate) fn relayout_realization_history_for_test() -> Vec<RelayoutRealizationRecord> {
    RELAYOUT_REALIZATION_HISTORY.with(|history| history.borrow().clone())
}

/// `elwindui_core::ui::RelayoutHost` for `TreeHost` — wraps a *weak* reference back to the
/// panel's own tree storage (not a full owned `TreeHost` clone) since a strong one would
/// create a reference cycle: this panel's own `tree` strongly holds the hosted tree's root, and
/// that root's own `UIElementImpl::invalidate_host` would then strongly hold this, right back to
/// the panel. `canvas` is captured strongly, weak `tree`.
///
/// Unlike AppKit's `AppKitRelayoutHost` (where `NSView.setNeedsLayout(true)` is itself already
/// coalesced by AppKit into a single pass per display cycle, no matter how many times it's called),
/// a plain `Canvas` has no equivalent self-batching invalidate primitive for the manually-driven
/// `relayout_static` pass this host runs. `request_relayout` therefore does its own per-host,
/// per-UI-turn coalescing: the *first* call in a burst posts exactly one `DispatcherQueue` job
/// (via the numeric-callback-id indirection every other native handler in this crate already uses,
/// since the generated dispatcher delegate requires a `Send` closure) and every subsequent call
/// against the same host, before that job runs, only updates the pending dirty state and returns.
/// This is a distinct layer from `RelayoutCycleState::run_coalesced`, which continues to own
/// same-host reentrancy *within* one already-running pass (see `relayout_static`'s own doc
/// comment) — `pending`/`queue_ticket` below own coalescing *across* separate turns instead.
/// Identifies why a given relayout cycle was scheduled/realized — attached to the
/// `ELWINDUI_PERF_TRACE` diagnostic line emitted per real cycle (see `run_relayout_now`) so a
/// live run's actual pass count can be attributed back to its originating call sites instead of
/// only being visible as one opaque total (Issue #261 review remediation §3.3). Deliberately has
/// *no* variant for a native size-change notification this host's own layout produced (e.g. a
/// `Canvas.SizeChanged`) — Issue #261 review remediation §2.6 removed that path entirely, since a
/// `TreeHost` observing its own native size output is exactly the same-host feedback loop that
/// caused a 595+-cycle cascade on live `docking-demo`; see `set_viewport`'s own doc comment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RelayoutSource {
    /// A general `elwindui_core::ui::RelayoutHost::request_relayout` invalidation (an element's
    /// own `invalidate_measure`/`invalidate_arrange`/`invalidate_render`).
    QueuedRequest,
    /// `TreeHost::set_tree()`'s own initial relayout of newly attached content.
    SetTreeInitial,
    /// `TreeHost::set_active(true)` reactivating a previously suspended host.
    SetActiveReactivate,
    /// `TreeHost::set_viewport()` pushing an owner-supplied viewport change.
    ViewportSync,
    /// `RelayoutHost::flush_interactive_relayout()` realizing an already-queued batch early.
    InteractiveFlush,
}

static NEXT_RELAYOUT_HOST_DIAGNOSTIC_ID: AtomicU64 = AtomicU64::new(1);

fn record_pending_kind_if_idle(
    in_progress: bool,
    pending_kind: &Cell<elwindui_core::ui::InvalidationKind>,
    kind: elwindui_core::ui::InvalidationKind,
) -> bool {
    if in_progress {
        return false;
    }
    pending_kind.set(pending_kind.get().max(kind));
    true
}

fn dispatcher_enqueue_accepted(result: windows::core::Result<bool>) -> bool {
    matches!(result, Ok(true))
}

pub(crate) struct WinUI3RelayoutHost {
    /// Stable, process-local id for `ELWINDUI_PERF_TRACE` diagnostics — lets a live run's log
    /// attribute every real relayout cycle to one specific host instance (Issue #261 review
    /// remediation §3.3), since two sibling hosts otherwise look identical in a raw log.
    diagnostic_id: u64,
    canvas: Canvas,
    /// Strongly retains the host's permanent input surface through queued relayouts.
    input_surface: Rectangle,
    composition: Weak<RefCell<CompositionRenderer>>,
    tree: Weak<RefCell<Option<Rc<dyn elwindui_core::ui::UIElementExt>>>>,
    render_tree: Weak<RefCell<Option<elwindui_core::graphics::RenderTree>>>,
    native_children: Weak<RefCell<NativeChildMap>>,
    /// Threaded through to `relayout_static`/`reconcile_native_children` so a genuinely new native
    /// child discovered during this relayout pass can wire its own `GotFocus`/`LostFocus` — see
    /// `reconcile_native_children`'s own doc comment on that wiring.
    keyboard: Weak<KeyboardDispatcher>,
    /// See `TreeHost::viewport`'s own doc comment.
    viewport: Weak<Cell<Option<TreeHostViewport>>>,
    /// See `TreeHost::active`.
    active: Weak<Cell<bool>>,
    /// See `RelayoutCycleState`'s own doc comment — this host's own reentrancy-coalescing state,
    /// never a thread-local, so it never suppresses a different `TreeHost`'s relayout.
    relayout_cycle: Weak<RelayoutCycleState>,
    /// `true` while a relayout obligation exists for this host that has not yet been realized —
    /// either a `DispatcherQueue` job is queued to realize it on the next UI-turn, or
    /// `flush_interactive_relayout` is about to realize it synchronously right now. Set at the
    /// start of the first `request_relayout` in a burst; cleared exactly once, by whichever of the
    /// queued job or an interactive flush realizes the pass first.
    pending: Cell<bool>,
    /// The strongest `InvalidationKind` observed *within the current batch* — reset to
    /// `InvalidationKind::default()` every time a batch is claimed (by the queued job, by
    /// `flush_interactive_relayout`, or by `force_relayout`'s synchronous path), so a previous
    /// `Measure` batch can never permanently upgrade a later, unrelated `Render`-only batch
    /// (Issue #261 review remediation §2.2 — the original version of this field never reset and
    /// so only ever grew monotonically for a host's whole lifetime). Kept so a future
    /// `InvalidationKind::Render` fast path can consult it without changing this coalescing
    /// layer's own contract; this backend still always performs a full `relayout_static` pass
    /// regardless of the recorded kind.
    pending_kind: Cell<elwindui_core::ui::InvalidationKind>,
    /// The `RelayoutSource` that started the *current* batch (first-wins: later calls folded into
    /// the same batch via `pending_kind`'s strongest-wins merge do not overwrite this) — recorded
    /// purely for the `ELWINDUI_PERF_TRACE` diagnostic emitted when the batch is realized (Issue
    /// #261 review remediation §3.3).
    pending_source: Cell<RelayoutSource>,
    /// Bumped every time the currently-queued `DispatcherQueue` job is superseded — either because
    /// it already ran (see `run_queued_relayout`) or because `flush_interactive_relayout` realized
    /// the pass first. The queued job's closure captures the ticket value valid at enqueue time and
    /// compares it against the live value before doing anything, so a stale job that still manages
    /// to fire after its work was already realized elsewhere is a guaranteed no-op rather than a
    /// second, duplicate pass.
    queue_ticket: Cell<u64>,
    /// `true` for the exact duration of `run_relayout_now`'s call to `TreeHost::relayout_static`
    /// — lets a same-host `request_relayout` call arriving *during* that pass (a structural change
    /// made mid-measure) route straight into `relayout_static` again instead of being deferred,
    /// preserving `RelayoutCycleState::run_coalesced`'s own reentrancy coalescing exactly as before
    /// this per-turn scheduling layer existed. Without this, such a call would see `pending` already
    /// cleared (case: this very call is what's currently realizing it) and incorrectly queue a
    /// second, separate `DispatcherQueue` job for work `run_coalesced`'s own rerun loop is already
    /// about to cover.
    in_progress: Cell<bool>,
    /// Owns the registration of whichever `UiCallbackRegistryOwner` numeric id the
    /// currently-queued `DispatcherQueueHandler` invokes — tied to this host's own lifetime (never
    /// a thread-local), so a detached/replaced host cannot leave a dangling queued callback that
    /// outlives it; a delegate that still fires after that safely resolves the missing id as a
    /// no-op through `invoke_ui_event_callback`, same as every other callback in this crate.
    callback_owner: UiCallbackRegistryOwner,
    /// Lets `request_relayout` (which only ever sees `&self`) upgrade to an owned `Rc<Self>` so it
    /// can read every other weakly-held backend field through one consistent handle — set once,
    /// right after this host is `Rc`-wrapped (see `TreeHost::set_tree`), the same
    /// self-referential-`Weak` pattern `InnerTabView`'s own event wiring uses for the same reason.
    weak_self: RefCell<Weak<WinUI3RelayoutHost>>,
    animation_runtime: Weak<AnimationRuntime>,
    rendering: Weak<WinUI3RenderingState>,
    /// Weak route used only after a real geometry-affecting relayout has completed. Keeping this
    /// route weak preserves the host/tree/accessibility ownership topology.
    accessibility: Weak<WinUI3AccessibilityState>,
}

impl WinUI3RelayoutHost {
    /// The actual measure/arrange/composition-reconcile realization, shared by the queued
    /// dispatcher job, `flush_interactive_relayout`, and `force_relayout`'s synchronous path.
    /// Caller is responsible for having already cleared `pending`/`pending_kind` (and bumped
    /// `queue_ticket`, if superseding a queued job) before calling this — this method only
    /// upgrades every weakly-held field, emits the `ELWINDUI_PERF_TRACE` cycle-attribution line,
    /// and runs the pass. `kind` is the strongest kind claimed for this realization and is
    /// retained in the diagnostic record even though Part A still performs a full realization
    /// for every kind.
    fn run_relayout_now(
        &self,
        source: RelayoutSource,
        kind: elwindui_core::ui::InvalidationKind,
    ) -> bool {
        // A reentrant call (see `schedule`'s `in_progress` branch) must not clear `in_progress`
        // when it returns — only the outermost call, which is the one that actually set it from
        // `false`, may do that. `RelayoutCycleState::run_coalesced` itself already guarantees a
        // reentrant call here never runs `relayout_static_pass` a second time; this only protects
        // `in_progress`'s own bookkeeping around that.
        let was_in_progress = self.in_progress.replace(true);
        let upgraded: (
            Option<Rc<RefCell<Option<Rc<dyn elwindui_core::ui::UIElementExt>>>>>,
            Option<Rc<RefCell<Option<elwindui_core::graphics::RenderTree>>>>,
            Option<Rc<RefCell<NativeChildMap>>>,
            Option<Rc<RefCell<CompositionRenderer>>>,
            Option<Rc<KeyboardDispatcher>>,
            Option<Rc<Cell<Option<TreeHostViewport>>>>,
            Option<Rc<Cell<bool>>>,
            Option<Rc<RelayoutCycleState>>,
        ) = (
            self.tree.upgrade(),
            self.render_tree.upgrade(),
            self.native_children.upgrade(),
            self.composition.upgrade(),
            self.keyboard.upgrade(),
            self.viewport.upgrade(),
            self.active.upgrade(),
            self.relayout_cycle.upgrade(),
        );
        let mut realized = false;
        if let (
            Some(tree),
            Some(render_tree),
            Some(native_children),
            Some(composition),
            Some(keyboard),
            Some(viewport),
            Some(active),
            Some(relayout_cycle),
        ) = upgraded
        {
            if std::env::var_os("ELWINDUI_PERF_TRACE").is_some() {
                eprintln!(
                    "[perf] relayout_cycle host={} source={:?} kind={:?}",
                    self.diagnostic_id, source, kind
                );
            }
            realized = TreeHost::relayout_static(
                &self.canvas,
                &self.input_surface,
                &composition,
                &tree,
                &render_tree,
                &native_children,
                &keyboard,
                viewport.get(),
                &active,
                &relayout_cycle,
                self.diagnostic_id,
            );
            if std::env::var_os("ELWINDUI_WINUI3_DIAGNOSTICS").is_some() {
                eprintln!(
                    "[elwindui-winui3] relayout_realization host={} source={:?} kind={:?} realized={}",
                    self.diagnostic_id, source, kind, realized
                );
            }
            #[cfg(test)]
            if realized {
                RELAYOUT_REALIZATION_HISTORY.with(|history| {
                    history.borrow_mut().push(RelayoutRealizationRecord {
                        diagnostic_id: self.diagnostic_id,
                        source,
                        kind,
                    });
                });
            }
            #[cfg(not(test))]
            let _ = realized;
        }
        if !was_in_progress {
            self.in_progress.set(false);
        }
        realized
    }

    fn rebuild_accessibility_after_realization(
        &self,
        realized: bool,
        kind: elwindui_core::ui::InvalidationKind,
    ) {
        if !realized || kind < elwindui_core::ui::InvalidationKind::Arrange {
            return;
        }
        let accessibility: Option<Rc<WinUI3AccessibilityState>> = self.accessibility.upgrade();
        if let Some(accessibility) = accessibility {
            accessibility.rebuild();
        }
    }

    /// Supersedes any queued dispatcher job and claims the current batch (resetting
    /// `pending`/`pending_kind` exactly once), then realizes it synchronously right now — the
    /// single shared path `flush_interactive_relayout` and `force_relayout` both use so neither
    /// leaves a stale queued job to fire a redundant second pass afterward (Issue #261 review
    /// remediation §2.5). Unlike `flush_interactive_relayout`, callers may invoke this
    /// unconditionally regardless of whether anything was actually pending.
    fn realize_synchronously(
        &self,
        source: RelayoutSource,
        kind: elwindui_core::ui::InvalidationKind,
    ) -> bool {
        self.queue_ticket
            .set(self.queue_ticket.get().wrapping_add(1));
        self.pending.set(false);
        self.pending_kind
            .set(elwindui_core::ui::InvalidationKind::default());
        self.run_relayout_now(source, kind)
    }

    /// Realizes the queued relayout, but only if `ticket` still matches this host's live
    /// `queue_ticket` — see `queue_ticket`'s own doc comment for why a stale job must be a no-op.
    fn run_queued_relayout(&self, ticket: u64) {
        if !self.pending.get() || self.queue_ticket.get() != ticket {
            return;
        }
        let source = self.pending_source.get();
        let kind = self.pending_kind.get();
        self.queue_ticket
            .set(self.queue_ticket.get().wrapping_add(1));
        self.pending.set(false);
        self.pending_kind
            .set(elwindui_core::ui::InvalidationKind::default());
        let realized = self.run_relayout_now(source, kind);
        self.rebuild_accessibility_after_realization(realized, kind);
    }

    /// The per-host, per-UI-turn coalescing layer for `RelayoutHost::request_relayout` (ordinary
    /// element invalidation) — coalesces a burst of separate top-level invalidations within one
    /// UI turn into exactly one queued job. `RelayoutHost::request_relayout` marks its render
    /// group dirty *before* calling this; this method owns everything from there on.
    fn schedule(&self, kind: elwindui_core::ui::InvalidationKind, source: RelayoutSource) {
        let active: Option<Rc<Cell<bool>>> = self.active.upgrade();
        let Some(active) = active else {
            return;
        };
        if !active.get() {
            return;
        }
        if !record_pending_kind_if_idle(self.in_progress.get(), &self.pending_kind, kind) {
            // A structural change made mid-measure, synchronously nested inside this same host's
            // own currently-running pass — hand it straight to `relayout_static` so
            // `RelayoutCycleState::run_coalesced`'s own reentrancy coalescing owns it, exactly as
            // it did before this per-turn scheduling layer existed. `run_relayout_now` upgrades
            // everything itself, so a dead host is already handled there.
            let _ = self.run_relayout_now(source, kind);
            return;
        }
        if self.pending.replace(true) {
            return; // a job is already queued for this UI-turn — it will pick up this call's changes too
        }
        self.pending_source.set(source);
        let this: Option<Rc<WinUI3RelayoutHost>> = self.weak_self.borrow().upgrade();
        let Some(this) = this else {
            self.pending.set(false);
            return;
        };
        let Ok(queue) = Microsoft::UI::Dispatching::DispatcherQueue::GetForCurrentThread() else {
            // No dispatcher available on this thread (shouldn't normally happen — `schedule` only
            // ever runs on the UI thread once a `DispatcherQueue` already exists). Realize
            // immediately rather than silently dropping the relayout.
            let kind = self.pending_kind.get();
            let realized = this.realize_synchronously(source, kind);
            this.rebuild_accessibility_after_realization(realized, kind);
            return;
        };
        let ticket = self.queue_ticket.get();
        let weak_this: Weak<WinUI3RelayoutHost> = Rc::downgrade(&this);
        let callback_id = this
            .callback_owner
            .register_one_shot_event(Rc::new(move || {
                let this: Option<Rc<WinUI3RelayoutHost>> = weak_this.upgrade();
                if let Some(this) = this {
                    this.run_queued_relayout(ticket);
                }
            }));
        let handler = Microsoft::UI::Dispatching::DispatcherQueueHandler::new(move || {
            invoke_ui_event_callback(callback_id);
            Ok(())
        });
        if !dispatcher_enqueue_accepted(queue.TryEnqueue(&handler)) {
            // Posting failed or was rejected — unregister the now-unused one-shot callback (it
            // will never fire) and realize immediately rather than leaving `pending` stuck.
            this.callback_owner.unregister_event(callback_id);
            let kind = self.pending_kind.get();
            let realized = this.realize_synchronously(source, kind);
            this.rebuild_accessibility_after_realization(realized, kind);
        }
    }
}

impl elwindui_core::ui::RelayoutHost for WinUI3RelayoutHost {
    // This backend has no `InvalidationKind::Render` fast path yet (every relayout is a full
    // `relayout_static` rebuild regardless of what was invalidated) — `kind` is folded into
    // `pending_kind` (strongest-wins within the current batch) purely so a future fast path can
    // consult it without changing this coalescing layer's contract; seeing `Render` calls here
    // does not itself skip anything.
    fn request_relayout(&self, dirty_group_id: u64, kind: elwindui_core::ui::InvalidationKind) {
        let active: Option<Rc<Cell<bool>>> = self.active.upgrade();
        let Some(active) = active else {
            return;
        };
        if !active.get() {
            return;
        }
        let render_tree: Option<Rc<RefCell<Option<elwindui_core::graphics::RenderTree>>>> =
            self.render_tree.upgrade();
        if let Some(render_tree) = render_tree {
            let mut render_tree = render_tree.borrow_mut();
            if let Some(render_tree) = render_tree.as_mut() {
                render_tree.mark_dirty(dirty_group_id);
            }
        }
        self.schedule(kind, RelayoutSource::QueuedRequest);
    }

    fn flush_interactive_relayout(&self) {
        let active: Option<Rc<Cell<bool>>> = self.active.upgrade();
        let Some(active) = active else {
            return;
        };
        if !active.get() {
            return;
        }
        if !self.pending.get() {
            return; // nothing queued or in flight — an interactive flush with no pending work is a no-op
        }
        let kind = self.pending_kind.get();
        let realized = self.realize_synchronously(RelayoutSource::InteractiveFlush, kind);
        self.rebuild_accessibility_after_realization(realized, kind);
    }
}

impl AnimationFrameHost for WinUI3RelayoutHost {
    fn animation_runtime(&self) -> Rc<AnimationRuntime> {
        let runtime = self
            .animation_runtime
            .upgrade()
            .unwrap_or_else(AnimationRuntime::new);
        runtime.sync_now();
        runtime
    }

    fn request_animation_frame(&self) {
        let rendering: Option<Rc<WinUI3RenderingState>> = self.rendering.upgrade();
        let Some(rendering) = rendering else {
            return;
        };
        rendering.ensure_started();
    }
}

/// `elwindui_core::ui::FocusHost` for `TreeHost` — the `FocusHost` counterpart to
/// `WinUI3RelayoutHost`, same weak-back-reference shape (a strong one would create the same
/// `tree` -> `focus_host` -> panel reference cycle `WinUI3RelayoutHost`'s own doc comment
/// describes). Delegates straight to `keyboard.focus`, the single source of truth for this panel's
/// own hosted tree — mirrors `elwindui_backend_appkit::inner::AppKitFocusHost`.
pub(crate) struct WinUI3FocusHost {
    keyboard: Weak<KeyboardDispatcher>,
    native_children: Weak<RefCell<NativeChildMap>>,
}

impl FocusHost for WinUI3FocusHost {
    fn request_focus(&self, target: &Rc<dyn elwindui_core::ui::UIElementExt>) -> bool {
        let keyboard: Option<Rc<KeyboardDispatcher>> = self.keyboard.upgrade();
        let Some(keyboard) = keyboard else {
            return false;
        };

        let focus: &elwindui_core::focus::FocusTracker = &keyboard.focus;
        if !target.is_tab_stop() {
            return false;
        }

        // Accessibility and programmatic Core focus both arrive through this host capability.
        // A native leaf receives real XAML focus first so WinUI3 emits GotFocus through the
        // existing native-to-Core bridge while the target is changing. The semantic peer remains
        // the only public UIA node; this lookup uses private projection bookkeeping solely for
        // the native synchronization step.
        let Some(native_children) = self.native_children.upgrade() else {
            return focus.set_focus(target, FocusState::Programmatic);
        };
        let Some(element) = native_focus_element(&native_children, target.render_group_id()) else {
            return focus.set_focus(target, FocusState::Programmatic);
        };
        let Ok(element) = element.cast::<UIElement>() else {
            return false;
        };
        if !element
            .Focus(Microsoft::UI::Xaml::FocusState::Programmatic)
            .unwrap_or(false)
        {
            return false;
        }
        if focus
            .focused()
            .is_some_and(|focused| Rc::ptr_eq(&focused, target))
        {
            return target.focus_state() == FocusState::Programmatic;
        }
        focus.set_focus(target, FocusState::Programmatic)
    }

    fn clear_focus_in_subtree(&self, subtree: &Rc<dyn elwindui_core::ui::UIElementExt>) -> bool {
        let keyboard: Option<Rc<KeyboardDispatcher>> = self.keyboard.upgrade();
        let Some(keyboard) = keyboard else {
            return false;
        };
        let focus: &elwindui_core::focus::FocusTracker = &keyboard.focus;
        elwindui_core::focus::FocusTracker::clear_focus_in_subtree(focus, subtree)
    }
}

/// Root/screen conversion for one WinUI3 hosted tree. XAML objects support WinRT weak references,
/// so the tree does not retain its owning Canvas through this capability.
pub(crate) struct WinUI3CoordinateHost {
    canvas: windows::core::Weak<Canvas>,
}

impl CoordinateHost for WinUI3CoordinateHost {
    fn root_to_screen(&self, point: Point) -> Option<Point> {
        TreeHost::canvas_to_screen_point(&self.canvas.upgrade()?, point)
    }

    fn screen_to_root(&self, point: Point) -> Option<Point> {
        TreeHost::screen_to_canvas_point(&self.canvas.upgrade()?, point)
    }
}

/// Private, observational native pointer evidence for the bounded WinUI3 E2E workflow. The trace
/// is deliberately owned by the callback closures rather than exposed through any host/public API.
/// Every write is best-effort and flushed independently so a PC-11 process shutdown does not lose
/// the preceding native ordering evidence.
#[derive(Clone)]
struct PointerTrace {
    path: Option<PathBuf>,
    next_order: Rc<Cell<u64>>,
    release_capture_on_press: bool,
}

impl PointerTrace {
    fn from_environment() -> Self {
        let path = std::env::var_os("ELWINDUI_WINUI3_POINTER_TRACE_PATH").map(PathBuf::from);
        let release_capture_on_press = Self::capture_loss_hook_enabled(
            path.is_some(),
            std::env::var("ELWINDUI_WINUI3_E2E_RELEASE_CAPTURE_ON_PRESS").ok(),
        );
        Self {
            path,
            next_order: Rc::new(Cell::new(0)),
            release_capture_on_press,
        }
    }

    fn capture_loss_hook_enabled(trace_enabled: bool, hook_value: Option<String>) -> bool {
        trace_enabled && hook_value.as_deref() == Some("1")
    }

    fn pointer_id(args: &PointerRoutedEventArgs) -> u32 {
        args.Pointer()
            .ok()
            .and_then(|pointer| pointer.PointerId().ok())
            .unwrap_or(0)
    }

    fn record_pointer(
        &self,
        event: &str,
        pointer_id: u32,
        source_classification: &str,
        forwarded_to_core: bool,
        root_position: Option<Point>,
        screen_position: Option<Point>,
    ) {
        self.write_record(
            event,
            pointer_id,
            source_classification,
            forwarded_to_core,
            root_position,
            screen_position,
            None,
            None,
        );
    }

    fn record_capture(&self, pointer_id: u32, source_classification: &str, success: bool) {
        self.write_record(
            "NativeCapture",
            pointer_id,
            source_classification,
            false,
            None,
            None,
            Some(success),
            None,
        );
    }

    fn record_release(&self, pointer_id: u32, source_classification: &str, success: bool) {
        self.write_record(
            "NativeCaptureRelease",
            pointer_id,
            source_classification,
            false,
            None,
            None,
            None,
            Some(success),
        );
    }

    fn write_record(
        &self,
        event: &str,
        pointer_id: u32,
        source_classification: &str,
        forwarded_to_core: bool,
        root_position: Option<Point>,
        screen_position: Option<Point>,
        native_capture_success: Option<bool>,
        native_capture_release_success: Option<bool>,
    ) {
        let Some(path) = self.path.as_ref() else {
            return;
        };
        let order = self.next_order.get();
        self.next_order.set(order.saturating_add(1));
        let line = format!(
            "{{\"order\":{order},\"event\":\"{event}\",\"pointer_id\":{pointer_id},\"source_classification\":\"{source_classification}\",\"forwarded_to_core\":{forwarded},\"root_position\":{root},\"screen_position\":{screen},\"native_capture_success\":{capture},\"native_capture_release_success\":{release}}}\n",
            forwarded = if forwarded_to_core { "true" } else { "false" },
            root = Self::format_point(root_position),
            screen = Self::format_point(screen_position),
            capture = native_capture_success
                .map(|success| if success { "true" } else { "false" })
                .unwrap_or("null"),
            release = native_capture_release_success
                .map(|success| if success { "true" } else { "false" })
                .unwrap_or("null"),
        );
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = file.write_all(line.as_bytes());
            let _ = file.flush();
        }
    }

    fn format_point(point: Option<Point>) -> String {
        match point {
            Some(point) => format!("{{\"x\":{},\"y\":{}}}", point.x, point.y),
            None => "null".to_string(),
        }
    }
}

/// Pointer-cancellation bridge for one WinUI3 hosted tree. Both references are weak so the Core
/// root cannot retain either its dispatcher or native Canvas owner.
pub(crate) struct WinUI3PointerGestureHost {
    pointer: Weak<PointerDispatcher>,
    canvas: windows::core::Weak<Canvas>,
}

impl PointerGestureHost for WinUI3PointerGestureHost {
    fn cancel_pointer_gesture_in_subtree(
        &self,
        subtree: &Rc<dyn elwindui_core::ui::UIElementExt>,
    ) -> bool {
        let pointer: Option<Rc<PointerDispatcher>> = self.pointer.upgrade();
        let canceled = pointer.is_some_and(|pointer| pointer.cancel_for_subtree(subtree));
        if canceled {
            if let Some(canvas) = self.canvas.upgrade() {
                let _ = canvas.ReleasePointerCaptures();
            }
        }
        canceled
    }
}

impl TreeHost {
    pub(crate) fn new() -> Self {
        let canvas = accessibility::create_canvas();
        let composition = CompositionRenderer::new(&canvas).expect("CompositionRenderer::new");
        // Keep a native hit-test source permanently attached for blank self-drawn content. This
        // must precede every dynamic projection child so it remains Canvas.Children()[0].
        let input_surface = Self::create_input_surface(&canvas);
        let tree = Rc::new(RefCell::new(None));
        let accessibility = WinUI3AccessibilityState::new(Rc::downgrade(&tree));
        let this = Self {
            canvas,
            input_surface,
            relayout_cycle: Rc::new(RelayoutCycleState::default()),
            composition: Rc::new(RefCell::new(composition)),
            tree,
            render_tree: Rc::new(RefCell::new(None)),
            native_children: Rc::new(RefCell::new(NativeChildMap::new())),
            keyboard: Rc::new(KeyboardDispatcher::new()),
            pointer: Rc::new(PointerDispatcher::new()),
            viewport: Rc::new(Cell::new(None)),
            active: Rc::new(Cell::new(true)),
            active_popup: Rc::new(RefCell::new(None)),
            callback_owner: UiCallbackRegistryOwner::default(),
            animation_runtime: AnimationRuntime::new(),
            rendering: Rc::new(WinUI3RenderingState::default()),
            accessibility,
            relayout_host: Rc::new(RefCell::new(Weak::<WinUI3RelayoutHost>::new())),
        };
        #[cfg(windows)]
        let _ = this.accessibility.bind_canvas(&this.canvas);
        let pointer_trace = PointerTrace::from_environment();
        // WinUI3's `Control.IsTabStop` gate. Once the WinRT event projection is restored this
        // allows the host to receive OS keyboard focus, mirroring AppKit's TreeHost.
        let _ = this.canvas.SetIsTabStop(true);
        {
            let tree_for_key = Rc::downgrade(&this.tree);
            let keyboard_for_key = Rc::downgrade(&this.keyboard);
            let active_for_key = Rc::downgrade(&this.active_popup);
            let canvas_for_context_key = this.canvas.clone();
            let context_callback_id = this.callback_owner.register_bool_event(Rc::new(move || {
                let tree_storage: Option<
                    Rc<RefCell<Option<Rc<dyn elwindui_core::ui::UIElementExt>>>>,
                > = tree_for_key.upgrade();
                let tree_ref: Option<Rc<dyn elwindui_core::ui::UIElementExt>> =
                    tree_storage.and_then(|t| t.borrow().clone());
                let kb_ref: Option<Rc<KeyboardDispatcher>> = keyboard_for_key.upgrade();
                let active_ref: Option<
                    Rc<RefCell<Option<Rc<dyn elwindui_core::ui::popup::PopupSurfaceHandle>>>>,
                > = active_for_key.upgrade();
                if let (Some(tree), Some(kb), Some(active)) = (tree_ref, kb_ref, active_ref) {
                    let screen_anchor = if let Some(focused) = kb.as_ref().focus.focused() {
                        let offset = focused
                            .arranged_offset()
                            .unwrap_or(Point { x: 0.0, y: 0.0 });
                        let w = focused.arranged_width().unwrap_or(0.0);
                        let h = focused.arranged_height().unwrap_or(0.0);
                        Self::canvas_to_screen_point(&canvas_for_context_key, offset).map(
                            |screen_pt| {
                                elwindui_core::ui::popup::PopupAnchor::Rect(
                                    elwindui_core::base::Rect {
                                        x: screen_pt.x,
                                        y: screen_pt.y,
                                        width: w,
                                        height: h,
                                    },
                                )
                            },
                        )
                    } else {
                        None
                    };
                    if let Some(screen_anchor) = screen_anchor {
                        let request =
                            elwindui_core::ui::ContextRequest::keyboard(Some(screen_anchor));
                        return Self::dispatch_context_request(
                            &Some(tree),
                            &kb,
                            &canvas_for_context_key,
                            &active,
                            &request,
                        );
                    }
                }
                false
            }));
            let tree_for_key = Rc::downgrade(&this.tree);
            let keyboard_for_key = Rc::downgrade(&this.keyboard);
            let callback_id = this.callback_owner.register_key(Rc::new(move |event| {
                let tree_storage: Option<
                    Rc<RefCell<Option<Rc<dyn elwindui_core::ui::UIElementExt>>>>,
                > = tree_for_key.upgrade();
                let keyboard: Option<Rc<KeyboardDispatcher>> = keyboard_for_key.upgrade();
                if let (Some(tree), Some(keyboard)) = (tree_storage, keyboard) {
                    let tree: Option<Rc<dyn elwindui_core::ui::UIElementExt>> =
                        tree.borrow().clone();
                    if let Some(tree) = tree {
                        keyboard.handle_key(&tree, event);
                    }
                }
            }));
            let _ = this
                .canvas
                .KeyDown(&KeyEventHandler::new(move |_sender, args| {
                    let Some(args) = args.cloned() else {
                        return Ok(());
                    };
                    let Ok(virtual_key) = args.Key() else {
                        return Ok(());
                    };
                    let Some(key) = winui_key(virtual_key) else {
                        return Ok(());
                    };
                    let is_repeat = args
                        .KeyStatus()
                        .map(|status| status.RepeatCount > 1)
                        .unwrap_or(false);
                    let modifiers = winui_modifiers();
                    if crate::host::event::is_context_menu_key(virtual_key, modifiers) {
                        if invoke_ui_bool_event_callback(context_callback_id) {
                            return Ok(());
                        }
                    }
                    invoke_ui_key_event_callback(
                        callback_id,
                        RawKeyEvent {
                            kind: RawKeyEventKind::Down { is_repeat },
                            key,
                            modifiers,
                            timestamp_ms: 0.0,
                        },
                    );
                    Ok(())
                }));
        }
        {
            let tree_for_key = Rc::downgrade(&this.tree);
            let keyboard_for_key = Rc::downgrade(&this.keyboard);
            let callback_id = this.callback_owner.register_key(Rc::new(move |event| {
                let tree_storage: Option<
                    Rc<RefCell<Option<Rc<dyn elwindui_core::ui::UIElementExt>>>>,
                > = tree_for_key.upgrade();
                let keyboard: Option<Rc<KeyboardDispatcher>> = keyboard_for_key.upgrade();
                if let (Some(tree), Some(keyboard)) = (tree_storage, keyboard) {
                    let tree: Option<Rc<dyn elwindui_core::ui::UIElementExt>> =
                        tree.borrow().clone();
                    if let Some(tree) = tree {
                        keyboard.handle_key(&tree, event);
                    }
                }
            }));
            let _ = this
                .canvas
                .KeyUp(&KeyEventHandler::new(move |_sender, args| {
                    let Some(args) = args.cloned() else {
                        return Ok(());
                    };
                    let Ok(virtual_key) = args.Key() else {
                        return Ok(());
                    };
                    let Some(key) = winui_key(virtual_key) else {
                        return Ok(());
                    };
                    invoke_ui_key_event_callback(
                        callback_id,
                        RawKeyEvent {
                            kind: RawKeyEventKind::Up,
                            key,
                            modifiers: winui_modifiers(),
                            timestamp_ms: 0.0,
                        },
                    );
                    Ok(())
                }));
        }
        {
            let tree_for_text = Rc::downgrade(&this.tree);
            let keyboard_for_text = Rc::downgrade(&this.keyboard);
            let callback_id = this.callback_owner.register_text(Rc::new(move |text| {
                let tree_storage: Option<
                    Rc<RefCell<Option<Rc<dyn elwindui_core::ui::UIElementExt>>>>,
                > = tree_for_text.upgrade();
                let keyboard: Option<Rc<KeyboardDispatcher>> = keyboard_for_text.upgrade();
                if let (Some(tree), Some(keyboard)) = (tree_storage, keyboard) {
                    if let Some(tree) = tree.borrow().clone() {
                        keyboard.handle_text_input(&tree, RawTextInputEvent { text });
                    }
                }
            }));
            let _ = this.canvas.CharacterReceived(&TypedEventHandler::<
                UIElement,
                CharacterReceivedRoutedEventArgs,
            >::new(move |_sender, args| {
                let Some(args) = args.cloned() else {
                    return Ok(());
                };
                let Ok(code_unit) = args.Character() else {
                    return Ok(());
                };
                let Some(ch) = char::from_u32(code_unit as u32) else {
                    return Ok(());
                };
                if !ch.is_control() {
                    invoke_ui_text_event_callback(callback_id, ch.to_string());
                }
                Ok(())
            }));
        }
        {
            let tree = Rc::downgrade(&this.tree);
            let pointer = Rc::downgrade(&this.pointer);
            let keyboard = Rc::downgrade(&this.keyboard);
            let canvas = this.canvas.clone();
            let canvas_for_callback = canvas.clone();
            let input_surface_for_callback = this.input_surface.clone();
            let pointer_trace = pointer_trace.clone();
            let callback_id = this
                .callback_owner
                .register_pointer_event(Rc::new(move |args| {
                    let Some(kind) = Self::pointer_button_kind(args, &canvas_for_callback, true)
                    else {
                        return;
                    };
                    if Self::dispatch_pointer_routed(
                        &tree,
                        &pointer,
                        &keyboard,
                        &canvas_for_callback,
                        &input_surface_for_callback,
                        args,
                        kind,
                        &pointer_trace,
                        "PointerPressed",
                    ) {
                        if let Ok(native_pointer) = args.Pointer() {
                            let capture_success =
                                canvas_for_callback.CapturePointer(&native_pointer).is_ok();
                            let source_classification = Self::pointer_source_classification(
                                &canvas_for_callback,
                                &input_surface_for_callback,
                                args,
                            );
                            pointer_trace.record_capture(
                                PointerTrace::pointer_id(args),
                                source_classification,
                                capture_success,
                            );
                            if capture_success && pointer_trace.release_capture_on_press {
                                let release_success = canvas_for_callback
                                    .ReleasePointerCapture(&native_pointer)
                                    .is_ok();
                                pointer_trace.record_release(
                                    PointerTrace::pointer_id(args),
                                    source_classification,
                                    release_success,
                                );
                            }
                        }
                        let _ = args.SetHandled(true);
                    }
                }));
            let _ = canvas.PointerPressed(&PointerEventHandler::new(move |_sender, args| {
                if let Some(args) = args.as_ref() {
                    invoke_ui_pointer_event_callback(callback_id, args);
                }
                Ok(())
            }));
        }
        {
            let tree = Rc::downgrade(&this.tree);
            let pointer = Rc::downgrade(&this.pointer);
            let keyboard = Rc::downgrade(&this.keyboard);
            let canvas = this.canvas.clone();
            let canvas_for_callback = canvas.clone();
            let input_surface_for_callback = this.input_surface.clone();
            let pointer_trace = pointer_trace.clone();
            let callback_id = this
                .callback_owner
                .register_pointer_event(Rc::new(move |args| {
                    if Self::dispatch_pointer_routed(
                        &tree,
                        &pointer,
                        &keyboard,
                        &canvas_for_callback,
                        &input_surface_for_callback,
                        args,
                        RawPointerEventKind::Moved,
                        &pointer_trace,
                        "PointerMoved",
                    ) {
                        let _ = args.SetHandled(true);
                    }
                }));
            let _ = canvas.PointerMoved(&PointerEventHandler::new(move |_sender, args| {
                if let Some(args) = args.as_ref() {
                    invoke_ui_pointer_event_callback(callback_id, args);
                }
                Ok(())
            }));
        }
        {
            let tree = Rc::downgrade(&this.tree);
            let pointer = Rc::downgrade(&this.pointer);
            let keyboard = Rc::downgrade(&this.keyboard);
            let canvas = this.canvas.clone();
            let canvas_for_callback = canvas.clone();
            let input_surface_for_callback = this.input_surface.clone();
            let pointer_trace = pointer_trace.clone();
            let callback_id = this
                .callback_owner
                .register_pointer_event(Rc::new(move |args| {
                    let Some(kind) = Self::pointer_button_kind(args, &canvas_for_callback, false)
                    else {
                        return;
                    };
                    if Self::dispatch_pointer_routed(
                        &tree,
                        &pointer,
                        &keyboard,
                        &canvas_for_callback,
                        &input_surface_for_callback,
                        args,
                        kind,
                        &pointer_trace,
                        "PointerReleased",
                    ) {
                        if let Ok(native_pointer) = args.Pointer() {
                            let _ = canvas_for_callback.ReleasePointerCapture(&native_pointer);
                        }
                        let _ = args.SetHandled(true);
                    }
                }));
            let _ = canvas.PointerReleased(&PointerEventHandler::new(move |_sender, args| {
                if let Some(args) = args.as_ref() {
                    invoke_ui_pointer_event_callback(callback_id, args);
                }
                Ok(())
            }));
        }
        {
            let tree = Rc::downgrade(&this.tree);
            let pointer = Rc::downgrade(&this.pointer);
            let keyboard = Rc::downgrade(&this.keyboard);
            let canvas = this.canvas.clone();
            let canvas_for_callback = canvas.clone();
            let input_surface_for_callback = this.input_surface.clone();
            let pointer_trace = pointer_trace.clone();
            let callback_id = this
                .callback_owner
                .register_pointer_event(Rc::new(move |args| {
                    if Self::dispatch_pointer_routed(
                        &tree,
                        &pointer,
                        &keyboard,
                        &canvas_for_callback,
                        &input_surface_for_callback,
                        args,
                        RawPointerEventKind::Canceled,
                        &pointer_trace,
                        "PointerCanceled",
                    ) {
                        let source_classification = Self::pointer_source_classification(
                            &canvas_for_callback,
                            &input_surface_for_callback,
                            args,
                        );
                        let release_success = canvas_for_callback.ReleasePointerCaptures().is_ok();
                        pointer_trace.record_release(
                            PointerTrace::pointer_id(args),
                            source_classification,
                            release_success,
                        );
                        let _ = args.SetHandled(true);
                    }
                }));
            let _ = canvas.PointerCanceled(&PointerEventHandler::new(move |_sender, args| {
                if let Some(args) = args.as_ref() {
                    invoke_ui_pointer_event_callback(callback_id, args);
                }
                Ok(())
            }));
        }
        {
            let tree = Rc::downgrade(&this.tree);
            let pointer = Rc::downgrade(&this.pointer);
            let keyboard = Rc::downgrade(&this.keyboard);
            let canvas = this.canvas.clone();
            let canvas_for_callback = canvas.clone();
            let input_surface_for_callback = this.input_surface.clone();
            let pointer_trace = pointer_trace.clone();
            let callback_id = this
                .callback_owner
                .register_pointer_event(Rc::new(move |args| {
                    if Self::dispatch_pointer_routed(
                        &tree,
                        &pointer,
                        &keyboard,
                        &canvas_for_callback,
                        &input_surface_for_callback,
                        args,
                        RawPointerEventKind::Canceled,
                        &pointer_trace,
                        "PointerCaptureLost",
                    ) {
                        let _ = args.SetHandled(true);
                    }
                }));
            let _ = canvas.PointerCaptureLost(&PointerEventHandler::new(move |_sender, args| {
                if let Some(args) = args.as_ref() {
                    invoke_ui_pointer_event_callback(callback_id, args);
                }
                Ok(())
            }));
        }
        // Issue #261 review remediation §2.6: `canvas`'s own `SizeChanged` is deliberately *not*
        // observed here. A `TreeHost` is a viewport *consumer*, never its own viewport producer —
        // see `set_viewport`'s own doc comment for why self-observing a native size notification
        // this host's own layout output produced is a same-host feedback loop, not a legitimate
        // relayout trigger, and how every real viewport authority (`Window`/`TabView`/`ScrollView`/
        // `Popup`) now pushes changes in through `set_viewport` instead.
        {
            let tree_for_context = Rc::downgrade(&this.tree);
            let keyboard_for_context = Rc::downgrade(&this.keyboard);
            let canvas_for_context = this.canvas.clone();
            let active_for_context = Rc::downgrade(&this.active_popup);
            let callback_id = this
                .callback_owner
                .register_right_tapped(Rc::new(move |args| {
                    if args.Handled().unwrap_or(false) {
                        return;
                    }
                    let tree_storage: Option<
                        Rc<RefCell<Option<Rc<dyn elwindui_core::ui::UIElementExt>>>>,
                    > = tree_for_context.upgrade();
                    let keyboard: Option<Rc<KeyboardDispatcher>> = keyboard_for_context.upgrade();
                    let active: Option<
                        Rc<RefCell<Option<Rc<dyn elwindui_core::ui::popup::PopupSurfaceHandle>>>>,
                    > = active_for_context.upgrade();
                    if let (Some(tree), Some(keyboard), Some(active)) =
                        (tree_storage, keyboard, active)
                    {
                        if let Some(tree) = tree.borrow().clone() {
                            let Ok(point) = args.GetPosition(&canvas_for_context) else {
                                return;
                            };
                            let local_pt = elwindui_core::base::Point {
                                x: point.X,
                                y: point.Y,
                            };
                            if let Some(screen_pt) =
                                Self::canvas_to_screen_point(&canvas_for_context, local_pt)
                            {
                                let request =
                                    elwindui_core::ui::ContextRequest::pointer(local_pt, screen_pt);
                                if Self::dispatch_context_request(
                                    &Some(tree),
                                    &keyboard,
                                    &canvas_for_context,
                                    &active,
                                    &request,
                                ) {
                                    let _ = args.SetHandled(true);
                                }
                            }
                        }
                    }
                }));
            let _ = this.canvas.RightTapped(
                &crate::bindings::Microsoft::UI::Xaml::Input::RightTappedEventHandler::new(
                    move |_sender, args| {
                        if let Some(args) = args.as_ref() {
                            invoke_ui_right_tapped_callback(callback_id, args);
                        }
                        Ok(())
                    },
                ),
            );
        }
        {
            let tree_for_ctx = Rc::downgrade(&this.tree);
            let keyboard_for_ctx = Rc::downgrade(&this.keyboard);
            let canvas_for_ctx = this.canvas.clone();
            let active_for_ctx = Rc::downgrade(&this.active_popup);
            let callback_id = this
                .callback_owner
                .register_context_event(Rc::new(move |args| {
                    let mut pt = windows::Foundation::Point::default();
                    let is_pointer = args
                        .TryGetPosition(&canvas_for_ctx, &mut pt)
                        .unwrap_or(false);
                    let request = if is_pointer {
                        let local_pt = elwindui_core::base::Point { x: pt.X, y: pt.Y };
                        TreeHost::canvas_to_screen_point(&canvas_for_ctx, local_pt).map(
                            |screen_pt| {
                                elwindui_core::ui::ContextRequest::pointer(local_pt, screen_pt)
                            },
                        )
                    } else {
                        let keyboard: Option<Rc<KeyboardDispatcher>> = keyboard_for_ctx.upgrade();
                        let screen_anchor = if let Some(keyboard) = keyboard {
                            if let Some(focused) = keyboard.as_ref().focus.focused() {
                                let offset = focused
                                    .arranged_offset()
                                    .unwrap_or(Point { x: 0.0, y: 0.0 });
                                let w = focused.arranged_width().unwrap_or(0.0);
                                let h = focused.arranged_height().unwrap_or(0.0);
                                TreeHost::canvas_to_screen_point(&canvas_for_ctx, offset).map(
                                    |screen_pt| {
                                        elwindui_core::ui::popup::PopupAnchor::Rect(
                                            elwindui_core::base::Rect {
                                                x: screen_pt.x,
                                                y: screen_pt.y,
                                                width: w,
                                                height: h,
                                            },
                                        )
                                    },
                                )
                            } else {
                                None
                            }
                        } else {
                            None
                        };
                        screen_anchor
                            .map(|anchor| elwindui_core::ui::ContextRequest::keyboard(Some(anchor)))
                    };
                    if let Some(request) = request {
                        let tree_storage: Option<
                            Rc<RefCell<Option<Rc<dyn elwindui_core::ui::UIElementExt>>>>,
                        > = tree_for_ctx.upgrade();
                        let keyboard: Option<Rc<KeyboardDispatcher>> = keyboard_for_ctx.upgrade();
                        let active: Option<
                            Rc<
                                RefCell<
                                    Option<Rc<dyn elwindui_core::ui::popup::PopupSurfaceHandle>>,
                                >,
                            >,
                        > = active_for_ctx.upgrade();
                        if let (Some(tree), Some(keyboard), Some(active)) =
                            (tree_storage, keyboard, active)
                        {
                            if let Some(tree) = tree.borrow().clone() {
                                if Self::dispatch_context_request(
                                    &Some(tree),
                                    &keyboard,
                                    &canvas_for_ctx,
                                    &active,
                                    &request,
                                ) {
                                    let _ = args.SetHandled(true);
                                }
                            }
                        }
                    }
                }));
            let _ = this.canvas.ContextRequested(&TypedEventHandler::new(
                move |_sender,
                      args: windows::core::Ref<
                    '_,
                    Microsoft::UI::Xaml::Input::ContextRequestedEventArgs,
                >| {
                    if let Some(args) = args.as_ref() {
                        invoke_ui_context_event_callback(callback_id, args);
                    }
                    Ok(())
                },
            ));
        }
        this
    }

    /// `Canvas` receives bubbled events from native XAML children too. Only the exact root Canvas
    /// or this host's exact input surface belongs to the self-drawn Core tree; native children and
    /// unrelated XAML descendants remain their own input owners.
    fn pointer_originates_from_canvas(
        canvas: &Canvas,
        input_surface: &Rectangle,
        args: &PointerRoutedEventArgs,
    ) -> bool {
        let Ok(source) = args.OriginalSource() else {
            return false;
        };
        Self::is_self_drawn_pointer_source(canvas, input_surface, &source)
    }

    fn pointer_source_classification(
        canvas: &Canvas,
        input_surface: &Rectangle,
        args: &PointerRoutedEventArgs,
    ) -> &'static str {
        let Ok(source) = args.OriginalSource() else {
            return "unknown";
        };
        if let Ok(canvas_source) = source.cast::<Canvas>() {
            if canvas_source == *canvas {
                return "root_canvas";
            }
        }
        if let Ok(surface_source) = source.cast::<Rectangle>() {
            if surface_source == *input_surface {
                return "input_surface";
            }
        }
        "native_xaml_child"
    }

    /// Classifies a routed event by exact native object identity. In particular, accepting any
    /// Rectangle or any XAML descendant would forward native-control input into Core as well.
    fn is_self_drawn_pointer_source(
        canvas: &Canvas,
        input_surface: &Rectangle,
        source: &impl Interface,
    ) -> bool {
        if let Ok(canvas_source) = source.cast::<Canvas>() {
            return canvas_source == *canvas;
        }
        if let Ok(surface_source) = source.cast::<Rectangle>() {
            return surface_source == *input_surface;
        }
        false
    }

    fn pointer_button_kind(
        args: &PointerRoutedEventArgs,
        canvas: &Canvas,
        pressed: bool,
    ) -> Option<RawPointerEventKind> {
        let update = args
            .GetCurrentPoint(canvas)
            .ok()?
            .Properties()
            .ok()?
            .PointerUpdateKind()
            .ok()?;
        let button = match (pressed, update) {
            (true, PointerUpdateKind::LeftButtonPressed)
            | (false, PointerUpdateKind::LeftButtonReleased) => MouseButton::Left,
            (true, PointerUpdateKind::RightButtonPressed)
            | (false, PointerUpdateKind::RightButtonReleased) => MouseButton::Right,
            (true, PointerUpdateKind::MiddleButtonPressed)
            | (false, PointerUpdateKind::MiddleButtonReleased) => MouseButton::Middle,
            _ => return None,
        };
        Some(if pressed {
            RawPointerEventKind::Pressed(button)
        } else {
            RawPointerEventKind::Released(button)
        })
    }

    fn dispatch_pointer_routed(
        tree: &Weak<RefCell<Option<Rc<dyn elwindui_core::ui::UIElementExt>>>>,
        pointer: &Weak<PointerDispatcher>,
        keyboard: &Weak<KeyboardDispatcher>,
        canvas: &Canvas,
        input_surface: &Rectangle,
        args: &PointerRoutedEventArgs,
        kind: RawPointerEventKind,
        trace: &PointerTrace,
        native_event: &str,
    ) -> bool {
        let source_classification =
            Self::pointer_source_classification(canvas, input_surface, args);
        let pointer_id = PointerTrace::pointer_id(args);
        let point = args.GetCurrentPoint(canvas).ok();
        let root_position =
            point
                .as_ref()
                .and_then(|point| point.Position().ok())
                .map(|position| Point {
                    x: position.X,
                    y: position.Y,
                });
        let screen_position =
            root_position.and_then(|position| Self::canvas_to_screen_point(canvas, position));
        let source_is_self_drawn =
            Self::pointer_originates_from_canvas(canvas, input_surface, args);
        let tree_storage: Option<Rc<RefCell<Option<Rc<dyn elwindui_core::ui::UIElementExt>>>>> =
            tree.upgrade();
        let tree = tree_storage.and_then(|tree| tree.borrow().clone());
        let pointer: Option<Rc<PointerDispatcher>> = pointer.upgrade();
        let keyboard: Option<Rc<KeyboardDispatcher>> = keyboard.upgrade();
        let forwarded_to_core = source_is_self_drawn
            && tree.is_some()
            && pointer.is_some()
            && keyboard.is_some()
            && root_position.is_some();
        trace.record_pointer(
            native_event,
            pointer_id,
            source_classification,
            forwarded_to_core,
            root_position,
            screen_position,
        );
        if !forwarded_to_core {
            return false;
        }
        let tree = tree.expect("forwarded pointer event requires a tree");
        let pointer = pointer.expect("forwarded pointer event requires a dispatcher");
        let keyboard = keyboard.expect("forwarded pointer event requires a keyboard");
        let local = root_position.expect("forwarded pointer event requires a point");
        let timestamp_ms = point
            .as_ref()
            .and_then(|point| point.Timestamp().ok())
            .unwrap_or(0) as f64
            / 1000.0;
        let focus = &keyboard.as_ref().focus;
        pointer.handle(
            &tree,
            focus,
            RawPointerEvent {
                kind,
                position: local,
                screen_position,
                modifiers: winui_modifiers(),
                timestamp_ms,
            },
        );
        true
    }

    pub(crate) fn as_element(&self) -> FrameworkElement {
        self.canvas
            .cast()
            .expect("Canvas must be a FrameworkElement")
    }

    pub(crate) fn canvas(&self) -> &Canvas {
        &self.canvas
    }

    fn create_input_surface(canvas: &Canvas) -> Rectangle {
        use crate::bindings::Microsoft::UI::Xaml::Media::SolidColorBrush;
        use windows::UI::Color;

        let surface = Rectangle::new().expect("Shapes::Rectangle::new");
        let fill = SolidColorBrush::new().expect("SolidColorBrush::new");
        fill.SetColor(Color {
            A: 0,
            R: 0,
            G: 0,
            B: 0,
        })
        .expect("SolidColorBrush::SetColor");
        surface.SetFill(&fill).expect("Rectangle::SetFill");

        let surface_ui: UIElement = surface.clone().cast().expect("Rectangle is a UIElement");
        surface_ui
            .SetIsHitTestVisible(true)
            .expect("UIElement::SetIsHitTestVisible");
        Canvas::SetLeft(&surface_ui, 0.0).expect("Canvas::SetLeft");
        Canvas::SetTop(&surface_ui, 0.0).expect("Canvas::SetTop");
        canvas
            .Children()
            .expect("Canvas::Children")
            .Append(&surface_ui)
            .expect("append permanent TreeHost input surface");
        surface
    }

    pub(crate) fn set_transparent_background(&self, transparent: bool) {
        use crate::bindings::Microsoft::UI::Xaml::Media::SolidColorBrush;
        use windows::UI::Color;

        if transparent {
            if let Ok(brush) = SolidColorBrush::new() {
                let _ = brush.SetColor(Color {
                    A: 0,
                    R: 0,
                    G: 0,
                    B: 0,
                });
                let _ = self.canvas.SetBackground(&brush);
            }
        } else if let Ok(property) =
            crate::bindings::Microsoft::UI::Xaml::Controls::Panel::BackgroundProperty()
        {
            let _ = self.canvas.ClearValue(&property);
        }
    }

    /// Shared realization body for a viewport/tree/activation change, parameterized by the
    /// `RelayoutSource` diagnostic tag so each caller (`set_tree`'s initial layout,
    /// `set_active(true)`'s reactivation, `set_viewport`'s explicit push) attributes correctly.
    /// Routed through the currently-installed `WinUI3RelayoutHost`'s own `realize_synchronously`
    /// (supersede queued ticket, consume pending state, run) instead of calling `relayout_static`
    /// directly and bypassing that state machine — without this, a queued dispatcher job already
    /// pending for this host would still fire afterward and run a second, redundant pass.
    fn force_relayout_with_source(&self, source: RelayoutSource) {
        if !self.active.get() {
            if std::env::var_os("ELWINDUI_WINUI3_DIAGNOSTICS").is_some() {
                eprintln!("[elwindui-winui3] skipped relayout for inactive TreeHost");
            }
            return;
        }
        let host: Option<Rc<WinUI3RelayoutHost>> = self.relayout_host.borrow().upgrade();
        if let Some(host) = host {
            host.realize_synchronously(source, elwindui_core::ui::InvalidationKind::Measure);
        } else {
            // No `WinUI3RelayoutHost` installed yet (called before any `set_tree()`) — fall back
            // to the raw pass directly; there is no queued job to supersede in this case.
            // `relayout_static_pass`'s own tree-check makes this a safe, cheap no-op when no tree
            // is attached yet (e.g. `set_viewport` called during popup construction, before
            // `set_tree`).
            Self::relayout_static(
                &self.canvas,
                &self.input_surface,
                &self.composition,
                &self.tree,
                &self.render_tree,
                &self.native_children,
                &self.keyboard,
                self.viewport.get(),
                &self.active,
                &self.relayout_cycle,
                0,
            );
        }
        self.accessibility.rebuild();
    }

    /// The single authority API every viewport *owner* (`Window`/`TabView`/`ScrollView`/`Popup` —
    /// see each type's own doc comment for which one owns which host) must use to push a viewport
    /// change into this host. This is the only source of `layout_root`'s available size (Issue
    /// #261 review remediation §2.2/§2.3) — `relayout_static_pass` never reads `canvas`'s own
    /// `ActualWidth`/`ActualHeight`, and this host never observes its own `canvas.SizeChanged`.
    ///
    /// That last point is load-bearing, not incidental: `canvas.SizeChanged` used to feed directly
    /// back into this same host's own relayout scheduling. On live `docking-demo` (a deeply nested
    /// `Window -> TreeHost -> TabView -> TreeHost -> ...` tree) that created a same-host feedback
    /// cascade — each layout's own presentation-size output re-triggered another layout of the
    /// *same* host, and because the trigger was posted through the async per-turn queue, each step
    /// of what used to be an immediately-resolving native resize cascade could only propagate one
    /// dispatcher turn at a time. Measured: 595+ separate queued cycles and 112.5s+ of cumulative
    /// text-measurement time on one single host before the run was force-stopped. A `TreeHost` must
    /// be a viewport *consumer*, never its own viewport producer — this method, not a native
    /// self-observed size notification, is the only legitimate way this host's layout input changes.
    ///
    /// A negative or non-finite constrained axis is normalized to `0.0` (see `TreeHostViewport::
    /// normalized`). An unchanged effective viewport (post-normalization) is a true no-op: no
    /// native `SetWidth`/`SetHeight` call, no relayout. If this host has no tree attached yet (e.g.
    /// during popup construction, before `set_tree`) or is currently suspended
    /// (`set_active(false)`), the viewport is still stored so the next `set_tree`/
    /// `set_active(true)` picks it up, but no layout work happens now.
    pub(crate) fn set_viewport(&self, viewport: TreeHostViewport) -> windows::core::Result<bool> {
        let viewport = viewport.normalized();
        let element = self.as_element();
        let changed = apply_native_viewport(
            &self.viewport,
            viewport,
            |width| element.SetWidth(width),
            |height| element.SetHeight(height),
        )?;
        if !changed {
            return Ok(false);
        }
        self.force_relayout_with_source(RelayoutSource::ViewportSync);
        Ok(true)
    }

    /// Activates or suspends this host's layout and rendering lifecycle.
    ///
    /// Suspending removes retained Composition and native children, clears focus, and turns later
    /// invalidations and forced layouts into no-ops. Reactivating performs one full relayout from
    /// the retained logical tree, so callers do not need to replay changes made while inactive.
    pub(crate) fn set_active(&self, active: bool) {
        if self.active.replace(active) == active {
            return;
        }
        if std::env::var_os("ELWINDUI_WINUI3_DIAGNOSTICS").is_some() {
            eprintln!("[elwindui-winui3] TreeHost active={active}");
        }
        if active {
            let canvas_ui: UIElement = self.canvas.clone().cast().expect("Canvas is a UIElement");
            let _ = canvas_ui.SetIsHitTestVisible(true);
            self.force_relayout_with_source(RelayoutSource::SetActiveReactivate);
            return;
        }

        self.accessibility.clear();
        if self.pointer.cancel() {
            let _ = self.canvas.ReleasePointerCaptures();
        }
        let canvas_ui: UIElement = self.canvas.clone().cast().expect("Canvas is a UIElement");
        let _ = canvas_ui.SetIsHitTestVisible(false);
        self.rendering.stop();
        self.keyboard.as_ref().focus.clear_focus();
        let _ = self
            .composition
            .borrow_mut()
            .reconcile(&self.canvas, Vec::new());
        reconcile_native_children(
            &self.canvas,
            &self.native_children,
            Vec::new(),
            &self.render_tree,
            &self.keyboard,
            None,
        );
        *self.render_tree.borrow_mut() = None;
    }

    /// Replaces this host's entire content. Both composition islands and `Text`/`NativeControl`
    /// children are reconciled by `relayout_static` rather than via `Children.Clear()`: a genuinely
    /// new tree's `RenderGroup` ids never match the old tree's, so the diff naturally tears down
    /// every old child and builds every new one on its own.
    pub(crate) fn set_tree(&self, tree: Rc<dyn elwindui_core::ui::UIElementExt>) {
        self.cancel_and_unregister_current_tree();
        let host = Rc::new(WinUI3RelayoutHost {
            diagnostic_id: NEXT_RELAYOUT_HOST_DIAGNOSTIC_ID.fetch_add(1, Ordering::Relaxed),
            canvas: self.canvas.clone(),
            input_surface: self.input_surface.clone(),
            composition: Rc::downgrade(&self.composition),
            tree: Rc::downgrade(&self.tree),
            render_tree: Rc::downgrade(&self.render_tree),
            native_children: Rc::downgrade(&self.native_children),
            keyboard: Rc::downgrade(&self.keyboard),
            viewport: Rc::downgrade(&self.viewport),
            active: Rc::downgrade(&self.active),
            relayout_cycle: Rc::downgrade(&self.relayout_cycle),
            pending: Cell::new(false),
            pending_kind: Cell::new(elwindui_core::ui::InvalidationKind::default()),
            pending_source: Cell::new(RelayoutSource::SetTreeInitial),
            queue_ticket: Cell::new(0),
            in_progress: Cell::new(false),
            callback_owner: UiCallbackRegistryOwner::default(),
            weak_self: RefCell::new(Weak::<WinUI3RelayoutHost>::new()),
            animation_runtime: Rc::downgrade(&self.animation_runtime),
            rendering: Rc::downgrade(&self.rendering),
            accessibility: Rc::downgrade(&self.accessibility),
        });
        *host.weak_self.borrow_mut() = Rc::downgrade(&host);
        *self.relayout_host.borrow_mut() = Rc::downgrade(&host);
        self.rendering.set_host(Rc::downgrade(&host));
        tree.as_ui_element()
            .set_invalidate_host(Some(Rc::clone(&host) as Rc<dyn RelayoutHost>));
        tree.as_ui_element().set_animation_frame_host(Some(host));
        tree.as_ui_element()
            .set_coordinate_host(Some(Rc::new(WinUI3CoordinateHost {
                canvas: self
                    .canvas
                    .downgrade()
                    .unwrap_or_else(|_| windows::core::Weak::new()),
            })));
        tree.as_ui_element()
            .set_pointer_gesture_host(Some(Rc::new(WinUI3PointerGestureHost {
                pointer: Rc::downgrade(&self.pointer),
                canvas: self
                    .canvas
                    .downgrade()
                    .unwrap_or_else(|_| windows::core::Weak::new()),
            })));
        tree.as_ui_element()
            .set_focus_host(Some(Rc::new(WinUI3FocusHost {
                keyboard: Rc::downgrade(&self.keyboard),
                native_children: Rc::downgrade(&self.native_children),
            })));
        tree.as_ui_element()
            .set_accessibility_host(Some(Rc::new(WinUI3AccessibilityHost::new(Rc::downgrade(
                &self.accessibility,
            )))));
        self.keyboard.as_ref().focus.clear_focus();
        self.keyboard.shortcuts().clear();
        self.keyboard.shortcuts().collect_from_tree(&tree);
        *self.tree.borrow_mut() = Some(tree);
        *self.render_tree.borrow_mut() = None;
        self.force_relayout_with_source(RelayoutSource::SetTreeInitial);
    }

    /// Clears this host's tree, cleans up native composition and children, and closes any active popup.
    pub(crate) fn clear_tree(&self) {
        self.cancel_and_unregister_current_tree();
        if let Some(old) = self.active_popup.borrow_mut().take() {
            old.close();
        }
        self.keyboard.as_ref().focus.clear_focus();
        self.keyboard.shortcuts().clear();
        let _ = self
            .composition
            .borrow_mut()
            .reconcile(&self.canvas, Vec::new());
        reconcile_native_children(
            &self.canvas,
            &self.native_children,
            Vec::new(),
            &self.render_tree,
            &self.keyboard,
            None,
        );
        *self.tree.borrow_mut() = None;
        *self.render_tree.borrow_mut() = None;
    }

    fn cancel_and_unregister_current_tree(&self) {
        self.rendering.clear_host();
        let old_tree = self.tree.borrow().clone();
        if let Some(old_tree) = old_tree {
            if self.pointer.cancel_for_subtree(&old_tree) {
                let _ = self.canvas.ReleasePointerCaptures();
            }
            old_tree.set_invalidate_host(None);
            old_tree.set_coordinate_host(None);
            old_tree.set_pointer_gesture_host(None);
            old_tree.set_animation_frame_host(None);
            old_tree.set_focus_host(None);
            old_tree.set_accessibility_host(None);
        }
        self.accessibility.clear();
    }

    /// Issue #162 §3.18: closes this host's own active custom popup/context-menu surface, if any —
    /// see `close_active_popup_slot`'s own doc comment for the reentrancy-safety reasoning. Shared
    /// by the existing request-replacement paths above and the owner `Window::unmount_override`
    /// path (`native_ui::window.rs`).
    pub(crate) fn close_active_popup(&self) {
        close_active_popup_slot(&self.active_popup);
    }

    /// Focuses the specified element within this host's focus tracker.
    pub(crate) fn focus_element(&self, element: &Rc<dyn elwindui_core::ui::UIElementExt>) {
        self.keyboard
            .focus
            .set_focus(element, elwindui_core::input::FocusState::Programmatic);
    }

    /// Issue #231/#235 review remediation: `request_relayout`'s own `pending` coalescing (see
    /// `WinUI3RelayoutHost`'s doc comment) clears `pending` and calls this function directly,
    /// synchronously, on the calling thread — there is no `DispatcherQueue` deferral here at all —
    /// so a `set_attached`/structural change made by a control while this very host's pass is already
    /// measuring/arranging it (e.g. a `Control`'s `on_apply_template` reconciling its own template
    /// children, or `visual_collection.add`/`remove` invalidating measure) re-enters this
    /// function synchronously instead of the pass already running here picking it up. Left
    /// unguarded, each reentry starts a brand-new full-tree pass nested on top of the still-running
    /// one, and none of them are tail calls, so a moderately nested tree overflows the stack
    /// (observed on `custom-controls-demo`, whose templated `CustomTabView`/
    /// `CustomTabContentPresenter` both reconcile structural children as part of being measured).
    ///
    /// `relayout_cycle` (this exact host's own `RelayoutCycleState`, never a thread-local — see
    /// its own doc comment) turns a same-host reentrant call into a queued rerun instead of either
    /// a recursive call or a silently dropped request: the invalidated subtree may already have
    /// been traversed by the outer pass, so the request cannot simply be discarded, but recursing
    /// is what overflows the stack. `relayout_static_pass` (this method's actual traversal body)
    /// runs in a loop, re-running once more whenever a same-host reentry arrived during the
    /// previous iteration, until a full pass leaves no rerun pending.
    fn relayout_static(
        canvas: &Canvas,
        input_surface: &Rectangle,
        composition: &Rc<RefCell<CompositionRenderer>>,
        tree: &Rc<RefCell<Option<Rc<dyn elwindui_core::ui::UIElementExt>>>>,
        retained_tree: &Rc<RefCell<Option<elwindui_core::graphics::RenderTree>>>,
        native_children: &Rc<RefCell<NativeChildMap>>,
        keyboard: &Rc<KeyboardDispatcher>,
        viewport: Option<TreeHostViewport>,
        active: &Cell<bool>,
        relayout_cycle: &RelayoutCycleState,
        diagnostic_id: u64,
    ) -> bool {
        if !active.get() {
            return false;
        }
        relayout_cycle.run_coalesced(|| {
            Self::relayout_static_pass(
                canvas,
                input_surface,
                composition,
                tree,
                retained_tree,
                native_children,
                keyboard,
                viewport,
                diagnostic_id,
            );
        })
    }

    /// The actual measure/arrange/composition-reconcile traversal for one relayout pass. Never
    /// call directly — go through `relayout_static`, which owns this host's reentrancy/rerun
    /// coalescing (see that method's own doc comment).
    fn relayout_static_pass(
        canvas: &Canvas,
        input_surface: &Rectangle,
        composition: &Rc<RefCell<CompositionRenderer>>,
        tree: &Rc<RefCell<Option<Rc<dyn elwindui_core::ui::UIElementExt>>>>,
        retained_tree: &Rc<RefCell<Option<elwindui_core::graphics::RenderTree>>>,
        native_children: &Rc<RefCell<NativeChildMap>>,
        keyboard: &Rc<KeyboardDispatcher>,
        viewport: Option<TreeHostViewport>,
        diagnostic_id: u64,
    ) {
        #[cfg(test)]
        RELAYOUT_STATIC_PASS_COUNT.with(|count| count.set(count.get() + 1));
        use elwindui_core::base::Size as LSize;

        // Issue #261 review remediation §2.2: the stored viewport (supplied exclusively by this
        // host's owner via `set_viewport`) is the *only* source of available size — never
        // `canvas.Width`/`Height`/`ActualWidth`/`ActualHeight`. No viewport yet (e.g. a tree
        // attached before its owner has ever called `set_viewport`) means there is nothing valid
        // to measure against yet; wait for the owner rather than guessing `0x0`.
        let Some(viewport) = viewport else {
            return;
        };
        let unconstrained_width = viewport.width.is_none();
        let unconstrained_height = viewport.height.is_none();
        let available = LSize {
            width: viewport.width.map(|w| w as f32).unwrap_or(f32::INFINITY),
            height: viewport.height.map(|h| h as f32).unwrap_or(f32::INFINITY),
        };

        let tree_ref = tree.borrow();
        let Some(tree) = tree_ref.as_ref() else {
            return;
        };
        elwindui_core::ui::layout_root(tree, available);
        // Grows `canvas`'s native *presentation* size to the resulting natural size on any
        // unconstrained axis — the WinUI3-side counterpart of AppKit's own post-`layout_root`
        // `setFrame` in `TreeHost::relayout`. This is presentation output only: it must never
        // be read back as this host's own layout input (that self-observation is exactly the
        // same-host feedback loop `set_viewport`'s own doc comment describes) — this host has no
        // `canvas.SizeChanged` listener, so it structurally cannot be.
        let final_width = if unconstrained_width {
            tree.arranged_width().unwrap_or(0.0) as f64
        } else {
            available.width as f64
        };
        let final_height = if unconstrained_height {
            tree.arranged_height().unwrap_or(0.0) as f64
        } else {
            available.height as f64
        };
        let final_width = if final_width.is_finite() {
            final_width.max(0.0)
        } else {
            0.0
        };
        let final_height = if final_height.is_finite() {
            final_height.max(0.0)
        } else {
            0.0
        };
        if unconstrained_width || unconstrained_height {
            let _ = canvas.SetWidth(final_width);
            let _ = canvas.SetHeight(final_height);
        }
        // The permanent input surface is presentation output from this final Core layout pass.
        // It never feeds a viewport back into this host and is always kept at the root origin.
        let input_surface_element: FrameworkElement = input_surface
            .clone()
            .cast()
            .expect("Rectangle is a FrameworkElement");
        let _ = input_surface_element.SetWidth(final_width);
        let _ = input_surface_element.SetHeight(final_height);
        let _ = Canvas::SetLeft(&input_surface_element, 0.0);
        let _ = Canvas::SetTop(&input_surface_element, 0.0);
        {
            let mut retained_tree = retained_tree.borrow_mut();
            if retained_tree
                .as_ref()
                .is_some_and(|render_tree| render_tree.root_id() == tree.render_group_id())
            {
                retained_tree
                    .as_mut()
                    .expect("checked above")
                    .reconcile::<AnyView>(tree);
            } else {
                *retained_tree = Some(elwindui_core::graphics::RenderTree::new::<AnyView>(tree));
            }
        }
        // Named distinctly from the `retained_tree: &Rc<RefCell<Option<RenderTree>>>` parameter
        // (not shadowed, unlike the `borrow_mut()` above whose own shadow already goes out of scope
        // at the end of its block) so that parameter is still reachable below, to pass into
        // `reconcile_native_children`.
        let retained_tree_ref = retained_tree.borrow();
        let Some(render_tree) = retained_tree_ref.as_ref() else {
            return;
        };

        let mut transforms = vec![elwindui_core::base::AffineTransform::identity()];
        let mut opacities = vec![1.0_f32];

        // Keyed by `(group.id, index within that group's own commands)` — see `NativeChildKey`'s
        // doc comment — so `reconcile_native_children` can tell a `Text`/`NativeControl` command
        // that's merely being updated in place apart from one that's genuinely new or gone,
        // without ever needing to `Clear()`/rebuild `canvas.Children()` wholesale.
        fn collect_commands<'a>(
            group: &'a elwindui_core::graphics::RenderGroup,
            origin: elwindui_core::base::Point,
            parent_transform: elwindui_core::base::AffineTransform,
            parent_opacity: f32,
            parent_input_enabled: bool,
            out: &mut Vec<(
                u64,
                usize,
                &'a elwindui_core::graphics::RenderCommand,
                elwindui_core::base::Point,
                elwindui_core::base::AffineTransform,
                f32,
                bool,
            )>,
        ) {
            let origin = elwindui_core::base::Point {
                x: origin.x + group.offset.x,
                y: origin.y + group.offset.y,
            };
            let transform = parent_transform.concat(&group.transform);
            let opacity = parent_opacity * group.opacity;
            let input_enabled = parent_input_enabled && group.input_enabled;
            for (index, command) in group.commands.iter().enumerate() {
                out.push((
                    group.id,
                    index,
                    command,
                    origin,
                    transform,
                    opacity,
                    input_enabled,
                ));
            }
            for child in &group.children {
                collect_commands(child, origin, transform, opacity, input_enabled, out);
            }
        }
        let mut commands = Vec::new();
        collect_commands(
            &render_tree.root,
            elwindui_core::base::Point { x: 0.0, y: 0.0 },
            elwindui_core::base::AffineTransform::identity(),
            1.0,
            true,
            &mut commands,
        );
        let mut native_wanted: Vec<(NativeChildKey, RenderedNativeChild)> = Vec::new();
        let mut composition_islands = Vec::<DesiredCompositionIsland>::new();
        let mut composition_nodes = Vec::<DesiredCompositionNode>::new();
        let mut layer_order = Vec::<RenderLayerKey>::new();
        let mut clip_stack = Vec::<CompositionClipSpec>::new();

        fn flush_composition_island(
            nodes: &mut Vec<DesiredCompositionNode>,
            islands: &mut Vec<DesiredCompositionIsland>,
            order: &mut Vec<RenderLayerKey>,
            clips: &[CompositionClipSpec],
        ) {
            if let Some(island) =
                DesiredCompositionIsland::from_nodes(std::mem::take(nodes), clips.to_vec())
            {
                order.push(RenderLayerKey::Composition(island.id));
                islands.push(island);
            }
        }

        // Composition handles every custom-drawn node. XAML controls and text remain normal
        // children of the host Canvas and are reconciled in place afterward.
        for (
            group_id,
            command_index,
            command,
            origin,
            group_transform,
            group_opacity,
            group_input_enabled,
        ) in commands
        {
            transforms.clear();
            transforms.push(group_transform);
            opacities.clear();
            opacities.push(group_opacity);
            match command {
                elwindui_core::graphics::RenderCommand::PushTransform { transform } => {
                    let next = transforms
                        .last()
                        .expect("transform stack")
                        .concat(transform);
                    transforms.push(next);
                    continue;
                }
                elwindui_core::graphics::RenderCommand::PopTransform => {
                    if transforms.len() > 1 {
                        transforms.pop();
                    }
                    continue;
                }
                elwindui_core::graphics::RenderCommand::PushOpacity { opacity } => {
                    let next = opacities.last().expect("opacity stack") * opacity;
                    opacities.push(next);
                    continue;
                }
                elwindui_core::graphics::RenderCommand::PushClip { clip } => {
                    flush_composition_island(
                        &mut composition_nodes,
                        &mut composition_islands,
                        &mut layer_order,
                        &clip_stack,
                    );
                    let transform = *transforms.last().expect("transform stack");
                    let spec = match clip {
                        elwindui_core::graphics::Clip::Rect(rect) => CompositionClipSpec::Rect {
                            rect: elwindui_core::base::Rect {
                                x: origin.x + rect.x,
                                y: origin.y + rect.y,
                                width: rect.width,
                                height: rect.height,
                            },
                            transform,
                        },
                        elwindui_core::graphics::Clip::RoundedRect { rect, radii } => {
                            CompositionClipSpec::RoundedRect {
                                rect: elwindui_core::base::Rect {
                                    x: origin.x + rect.x,
                                    y: origin.y + rect.y,
                                    width: rect.width,
                                    height: rect.height,
                                },
                                radii: *radii,
                                transform,
                            }
                        }
                        elwindui_core::graphics::Clip::Path { path, rule } => {
                            CompositionClipSpec::Path {
                                commands: path.commands().to_vec(),
                                rule: *rule,
                                origin,
                                transform,
                            }
                        }
                    };
                    clip_stack.push(spec);
                    continue;
                }
                elwindui_core::graphics::RenderCommand::PopClip => {
                    flush_composition_island(
                        &mut composition_nodes,
                        &mut composition_islands,
                        &mut layer_order,
                        &clip_stack,
                    );
                    clip_stack.pop();
                    continue;
                }
                elwindui_core::graphics::RenderCommand::PopOpacity => {
                    if opacities.len() > 1 {
                        opacities.pop();
                    }
                    continue;
                }
                _ => {}
            }

            let node_id = (group_id, command_index);
            let transform = *transforms.last().expect("transform stack");
            let opacity = *opacities.last().expect("opacity stack");
            let absolute_rect = |rect: &elwindui_core::base::Rect| elwindui_core::base::Rect {
                x: origin.x + rect.x,
                y: origin.y + rect.y,
                width: rect.width,
                height: rect.height,
            };
            // The active clip is applied at the island root. Adjacent commands are flushed at
            // every clip boundary, so each node belongs to the innermost active clip island and
            // remains a retained Composition primitive. General intersecting nested clips still
            // need a dedicated nested-container representation before this can preserve arbitrary
            // overlapping clip regions without a surface fallback.
            let fallback_if_clipped = |primitive: CompositionPrimitive| primitive;

            let composition_node = match command {
                elwindui_core::graphics::RenderCommand::FillRect { rect, brush } => {
                    Some(DesiredCompositionNode {
                        id: node_id,
                        primitive: fallback_if_clipped(CompositionPrimitive::Rectangle {
                            rect: absolute_rect(rect),
                        }),
                        fill: Some(brush.clone()),
                        stroke: None,
                        transform,
                        opacity,
                    })
                }
                elwindui_core::graphics::RenderCommand::StrokeRect {
                    rect,
                    brush,
                    stroke,
                } => Some(DesiredCompositionNode {
                    id: node_id,
                    primitive: fallback_if_clipped(CompositionPrimitive::Rectangle {
                        rect: absolute_rect(rect),
                    }),
                    fill: None,
                    stroke: Some((brush.clone(), stroke.clone())),
                    transform,
                    opacity,
                }),
                elwindui_core::graphics::RenderCommand::FillRoundedRect { rect, radii, brush } => {
                    Some(DesiredCompositionNode {
                        id: node_id,
                        primitive: fallback_if_clipped(CompositionPrimitive::RoundedRectangle {
                            rect: absolute_rect(rect),
                            radii: *radii,
                        }),
                        fill: Some(brush.clone()),
                        stroke: None,
                        transform,
                        opacity,
                    })
                }
                elwindui_core::graphics::RenderCommand::StrokeRoundedRect {
                    rect,
                    radii,
                    brush,
                    stroke,
                } => Some(DesiredCompositionNode {
                    id: node_id,
                    primitive: fallback_if_clipped(CompositionPrimitive::RoundedRectangle {
                        rect: absolute_rect(rect),
                        radii: *radii,
                    }),
                    fill: None,
                    stroke: Some((brush.clone(), stroke.clone())),
                    transform,
                    opacity,
                }),
                elwindui_core::graphics::RenderCommand::FillEllipse { rect, brush } => {
                    Some(DesiredCompositionNode {
                        id: node_id,
                        primitive: fallback_if_clipped(CompositionPrimitive::Ellipse {
                            rect: absolute_rect(rect),
                        }),
                        fill: Some(brush.clone()),
                        stroke: None,
                        transform,
                        opacity,
                    })
                }
                elwindui_core::graphics::RenderCommand::StrokeEllipse {
                    rect,
                    brush,
                    stroke,
                } => Some(DesiredCompositionNode {
                    id: node_id,
                    primitive: fallback_if_clipped(CompositionPrimitive::Ellipse {
                        rect: absolute_rect(rect),
                    }),
                    fill: None,
                    stroke: Some((brush.clone(), stroke.clone())),
                    transform,
                    opacity,
                }),
                elwindui_core::graphics::RenderCommand::DrawLine {
                    from,
                    to,
                    brush,
                    stroke,
                } => Some(DesiredCompositionNode {
                    id: node_id,
                    primitive: fallback_if_clipped(CompositionPrimitive::Line {
                        from: elwindui_core::base::Point {
                            x: origin.x + from.x,
                            y: origin.y + from.y,
                        },
                        to: elwindui_core::base::Point {
                            x: origin.x + to.x,
                            y: origin.y + to.y,
                        },
                    }),
                    fill: None,
                    stroke: Some((brush.clone(), stroke.clone())),
                    transform,
                    opacity,
                }),
                elwindui_core::graphics::RenderCommand::FillPath { path, brush, rule } => {
                    Some(DesiredCompositionNode {
                        id: node_id,
                        primitive: fallback_if_clipped(CompositionPrimitive::Path {
                            commands: path.commands().to_vec(),
                            rule: *rule,
                            origin,
                        }),
                        fill: Some(brush.clone()),
                        stroke: None,
                        transform,
                        opacity,
                    })
                }
                elwindui_core::graphics::RenderCommand::StrokePath {
                    path,
                    brush,
                    stroke,
                } => Some(DesiredCompositionNode {
                    id: node_id,
                    primitive: fallback_if_clipped(CompositionPrimitive::Path {
                        commands: path.commands().to_vec(),
                        rule: elwindui_core::graphics::FillRule::NonZero,
                        origin,
                    }),
                    fill: None,
                    stroke: Some((brush.clone(), stroke.clone())),
                    transform,
                    opacity,
                }),
                elwindui_core::graphics::RenderCommand::DrawImage {
                    image,
                    dest,
                    source,
                    options,
                } => Some(DesiredCompositionNode {
                    id: node_id,
                    primitive: fallback_if_clipped(CompositionPrimitive::Rectangle {
                        rect: absolute_rect(dest),
                    }),
                    fill: Some(elwindui_core::graphics::Brush::Image(
                        elwindui_core::graphics::ImageBrush {
                            image: image.clone(),
                            source_rect: *source,
                            stretch: match options.fit {
                                elwindui_core::graphics::ImageFit::Fill => {
                                    elwindui_core::graphics::Stretch::Fill
                                }
                                elwindui_core::graphics::ImageFit::Contain => {
                                    elwindui_core::graphics::Stretch::Uniform
                                }
                                elwindui_core::graphics::ImageFit::Cover => {
                                    elwindui_core::graphics::Stretch::UniformToFill
                                }
                                elwindui_core::graphics::ImageFit::None => {
                                    elwindui_core::graphics::Stretch::None
                                }
                            },
                            alignment_x: options.alignment_x,
                            alignment_y: options.alignment_y,
                            tile_mode: options.repeat,
                            opacity: options.opacity,
                            transform: elwindui_core::base::AffineTransform::IDENTITY,
                        },
                    )),
                    stroke: None,
                    transform,
                    opacity,
                }),
                elwindui_core::graphics::RenderCommand::DrawVectorImage {
                    image,
                    dest,
                    source,
                    options,
                } => Some(DesiredCompositionNode {
                    id: node_id,
                    primitive: CompositionPrimitive::VectorImage {
                        image: image.clone(),
                        dest: absolute_rect(dest),
                        source: *source,
                        options: *options,
                    },
                    fill: None,
                    stroke: None,
                    transform,
                    opacity,
                }),
                elwindui_core::graphics::RenderCommand::Text { .. } => {
                    flush_composition_island(
                        &mut composition_nodes,
                        &mut composition_islands,
                        &mut layer_order,
                        &clip_stack,
                    );
                    layer_order.push(RenderLayerKey::Native(node_id));
                    None
                }
                elwindui_core::graphics::RenderCommand::NativeControl { handle, .. } => {
                    flush_composition_island(
                        &mut composition_nodes,
                        &mut composition_islands,
                        &mut layer_order,
                        &clip_stack,
                    );
                    if handle.downcast_ref::<AnyView>().is_some() {
                        layer_order.push(RenderLayerKey::Native(node_id));
                    }
                    None
                }
                elwindui_core::graphics::RenderCommand::PushClip { .. }
                | elwindui_core::graphics::RenderCommand::PopClip
                | elwindui_core::graphics::RenderCommand::PushTransform { .. }
                | elwindui_core::graphics::RenderCommand::PopTransform
                | elwindui_core::graphics::RenderCommand::PushOpacity { .. }
                | elwindui_core::graphics::RenderCommand::PopOpacity => None,
            };
            if let Some(node) = composition_node {
                composition_nodes.push(node);
            }

            match command {
                elwindui_core::graphics::RenderCommand::Text {
                    content,
                    rect,
                    style,
                    foreground,
                    alignment,
                } => {
                    native_wanted.push((
                        (group_id, command_index),
                        RenderedNativeChild::Text {
                            content: content.clone(),
                            rect: elwindui_core::base::Rect {
                                x: origin.x + rect.x,
                                y: origin.y + rect.y,
                                width: rect.width,
                                height: rect.height,
                            },
                            style: style.clone(),
                            foreground: foreground.clone(),
                            alignment: *alignment,
                            transform,
                            opacity,
                        },
                    ));
                }
                elwindui_core::graphics::RenderCommand::NativeControl { handle, rect, .. } => {
                    if let Some(view) = handle.downcast_ref::<AnyView>().cloned() {
                        native_wanted.push((
                            (group_id, command_index),
                            RenderedNativeChild::Native {
                                view,
                                rect: elwindui_core::base::Rect {
                                    x: origin.x + rect.x,
                                    y: origin.y + rect.y,
                                    width: rect.width,
                                    height: rect.height,
                                },
                                transform,
                                opacity,
                                input_enabled: group_input_enabled,
                            },
                        ));
                    }
                }
                _ => {}
            }
        }
        flush_composition_island(
            &mut composition_nodes,
            &mut composition_islands,
            &mut layer_order,
            &clip_stack,
        );
        let composition_hosts = match composition
            .borrow_mut()
            .reconcile(canvas, composition_islands)
        {
            Ok((hosts, unsupported)) => {
                for unsupported in unsupported {
                    eprintln!(
                        "elwindui-winui3: render node {:?} routed to surface fallback: {}",
                        unsupported.id, unsupported.reason
                    );
                }
                hosts.into_iter().collect::<HashMap<IslandId, UIElement>>()
            }
            Err(error) => {
                eprintln!("elwindui-winui3: Composition reconciliation failed: {error}");
                HashMap::new()
            }
        };
        // Loaded handlers can resolve synchronously from the append below. Drop the retained
        // RenderTree borrow before entering reconciliation so the barrier's Core invalidation can
        // mark that tree dirty without colliding with this replay's immutable borrow.
        drop(retained_tree_ref);
        reconcile_native_children(
            canvas,
            native_children,
            native_wanted,
            retained_tree,
            keyboard,
            Some(diagnostic_id),
        );
        {
            let native_children = native_children.borrow();
            for (z, layer) in layer_order.into_iter().enumerate() {
                let element = match layer {
                    RenderLayerKey::Composition(id) => composition_hosts.get(&id).cloned(),
                    RenderLayerKey::Native(id) => native_children
                        .get(&id)
                        .and_then(|child| child.framework_element().cast::<UIElement>().ok()),
                };
                if let Some(element) = element {
                    let _ = Canvas::SetZIndex(&element, z as i32);
                }
            }
        }
    }

    /// Converts canvas-local logical DIPs to desktop screen physical coordinates.
    ///
    /// The XAML root/content transform and ContentCoordinateConverter are the only source of
    /// screen origin information. In particular, this must not approximate the title-bar or
    /// window-decoration offset from AppWindow position.
    pub(crate) fn canvas_to_screen_physical_point(
        canvas: &crate::bindings::Microsoft::UI::Xaml::Controls::Canvas,
        canvas_point: Point,
    ) -> Option<Point> {
        let xaml_root = canvas.XamlRoot().ok()?;
        let content = xaml_root.Content().ok()?;
        let uie: crate::bindings::Microsoft::UI::Xaml::UIElement = canvas.cast().ok()?;
        let transform = uie.TransformToVisual(&content).ok()?;
        let pt = windows::Foundation::Point {
            X: canvas_point.x,
            Y: canvas_point.y,
        };
        let local_dip = transform.TransformPoint(pt).ok()?;

        let island = xaml_root.ContentIslandEnvironment().ok()?;
        let app_window_id = island.AppWindowId().ok()?;
        let converter =
            crate::bindings::Microsoft::UI::Content::ContentCoordinateConverter::CreateForWindowId(
                app_window_id,
            )
            .ok()?;
        let screen_phys = converter.ConvertLocalToScreenWithPoint(local_dip).ok()?;
        Some(Point {
            x: screen_phys.X as f32,
            y: screen_phys.Y as f32,
        })
    }

    /// Converts canvas-local logical DIPs to desktop screen logical coordinates.
    pub(crate) fn canvas_to_screen_point(
        canvas: &crate::bindings::Microsoft::UI::Xaml::Controls::Canvas,
        canvas_point: Point,
    ) -> Option<Point> {
        let scale = canvas.XamlRoot().ok()?.RasterizationScale().unwrap_or(1.0);
        let scale = if scale <= 0.0 { 1.0 } else { scale };
        let physical = Self::canvas_to_screen_physical_point(canvas, canvas_point)?;
        Some(Point {
            x: physical.x / scale as f32,
            y: physical.y / scale as f32,
        })
    }

    /// Converts normalized screen logical coordinates to this Canvas's own root-local DIPs.
    pub(crate) fn screen_to_canvas_point(
        canvas: &crate::bindings::Microsoft::UI::Xaml::Controls::Canvas,
        screen_point: Point,
    ) -> Option<Point> {
        let xaml_root = canvas.XamlRoot().ok()?;
        let content = xaml_root.Content().ok()?;
        let xaml_local = Self::screen_logical_to_xaml_local(canvas, screen_point)?;
        let canvas_element: crate::bindings::Microsoft::UI::Xaml::UIElement = canvas.cast().ok()?;
        let transform = content.TransformToVisual(&canvas_element).ok()?;
        let canvas_local = transform
            .TransformPoint(windows::Foundation::Point {
                X: xaml_local.x,
                Y: xaml_local.y,
            })
            .ok()?;
        Some(Point {
            x: canvas_local.X,
            y: canvas_local.Y,
        })
    }

    /// Converts desktop screen logical coordinates to XAML root local DIPs.
    pub(crate) fn screen_logical_to_xaml_local(
        canvas: &crate::bindings::Microsoft::UI::Xaml::Controls::Canvas,
        screen_point: Point,
    ) -> Option<Point> {
        let xaml_root = canvas.XamlRoot().ok()?;
        let scale = xaml_root.RasterizationScale().unwrap_or(1.0);
        let scale_safe = if scale <= 0.0 { 1.0 } else { scale };
        let screen_phys = windows::Graphics::PointInt32 {
            X: (screen_point.x as f64 * scale_safe) as i32,
            Y: (screen_point.y as f64 * scale_safe) as i32,
        };

        if let Ok(island) = xaml_root.ContentIslandEnvironment() {
            if let Ok(app_window_id) = island.AppWindowId() {
                if let Ok(converter) = crate::bindings::Microsoft::UI::Content::ContentCoordinateConverter::CreateForWindowId(app_window_id) {
                    if let Ok(local_dip) = converter.ConvertScreenToLocalWithPoint(screen_phys) {
                        return Some(Point {
                            x: local_dip.X,
                            y: local_dip.Y,
                        });
                    }
                }
                if let Ok(app_window) =
                    crate::bindings::Microsoft::UI::Windowing::AppWindow::GetFromWindowId(
                        app_window_id,
                    )
                {
                    if let Ok(pos) = app_window.Position() {
                        return Some(screen_logical_to_xaml_local_pure(
                            screen_point,
                            (pos.X, pos.Y),
                            scale,
                        ));
                    }
                }
            }
        }

        None
    }

    /// Queries the real monitor work area in screen logical coordinates for the given canvas and optional anchor point.
    pub(crate) fn query_work_area_for_canvas(
        canvas: &crate::bindings::Microsoft::UI::Xaml::Controls::Canvas,
        anchor_pt: Option<Point>,
    ) -> Option<elwindui_core::base::Rect> {
        let xaml_root = canvas.XamlRoot().ok()?;
        let scale = xaml_root.RasterizationScale().unwrap_or(1.0);
        let scale_safe = if scale <= 0.0 { 1.0 } else { scale };

        // 1. Try DisplayArea::GetFromPoint if anchor_pt given
        if let Some(anchor) = anchor_pt {
            let pt_int = windows::Graphics::PointInt32 {
                X: (anchor.x as f64 * scale_safe) as i32,
                Y: (anchor.y as f64 * scale_safe) as i32,
            };
            if let Ok(display_area) =
                crate::bindings::Microsoft::UI::Windowing::DisplayArea::GetFromPoint(
                    pt_int,
                    crate::bindings::Microsoft::UI::Windowing::DisplayAreaFallback::Nearest,
                )
            {
                let outer = display_area.OuterBounds().unwrap_or_default();
                let work = display_area.WorkArea().unwrap_or_default();
                return Some(display_area_to_core_work_area(
                    outer.X,
                    outer.Y,
                    work.X,
                    work.Y,
                    work.Width,
                    work.Height,
                    scale_safe,
                ));
            }
        }

        // 2. Try DisplayArea::GetFromWindowId
        if let Ok(island) = xaml_root.ContentIslandEnvironment() {
            if let Ok(app_window_id) = island.AppWindowId() {
                if let Ok(display_area) =
                    crate::bindings::Microsoft::UI::Windowing::DisplayArea::GetFromWindowId(
                        app_window_id,
                        crate::bindings::Microsoft::UI::Windowing::DisplayAreaFallback::Nearest,
                    )
                {
                    let outer = display_area.OuterBounds().unwrap_or_default();
                    let work = display_area.WorkArea().unwrap_or_default();
                    return Some(display_area_to_core_work_area(
                        outer.X,
                        outer.Y,
                        work.X,
                        work.Y,
                        work.Width,
                        work.Height,
                        scale_safe,
                    ));
                }
            }
        }

        // 3. Fallback: Convert XamlRoot bounds (local DIP) explicitly to screen logical coordinates
        if let Ok(size) = xaml_root.Size() {
            if let (Some(p0), Some(p1)) = (
                Self::canvas_to_screen_point(canvas, Point { x: 0.0, y: 0.0 }),
                Self::canvas_to_screen_point(
                    canvas,
                    Point {
                        x: size.Width as f32,
                        y: size.Height as f32,
                    },
                ),
            ) {
                return Some(elwindui_core::base::Rect {
                    x: p0.x.min(p1.x),
                    y: p0.y.min(p1.y),
                    width: (p1.x - p0.x).abs().max(100.0),
                    height: (p1.y - p0.y).abs().max(100.0),
                });
            }
        }

        None
    }

    pub(crate) fn dispatch_context_request(
        tree: &Option<Rc<dyn UIElementExt>>,
        keyboard: &crate::host::KeyboardDispatcher,
        canvas: &crate::bindings::Microsoft::UI::Xaml::Controls::Canvas,
        active_popup: &RefCell<Option<Rc<dyn elwindui_core::ui::popup::PopupSurfaceHandle>>>,
        request: &elwindui_core::ui::ContextRequest,
    ) -> bool {
        let Some(tree) = tree.as_ref() else {
            return false;
        };
        let Some((resolved, anchor)) =
            elwindui_core::ui::ContextMenuService::process_request(tree, &keyboard.focus, request)
        else {
            return false;
        };
        match resolved.definition {
            elwindui_core::ui::popup::ResolvedContextDefinition::Menu { menu, presentation } => {
                match presentation {
                    elwindui_core::ui::ContextMenuPresentation::Native => {
                        if let Some(winui_menu) =
                            menu.as_any().downcast_ref::<crate::native_ui::Menu>()
                        {
                            if let Ok(flyout) = winui_menu.create_flyout() {
                                let fe: FrameworkElement =
                                    canvas.cast().expect("Canvas as FrameworkElement");
                                if flyout.show_at(&fe).is_ok() {
                                    return true;
                                }
                            }
                        }
                    }
                    elwindui_core::ui::ContextMenuPresentation::Custom => {
                        let anchor_pt = match &anchor {
                            elwindui_core::ui::popup::PopupAnchor::Point(pt) => Some(*pt),
                            elwindui_core::ui::popup::PopupAnchor::Rect(r) => {
                                Some(elwindui_core::base::Point { x: r.x, y: r.y })
                            }
                        };
                        let Some(work_area) = Self::query_work_area_for_canvas(canvas, anchor_pt)
                        else {
                            return false;
                        };
                        if let Some(old) = active_popup.borrow_mut().take() {
                            old.close();
                        }
                        let host = crate::inner::WinUI3PopupHost::new(canvas.clone());
                        let handle = elwindui_core::ui::ContextMenuService::open_custom_menu(
                            &host, &*menu, &anchor, work_area,
                        );
                        let opened = handle.is_some();
                        *active_popup.borrow_mut() = handle;
                        return opened;
                    }
                }
            }
            elwindui_core::ui::popup::ResolvedContextDefinition::Popup { template } => {
                let anchor_pt = match &anchor {
                    elwindui_core::ui::popup::PopupAnchor::Point(pt) => Some(*pt),
                    elwindui_core::ui::popup::PopupAnchor::Rect(r) => {
                        Some(elwindui_core::base::Point { x: r.x, y: r.y })
                    }
                };
                let Some(work_area) = Self::query_work_area_for_canvas(canvas, anchor_pt) else {
                    return false;
                };
                if let Some(old) = active_popup.borrow_mut().take() {
                    old.close();
                }
                let host = crate::inner::WinUI3PopupHost::new(canvas.clone());
                let handle = elwindui_core::ui::ContextMenuService::open_custom_popup(
                    &host,
                    &resolved.owner,
                    &template,
                    &anchor,
                    resolved.owner.effective_environment(),
                    work_area,
                );
                let opened = handle.is_some();
                *active_popup.borrow_mut() = handle;
                return opened;
            }
        }
        false
    }
}

/// Pure helper: converts physical display area bounds and work area offset into a global screen logical Rect.
pub fn display_area_to_core_work_area(
    outer_x: i32,
    outer_y: i32,
    work_x: i32,
    work_y: i32,
    work_width: i32,
    work_height: i32,
    scale: f64,
) -> Rect {
    let scale = if scale <= 0.0 { 1.0 } else { scale };
    let global_x = outer_x + work_x;
    let global_y = outer_y + work_y;
    Rect {
        x: (global_x as f64 / scale) as f32,
        y: (global_y as f64 / scale) as f32,
        width: (work_width as f64 / scale) as f32,
        height: (work_height as f64 / scale) as f32,
    }
}

/// Pure helper: converts canvas-to-window local DIP + window origin physical px into desktop screen logical DIP.
pub fn canvas_local_to_screen_logical_pure(
    canvas_to_window_local_dip: Point,
    window_origin_physical: (i32, i32),
    scale: f64,
) -> Point {
    let scale = if scale <= 0.0 { 1.0 } else { scale };
    let window_origin_dip_x = window_origin_physical.0 as f64 / scale;
    let window_origin_dip_y = window_origin_physical.1 as f64 / scale;
    Point {
        x: (window_origin_dip_x + canvas_to_window_local_dip.x as f64) as f32,
        y: (window_origin_dip_y + canvas_to_window_local_dip.y as f64) as f32,
    }
}

/// Pure helper: converts desktop screen logical DIP into XAML root local DIP.
pub fn screen_logical_to_xaml_local_pure(
    screen_logical: Point,
    window_origin_physical: (i32, i32),
    scale: f64,
) -> Point {
    let scale = if scale <= 0.0 { 1.0 } else { scale };
    let window_origin_dip_x = window_origin_physical.0 as f64 / scale;
    let window_origin_dip_y = window_origin_physical.1 as f64 / scale;
    Point {
        x: (screen_logical.x as f64 - window_origin_dip_x) as f32,
        y: (screen_logical.y as f64 - window_origin_dip_y) as f32,
    }
}

/// PR #165 rereview remediation round 2, A6/T25 (Layer 2): closes `slot`'s own active custom
/// popup/context-menu surface, if any — extracted out of `TreeHost::close_active_popup` as a
/// free function over a bare `&RefCell<..>` (no `TreeHost`/native host construction needed)
/// so it is unit-testable in isolation, mirroring `elwindui-backend-appkit`'s own identical
/// extraction (`host::close_active_popup_slot`). `take()`s the slot *before* calling `close()` so
/// a reentrant close triggered from within `close()` itself (e.g. the popup's own `on_unmount`
/// closing the owner Window again, which reaches `Window::unmount_override` ->
/// `close_active_popup` -> this same function a second time) finds the slot already empty rather
/// than double-closing it or panicking on a nested `RefCell` borrow. This crate is
/// `#![cfg(target_os = "windows")]`-gated in its entirety, so — like every other test in this same
/// module's own `#[cfg(test)] mod tests` below — this function's own unit tests cannot run in this
/// (macOS) environment regardless of how pure the function itself is; NOT VERIFIED here.
pub(crate) fn close_active_popup_slot(
    slot: &RefCell<Option<Rc<dyn elwindui_core::ui::popup::PopupSurfaceHandle>>>,
) {
    let popup = slot.borrow_mut().take();
    if let Some(popup) = popup {
        popup.close();
    }
}

/// Hosted structural regressions for the permanent self-drawn input surface. This helper is
/// called by the existing single-Application XAML regression test in `inner::button`; it must not
/// bootstrap another XAML `Application` in the same process.
#[cfg(test)]
pub(crate) mod live_input_surface_tests {
    use super::*;

    fn assert_surface_is_first(panel: &TreeHost, surface: &UIElement) {
        let children = panel.canvas().Children().expect("Canvas.Children");
        let child = children.GetAt(0).expect("Children.GetAt(0)");
        assert_eq!(child, *surface, "the input surface must remain at index 0");
        let mut surface_count = 0;
        for index in 0..children.Size().expect("Children.Size") {
            if children.GetAt(index).expect("Children.GetAt(index)") == *surface {
                surface_count += 1;
            }
        }
        assert_eq!(
            surface_count, 1,
            "exactly one permanent input surface must remain attached"
        );
    }

    pub(crate) fn live_input_surface_creation_persistence_viewport_and_source_classification() {
        let panel = TreeHost::new();
        let surface_ui: UIElement = panel
            .input_surface
            .clone()
            .cast()
            .expect("Rectangle is a UIElement");
        assert_surface_is_first(&panel, &surface_ui);
        assert!(
            surface_ui.IsHitTestVisible().expect("IsHitTestVisible"),
            "the input surface must be hit-testable"
        );
        assert_eq!(
            Canvas::GetLeft(&surface_ui).expect("Canvas.GetLeft"),
            0.0,
            "the input surface must start at Canvas.Left=0"
        );
        assert_eq!(
            Canvas::GetTop(&surface_ui).expect("Canvas.GetTop"),
            0.0,
            "the input surface must start at Canvas.Top=0"
        );
        let fill = panel
            .input_surface
            .Fill()
            .expect("Rectangle.Fill must be non-null");
        let solid_fill: crate::bindings::Microsoft::UI::Xaml::Media::SolidColorBrush = fill
            .cast()
            .expect("the input surface Fill must be a SolidColorBrush");
        assert_eq!(
            solid_fill.Color().expect("SolidColorBrush.Color").A,
            0,
            "the input surface Fill must be fully transparent"
        );

        // H3: constrained dimensions come from the owner-supplied final viewport.
        panel
            .set_viewport(TreeHostViewport {
                width: Some(320.0),
                height: Some(180.0),
            })
            .expect("set constrained viewport");
        let first_tree = elwindui_core::ui::Rectangle::new();
        first_tree.set_width(160.0);
        first_tree.set_height(96.0);
        panel.set_tree(first_tree);
        assert_eq!(panel.input_surface.Width().expect("Rectangle.Width"), 320.0);
        assert_eq!(
            panel.input_surface.Height().expect("Rectangle.Height"),
            180.0
        );
        assert_surface_is_first(&panel, &surface_ui);

        // PC-14: this is the actual renderer-created TextBlock projection for a Core text node,
        // not an unrelated manually-created native XAML child. The projection is paint-only, so
        // native hit testing must pass through to the permanent host input surface.
        use elwindui_core::ui::{TextBlock as CoreTextBlock, TextBlockExt};
        let renderer_probe = CoreTextBlock::new();
        renderer_probe.set_text("renderer-proxy probe");
        renderer_probe.set_width(120.0);
        renderer_probe.set_height(24.0);
        panel.set_tree(renderer_probe);
        assert_surface_is_first(&panel, &surface_ui);
        let children = panel.canvas().Children().expect("Canvas.Children");
        let projected = children.GetAt(1).expect("renderer TextBlock projection");
        let projected_text: crate::bindings::Microsoft::UI::Xaml::Controls::TextBlock = projected
            .cast()
            .expect("renderer projection must be a XAML TextBlock");
        assert_eq!(
            projected_text
                .Text()
                .expect("renderer TextBlock.Text")
                .to_string(),
            "renderer-proxy probe"
        );
        let projected_ui: UIElement = projected_text
            .clone()
            .cast()
            .expect("renderer TextBlock is a UIElement");
        assert!(
            !projected_ui
                .IsHitTestVisible()
                .expect("renderer TextBlock IsHitTestVisible"),
            "renderer-created TextBlock projections must not own native pointer input"
        );
        assert!(TreeHost::is_self_drawn_pointer_source(
            panel.canvas(),
            &panel.input_surface,
            panel.canvas()
        ));
        assert!(TreeHost::is_self_drawn_pointer_source(
            panel.canvas(),
            &panel.input_surface,
            &panel.input_surface
        ));
        assert!(!TreeHost::is_self_drawn_pointer_source(
            panel.canvas(),
            &panel.input_surface,
            &projected_ui
        ));

        // H3: unconstrained dimensions come from the post-layout natural extent, not from a
        // requested viewport or the native Canvas output fed back into layout.
        let natural_panel = TreeHost::new();
        natural_panel
            .set_viewport(TreeHostViewport {
                width: None,
                height: None,
            })
            .expect("set unconstrained viewport");
        let natural_probe = elwindui_core::ui::Rectangle::new();
        natural_probe.set_width(96.0);
        natural_probe.set_height(48.0);
        let natural_probe_for_assert = natural_probe.clone();
        natural_panel.set_tree(natural_probe);
        let natural_width = natural_probe_for_assert
            .arranged_width()
            .expect("natural arranged width");
        let natural_height = natural_probe_for_assert
            .arranged_height()
            .expect("natural arranged height");
        assert!(natural_width > 0.0, "natural width must be non-zero");
        assert!(natural_height > 0.0, "natural height must be non-zero");
        assert_eq!(
            natural_panel.input_surface.Width().expect("natural Width"),
            natural_width as f64
        );
        assert_eq!(
            natural_panel
                .input_surface
                .Height()
                .expect("natural Height"),
            natural_height as f64
        );

        // H2: replacement, clear, and visual transparency changes never recreate or remove the
        // permanent surface.
        let replacement = elwindui_core::ui::Rectangle::new();
        replacement.set_width(128.0);
        replacement.set_height(64.0);
        panel.set_tree(replacement);
        assert_surface_is_first(&panel, &surface_ui);
        panel.clear_tree();
        assert_surface_is_first(&panel, &surface_ui);
        for transparent in [true, false, true] {
            panel.set_transparent_background(transparent);
            assert_surface_is_first(&panel, &surface_ui);
            assert!(
                surface_ui.IsHitTestVisible().expect("IsHitTestVisible"),
                "transparency must not disable the input surface"
            );
            assert_eq!(
                panel
                    .input_surface
                    .Fill()
                    .expect("persistent Rectangle.Fill")
                    .cast::<crate::bindings::Microsoft::UI::Xaml::Media::SolidColorBrush>()
                    .expect("persistent SolidColorBrush")
                    .Color()
                    .expect("persistent brush color")
                    .A,
                0
            );
        }

        // H4: only the exact root Canvas and exact host surface are accepted. Both another
        // Rectangle and a real native Button must be rejected.
        assert!(TreeHost::is_self_drawn_pointer_source(
            panel.canvas(),
            &panel.input_surface,
            panel.canvas()
        ));
        assert!(TreeHost::is_self_drawn_pointer_source(
            panel.canvas(),
            &panel.input_surface,
            &panel.input_surface
        ));
        let other_rectangle = Rectangle::new().expect("other Rectangle::new");
        assert!(!TreeHost::is_self_drawn_pointer_source(
            panel.canvas(),
            &panel.input_surface,
            &other_rectangle
        ));
        let unrelated_text = crate::bindings::Microsoft::UI::Xaml::Controls::TextBlock::new()
            .expect("native TextBlock::new");
        assert!(!TreeHost::is_self_drawn_pointer_source(
            panel.canvas(),
            &panel.input_surface,
            &unrelated_text
        ));
        let native_button = crate::bindings::Microsoft::UI::Xaml::Controls::Button::new()
            .expect("native Button::new");
        assert!(!TreeHost::is_self_drawn_pointer_source(
            panel.canvas(),
            &panel.input_surface,
            &native_button
        ));

        // H5: only the root Canvas is gated for host activation. The permanent surface stays
        // attached, hit-testable, and correctly sized across the transition.
        let canvas_ui: UIElement = panel
            .canvas()
            .clone()
            .cast()
            .expect("Canvas is a UIElement");
        assert!(
            canvas_ui
                .IsHitTestVisible()
                .expect("active Canvas hit testing")
        );
        let active_tree = elwindui_core::ui::Rectangle::new();
        active_tree.set_width(160.0);
        active_tree.set_height(96.0);
        panel.set_tree(active_tree);
        panel.set_active(false);
        assert!(
            !canvas_ui
                .IsHitTestVisible()
                .expect("inactive Canvas hit testing")
        );
        assert_surface_is_first(&panel, &surface_ui);
        assert!(
            surface_ui
                .IsHitTestVisible()
                .expect("inactive surface remains hit-testable")
        );
        panel.set_active(true);
        assert!(
            canvas_ui
                .IsHitTestVisible()
                .expect("reactivated Canvas hit testing")
        );
        assert_surface_is_first(&panel, &surface_ui);
        assert_eq!(
            panel.input_surface.Width().expect("reactivated Width"),
            320.0
        );
        assert_eq!(
            panel.input_surface.Height().expect("reactivated Height"),
            180.0
        );
    }
}

/// Hosted accessibility bridge regressions. This helper is called by the existing
/// single-Application XAML regression test in `inner::button`; it must not bootstrap another XAML
/// `Application` in the same process.
#[cfg(test)]
pub(crate) mod accessibility_tests {
    use super::*;
    use elwindui_core::ui::{TextBlock, TextBlockExt};

    pub(crate) fn bridge_binding_and_core_snapshot() {
        let tree: Rc<RefCell<Option<Rc<dyn UIElementExt>>>> = Rc::new(RefCell::new(None));
        let state = WinUI3AccessibilityState::new(Rc::downgrade(&tree));
        let canvas = accessibility::create_canvas();

        assert!(
            state.bind_canvas(&canvas),
            "WinUI3 accessibility callbacks must bind through the canonical bridge key"
        );

        let text = TextBlock::new();
        text.set_text("semantic bridge probe");
        let text: Rc<dyn UIElementExt> = text;
        *tree.borrow_mut() = Some(text);
        state.rebuild();

        let snapshot = state.runtime.snapshot();
        assert!(
            !snapshot.roots.is_empty(),
            "Core accessibility snapshot is empty"
        );
        assert_eq!(
            snapshot.roots[0].semantics.label.as_deref(),
            Some("semantic bridge probe")
        );
        assert_eq!(
            snapshot.roots[0].semantics.value.as_deref(),
            Some("semantic bridge probe")
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn reentrant_invalidation_does_not_contaminate_the_next_pending_kind() {
        let pending_kind = Cell::new(elwindui_core::ui::InvalidationKind::Render);

        assert!(
            !record_pending_kind_if_idle(
                true,
                &pending_kind,
                elwindui_core::ui::InvalidationKind::Measure,
            ),
            "a reentrant request must be handled by run_coalesced, not the queued batch"
        );
        assert_eq!(
            pending_kind.get(),
            elwindui_core::ui::InvalidationKind::Render,
            "reentrant work must not mutate pending_kind"
        );

        assert!(record_pending_kind_if_idle(
            false,
            &pending_kind,
            elwindui_core::ui::InvalidationKind::Measure,
        ));
        assert_eq!(
            pending_kind.get(),
            elwindui_core::ui::InvalidationKind::Measure,
            "the next unrelated batch must record its own strongest kind"
        );
    }

    #[test]
    fn dispatcher_enqueue_acceptance_requires_ok_true() {
        assert!(dispatcher_enqueue_accepted(Ok(true)));
        assert!(!dispatcher_enqueue_accepted(Ok(false)));
        assert!(!dispatcher_enqueue_accepted(Err(
            windows::core::Error::new(
                windows::core::HRESULT(0x80004005u32 as i32),
                "dispatcher rejected enqueue",
            )
        )));
    }

    #[test]
    fn viewport_state_is_not_advanced_when_native_height_application_fails() {
        let stored = Cell::new(None);
        let order = Cell::new(0);
        let viewport = TreeHostViewport {
            width: Some(320.0),
            height: Some(240.0),
        };

        let result = apply_native_viewport(
            &stored,
            viewport,
            |_width| {
                order.set(1);
                Ok(())
            },
            |_height| {
                order.set(2);
                Err(windows::core::Error::new(
                    windows::core::HRESULT(0x80004005u32 as i32),
                    "height application failed",
                ))
            },
        );

        assert!(result.is_err());
        assert_eq!(order.get(), 2, "width must be attempted before height");
        assert_eq!(
            stored.get(),
            None,
            "failed native application must not publish viewport"
        );
    }

    /// T1 (Issue #235 review remediation): a same-host reentrant call — `pass` itself
    /// synchronously calling `run_coalesced` again on the *same* `RelayoutCycleState`, mirroring
    /// `relayout_static` re-entering itself via a structural change made mid-measure — must not
    /// recurse into a nested `pass` invocation. It must instead queue exactly one rerun, which the
    /// still-running outer loop picks up after the current iteration finishes.
    #[test]
    fn run_coalesced_same_host_reentry_reruns_once_without_recursing() {
        let state = RelayoutCycleState::default();
        let pass_count = Rc::new(Cell::new(0));
        let max_observed_depth = Rc::new(Cell::new(0));
        let depth = Rc::new(Cell::new(0));

        let state_for_pass = &state;
        let pass_count_for_pass = pass_count.clone();
        let max_observed_depth_for_pass = max_observed_depth.clone();
        let depth_for_pass = depth.clone();
        let ran = state_for_pass.run_coalesced(|| {
            depth_for_pass.set(depth_for_pass.get() + 1);
            max_observed_depth_for_pass
                .set(max_observed_depth_for_pass.get().max(depth_for_pass.get()));
            pass_count_for_pass.set(pass_count_for_pass.get() + 1);
            if pass_count_for_pass.get() == 1 {
                // The reentrant call: same `RelayoutCycleState`, from inside the first pass.
                let reentrant_ran = state_for_pass.run_coalesced(|| {
                    depth_for_pass.set(depth_for_pass.get() + 1);
                    max_observed_depth_for_pass
                        .set(max_observed_depth_for_pass.get().max(depth_for_pass.get()));
                    pass_count_for_pass.set(pass_count_for_pass.get() + 1);
                    depth_for_pass.set(depth_for_pass.get() - 1);
                });
                assert!(
                    !reentrant_ran,
                    "a same-host reentrant call must not itself run pass or claim to have looped"
                );
            }
            depth_for_pass.set(depth_for_pass.get() - 1);
        });

        assert!(ran, "the outermost call for this host must report it ran");
        assert_eq!(
            pass_count.get(),
            2,
            "the reentrant request must produce exactly one rerun after the current pass finishes"
        );
        assert_eq!(
            max_observed_depth.get(),
            1,
            "the reentrant call must never nest a second pass invocation on the call stack"
        );
    }

    /// T2 (Issue #235 review remediation): a *different* host's `RelayoutCycleState` must run
    /// immediately and independently even while this host's own pass is in progress — the guard is
    /// per-host state, never a thread-global.
    #[test]
    fn run_coalesced_different_host_is_not_suppressed() {
        let host_a = RelayoutCycleState::default();
        let host_b = RelayoutCycleState::default();
        let a_count = Rc::new(Cell::new(0));
        let b_count = Rc::new(Cell::new(0));

        let b_count_for_a = b_count.clone();
        let host_b_ref = &host_b;
        let ran_a = host_a.run_coalesced(|| {
            a_count.set(a_count.get() + 1);
            let b_count_for_pass = b_count_for_a.clone();
            let ran_b = host_b_ref.run_coalesced(move || {
                b_count_for_pass.set(b_count_for_pass.get() + 1);
            });
            assert!(
                ran_b,
                "a different host's relayout must execute immediately, not be blocked by host A's \
                 in-progress pass"
            );
        });

        assert!(ran_a);
        assert_eq!(a_count.get(), 1);
        assert_eq!(b_count.get(), 1);
    }

    /// T3 (Issue #235 review remediation): several synchronous same-host reentrant requests raised
    /// during one pass must coalesce into a single later rerun, not one rerun per request and not
    /// a recursive call per request.
    #[test]
    fn run_coalesced_multiple_same_host_requests_coalesce_into_one_rerun() {
        let state = RelayoutCycleState::default();
        let pass_count = Rc::new(Cell::new(0));

        let state_ref = &state;
        let pass_count_ref = pass_count.clone();
        state_ref.run_coalesced(|| {
            let current = pass_count_ref.get() + 1;
            pass_count_ref.set(current);
            if current == 1 {
                // Several reentrant requests within the same still-running pass.
                assert!(!state_ref.run_coalesced(|| {}));
                assert!(!state_ref.run_coalesced(|| {}));
                assert!(!state_ref.run_coalesced(|| {}));
            }
        });

        assert_eq!(
            pass_count.get(),
            2,
            "multiple same-host requests during one pass must coalesce into exactly one rerun"
        );
    }

    struct FakePopupSurfaceHandle {
        slot: Rc<RefCell<Option<Rc<dyn elwindui_core::ui::popup::PopupSurfaceHandle>>>>,
        close_count: Rc<Cell<u32>>,
        reenter: bool,
    }

    impl elwindui_core::ui::popup::PopupSurfaceHandle for FakePopupSurfaceHandle {
        fn close(&self) {
            assert!(
                self.slot.borrow().is_none(),
                "the slot must already be empty by the time PopupSurfaceHandle::close() runs"
            );
            self.close_count.set(self.close_count.get() + 1);
            if self.reenter {
                close_active_popup_slot(&self.slot);
            }
        }
    }

    /// T25 (Layer 2): a plain close — the slot holds a handle, `close_active_popup_slot` takes it
    /// (leaving the slot empty) before calling `close()`, and `close()` runs exactly once.
    #[test]
    fn close_active_popup_slot_takes_before_close_and_closes_exactly_once() {
        let slot: Rc<RefCell<Option<Rc<dyn elwindui_core::ui::popup::PopupSurfaceHandle>>>> =
            Rc::new(RefCell::new(None));
        let close_count = Rc::new(Cell::new(0));
        let handle: Rc<dyn elwindui_core::ui::popup::PopupSurfaceHandle> =
            Rc::new(FakePopupSurfaceHandle {
                slot: slot.clone(),
                close_count: close_count.clone(),
                reenter: false,
            });
        *slot.borrow_mut() = Some(handle);

        close_active_popup_slot(&slot);

        assert_eq!(close_count.get(), 1);
        assert!(slot.borrow().is_none());
    }

    /// T25 (Layer 2): an empty slot is a no-op — no panic, nothing closed.
    #[test]
    fn close_active_popup_slot_on_empty_slot_is_a_no_op() {
        let slot: Rc<RefCell<Option<Rc<dyn elwindui_core::ui::popup::PopupSurfaceHandle>>>> =
            Rc::new(RefCell::new(None));
        close_active_popup_slot(&slot);
        assert!(slot.borrow().is_none());
    }

    /// T25 (Layer 2): reentrancy safety — `PopupSurfaceHandle::close()` itself calls back into
    /// `close_active_popup_slot` on the *same* slot. Must not panic on a nested `RefCell` borrow,
    /// and the reentrant call must observe an already-empty slot (no second close).
    #[test]
    fn close_active_popup_slot_is_reentrancy_safe() {
        let slot: Rc<RefCell<Option<Rc<dyn elwindui_core::ui::popup::PopupSurfaceHandle>>>> =
            Rc::new(RefCell::new(None));
        let close_count = Rc::new(Cell::new(0));
        let handle: Rc<dyn elwindui_core::ui::popup::PopupSurfaceHandle> =
            Rc::new(FakePopupSurfaceHandle {
                slot: slot.clone(),
                close_count: close_count.clone(),
                reenter: true,
            });
        *slot.borrow_mut() = Some(handle);

        close_active_popup_slot(&slot);

        assert_eq!(
            close_count.get(),
            1,
            "the reentrant call must find the slot already empty and close nothing a second time"
        );
        assert!(slot.borrow().is_none());
    }

    #[test]
    fn display_area_work_area_primary_monitor_scale_1() {
        let work = display_area_to_core_work_area(0, 0, 0, 0, 1920, 1040, 1.0);
        assert_eq!(work.x, 0.0);
        assert_eq!(work.y, 0.0);
        assert_eq!(work.width, 1920.0);
        assert_eq!(work.height, 1040.0);
    }

    #[test]
    fn display_area_work_area_secondary_monitor_right() {
        // Secondary monitor to the right at (1920, 0) with work area offset (0, 0)
        let work = display_area_to_core_work_area(1920, 0, 0, 0, 1920, 1080, 1.0);
        assert_eq!(work.x, 1920.0);
        assert_eq!(work.y, 0.0);
        assert_eq!(work.width, 1920.0);
        assert_eq!(work.height, 1080.0);
    }

    #[test]
    fn display_area_work_area_secondary_monitor_left_negative_x() {
        // Secondary monitor to the left at (-1920, 0) with work area offset (0, 0)
        let work = display_area_to_core_work_area(-1920, 0, 0, 0, 1920, 1080, 1.0);
        assert_eq!(work.x, -1920.0);
        assert_eq!(work.y, 0.0);
        assert_eq!(work.width, 1920.0);
        assert_eq!(work.height, 1080.0);
    }

    #[test]
    fn display_area_work_area_secondary_left_with_left_taskbar() {
        // Secondary monitor at (-1920, 0) with 60px left taskbar offset
        let work = display_area_to_core_work_area(-1920, 0, 60, 0, 1860, 1080, 1.0);
        assert_eq!(work.x, -1860.0);
        assert_eq!(work.y, 0.0);
        assert_eq!(work.width, 1860.0);
        assert_eq!(work.height, 1080.0);
    }

    #[test]
    fn display_area_work_area_taskbar_top() {
        // Taskbar at top (offset y = 40)
        let work_top = display_area_to_core_work_area(0, 0, 0, 40, 1920, 1040, 1.0);
        assert_eq!(work_top.x, 0.0);
        assert_eq!(work_top.y, 40.0);
        assert_eq!(work_top.width, 1920.0);
        assert_eq!(work_top.height, 1040.0);
    }

    #[test]
    fn display_area_work_area_negative_y_monitor() {
        // Secondary monitor above primary at (0, -1080)
        let work = display_area_to_core_work_area(0, -1080, 0, 0, 1920, 1080, 1.0);
        assert_eq!(work.x, 0.0);
        assert_eq!(work.y, -1080.0);
        assert_eq!(work.width, 1920.0);
        assert_eq!(work.height, 1080.0);
    }

    #[test]
    fn display_area_work_area_fractional_scale() {
        // High-DPI monitor with scale 1.5 (3840x2160 physical -> 2560x1440 logical)
        let work_15 = display_area_to_core_work_area(0, 0, 0, 0, 3840, 2160, 1.5);
        assert_eq!(work_15.width, 2560.0);
        assert_eq!(work_15.height, 1440.0);

        // Scale 2.0 on secondary right monitor
        let work_20 = display_area_to_core_work_area(1920, 0, 0, 0, 3840, 2160, 2.0);
        assert_eq!(work_20.x, 960.0);
        assert_eq!(work_20.width, 1920.0);
        assert_eq!(work_20.height, 1080.0);
    }

    #[test]
    fn coordinate_round_trip_canvas_to_screen_to_xaml_local() {
        let canvas_local = Point { x: 50.0, y: 75.0 };
        let window_phys = (300, 450); // window at (300, 450) physical px
        let scale = 1.5;

        let screen = canvas_local_to_screen_logical_pure(canvas_local, window_phys, scale);
        // window_phys (300, 450) at scale 1.5 -> (200.0, 300.0) DIP
        // screen -> (200 + 50, 300 + 75) = (250.0, 375.0)
        assert_eq!(screen.x, 250.0);
        assert_eq!(screen.y, 375.0);

        let xaml_local = screen_logical_to_xaml_local_pure(screen, window_phys, scale);
        assert_eq!(xaml_local.x, 50.0);
        assert_eq!(xaml_local.y, 75.0);
    }

    #[test]
    fn coordinate_conversion_negative_window_origin() {
        let canvas_local = Point { x: 10.0, y: 20.0 };
        let window_phys = (-1920, -100);
        let scale = 1.0;

        let screen = canvas_local_to_screen_logical_pure(canvas_local, window_phys, scale);
        assert_eq!(screen.x, -1910.0);
        assert_eq!(screen.y, -80.0);

        let local = screen_logical_to_xaml_local_pure(screen, window_phys, scale);
        assert_eq!(local.x, 10.0);
        assert_eq!(local.y, 20.0);
    }

    #[test]
    fn pointer_trace_capture_loss_hook_requires_trace_and_exact_flag() {
        assert!(PointerTrace::capture_loss_hook_enabled(
            true,
            Some("1".to_string())
        ));
        assert!(!PointerTrace::capture_loss_hook_enabled(
            false,
            Some("1".to_string())
        ));
        assert!(!PointerTrace::capture_loss_hook_enabled(
            true,
            Some("0".to_string())
        ));
        assert!(!PointerTrace::capture_loss_hook_enabled(true, None));
    }

    #[test]
    fn pointer_trace_disabled_is_a_no_op() {
        let trace = PointerTrace {
            path: None,
            next_order: Rc::new(Cell::new(0)),
            release_capture_on_press: false,
        };
        trace.record_release(1, "root_canvas", true);
        assert_eq!(trace.next_order.get(), 0);
    }

    #[test]
    fn pointer_trace_write_failure_is_non_panicking() {
        let trace = PointerTrace {
            path: Some(PathBuf::from("C:\\")),
            next_order: Rc::new(Cell::new(0)),
            release_capture_on_press: false,
        };
        trace.record_release(1, "root_canvas", false);
        assert_eq!(trace.next_order.get(), 1);
    }

    #[test]
    fn pointer_trace_records_release_result_without_core_callback_claim() {
        let path = std::env::temp_dir().join(format!(
            "elwindui-pointer-trace-{}.jsonl",
            std::process::id()
        ));
        let trace = PointerTrace {
            path: Some(path.clone()),
            next_order: Rc::new(Cell::new(0)),
            release_capture_on_press: false,
        };
        trace.record_pointer(
            "PointerCanceled",
            1,
            "root_canvas",
            true,
            Some(Point { x: 10.0, y: 20.0 }),
            Some(Point { x: 110.0, y: 120.0 }),
        );
        trace.record_release(1, "root_canvas", true);
        trace.record_release(1, "root_canvas", false);
        let contents = std::fs::read_to_string(&path).expect("trace file should be readable");
        assert!(contents.contains("\"event\":\"PointerCanceled\""));
        assert!(contents.contains("\"native_capture_release_success\":true"));
        assert!(contents.contains("\"native_capture_release_success\":false"));
        assert!(!contents.contains("CoreCancellation"));
        let _ = std::fs::remove_file(path);
    }
}
