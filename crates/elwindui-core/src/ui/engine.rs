//! Layout, render-group building/reconciliation, hit-testing, and routed-event dispatch — the
//! tree-walking engine that operates *over* the element classes rather than being part of any one
//! of them.
//!
//! Split out of this module's former single-file form unchanged; `use super::*` keeps the shared
//! import block in `mod.rs` so that split stayed a pure code move.

use super::*;
use crate::base::Vector;

/// WinUI3's `FrameworkElement.MeasureCore`-style constraint step, used by `UIElement::measure`: an
/// explicit `width`/`height` overrides that axis outright, then both axes are clamped to
/// `min_width..max_width`/`min_height..max_height` (`crate::layout::apply_size_constraints`).
/// Applied twice per element per the same WinUI3 algorithm — once to the space handed down to
/// `measure_override` (a fixed `Width` shouldn't let a container measure against the parent's
/// *actual* available space), once to `measure_override`'s own returned size (a container's
/// natural content size shouldn't override an explicit `Width`/`Height`/`Max*`). Generic over
/// `?Sized` so it can be called with `self: &Self` from inside the `measure` trait default method
/// (where `Self` isn't known to be `Sized`, since `measure` must stay callable through
/// `dyn UIElement`) without an unsized coercion.
pub(crate) fn constrain<T: UIElementExt + ?Sized>(elem: &T, size: Size) -> Size {
    let overridden = Size {
        width: elem.presentation_width().unwrap_or(size.width),
        height: elem.presentation_height().unwrap_or(size.height),
    };
    apply_size_constraints(
        overridden,
        elem.presentation_min_width(),
        elem.presentation_max_width(),
        elem.presentation_min_height(),
        elem.presentation_max_height(),
    )
}

/// This element's natural (unconstrained) size — e.g. for a container that must report an
/// `intrinsicContentSize` to an Auto-Layout-managed ancestor (see `elwindui-backend-appkit`'s
/// `TreeHost`) before it has ever actually been given a frame to lay out into.
pub fn natural_size(elem: &dyn UIElementExt) -> Size {
    elem.measure(Size {
        width: 0.0,
        height: 0.0,
    });
    elem.measured_size().unwrap_or_default()
}

/// Records one Visual's local retained commands. Geometry and hierarchy are reconciled separately
/// so a dirty Visual does not require replacing its RenderGroup allocation.
pub(crate) fn record_group_commands<H: Clone + 'static>(
    elem: &Rc<dyn UIElementExt>,
    group: &mut RenderGroup,
) {
    group.commands.clear();
    let size = Size {
        width: elem.arranged_width().unwrap_or(0.0),
        height: elem.arranged_height().unwrap_or(0.0),
    };
    let mut context = RenderContext::begin_group(&mut group.commands, group.offset, group.clip);
    if let Some(native) = elem
        .as_ref()
        .try_as_native_control()
        .and_then(|value| value.downcast_ref::<H>())
    {
        context.native_control(
            group.id,
            Rc::new(native.clone()),
            Rect {
                x: 0.0,
                y: 0.0,
                width: size.width,
                height: size.height,
            },
        );
    }
    elem.render(&mut context);
    context.end_group();
}

/// Builds one retained RenderGroup for every arranged, visible Visual — threading `path`/
/// `group_paths`/`visual_index` bookkeeping through this same recursive walk (formerly a
/// separate `index_render_groups` pass run afterward over the already-built tree) so
/// `RenderTree::new`/`reconcile` each visit every node exactly once, not twice.
pub(crate) fn build_render_group<H: Clone + 'static>(
    elem: &Rc<dyn UIElementExt>,
    offset: Point,
    path: &mut Vec<usize>,
    group_paths: &mut HashMap<u64, Vec<usize>>,
    visual_index: &mut HashMap<u64, Weak<dyn UIElementExt>>,
) -> Option<RenderGroup> {
    if !elem.participates_in_render() {
        return None;
    }
    let size = Size {
        width: elem.arranged_width().unwrap_or(0.0),
        height: elem.arranged_height().unwrap_or(0.0),
    };
    let clip = elem.clip_to_bounds().then_some(Rect {
        x: 0.0,
        y: 0.0,
        width: size.width,
        height: size.height,
    });
    let id = elem.render_group_id();
    let mut group = RenderGroup::new(id, offset, clip);
    group.size = size;
    group.transform = effective_local_presentation_transform(elem.as_ref(), size);
    group.opacity = elem.presentation_opacity();
    group.input_enabled = elem.participates_in_layout() && elem.hit_test_visible();
    record_group_commands::<H>(elem, &mut group);
    group.generation += 1;
    group_paths.insert(id, path.clone());
    visual_index.insert(id, Rc::downgrade(elem));
    for child in elem.visual_children() {
        let child_offset = child.arranged_offset().unwrap_or(Point { x: 0.0, y: 0.0 });
        // The dense index this child would occupy in `group.children` if it participates —
        // pushed before recursing so the child's own `group_paths` entry (inserted inside that
        // call, only if it participates) ends with that index. Popped unconditionally afterward;
        // a non-participating child returns `None` before ever reading `path`, so pushing an
        // index for it that ends up unused is harmless.
        path.push(group.children.len());
        if let Some(child_group) =
            build_render_group::<H>(&child, child_offset, path, group_paths, visual_index)
        {
            group.children.push(child_group);
        }
        path.pop();
    }
    group.is_dirty = false;
    Some(group)
}

/// Measures and arranges a host's content root. Rendering is intentionally separate: a host keeps
/// its RenderTree and calls `RenderTree::new` once, then `RenderTree::reconcile` after each layout.
pub fn layout_root(root: &Rc<dyn UIElementExt>, available: Size) {
    root.measure(available);
    // `available` may be infinite on an axis (e.g. `InnerScrollView`'s content host measures its
    // scrolling axis unconstrained, so the hosted tree reports its own natural size instead of
    // being clamped to the viewport — see that type's own doc comment). Arranging into that same
    // infinite rect would make `arranged_width`/`arranged_height` report infinity too, instead of
    // the finite natural size `measure` just resolved — so any non-finite axis falls back to the
    // measured size here, and `arrange` always receives a real, finite rect.
    let measured = root.measured_size().unwrap_or(available);
    let allotted = Rect {
        x: 0.0,
        y: 0.0,
        width: if available.width.is_finite() {
            available.width
        } else {
            measured.width
        },
        height: if available.height.is_finite() {
            available.height
        } else {
            measured.height
        },
    };
    root.arrange(allotted);
    // The target layout now reflects every structural change recorded so far, so the next
    // reconcile may compare it against the retained RenderTree and consume the reflow intent.
    if let Some(host) = animation_frame_host(root.as_ui_element()) {
        host.animation_runtime().arm_layout_reflow_intent();
    }
}

/// Per-reconcile layout reflow state: the host runtime, the consumed armed intent (if any), and
/// whether this pass started or retargeted a reflow that needs a frame.
pub(crate) struct LayoutReflowPass {
    runtime: Rc<AnimationRuntime>,
    intent: Option<LayoutReflowIntent>,
    started: Cell<bool>,
}

/// Starts, retargets, or snaps the layout reflow of a surviving child whose retained group was
/// last rendered at `old` and whose new target is `new`, both in the parent's local layout space.
/// Publishes the starting translation synchronously into the element; never invalidates, because
/// the caller holds the RenderTree mutably.
fn apply_layout_reflow(
    child: &Rc<dyn UIElementExt>,
    old: Point,
    new: Point,
    pass: &LayoutReflowPass,
) {
    let Some(intent) = pass.intent else {
        return;
    };
    if old == new || !child.participates_in_layout() {
        return;
    }
    let base = child.as_ui_element();
    let id = child.render_group_id();
    let finite = old.x.is_finite() && old.y.is_finite() && new.x.is_finite() && new.y.is_finite();
    if !finite {
        eprintln!(
            "[animation] layout reflow snapped for group {id}: non-finite layout origin \
             {old:?} -> {new:?}"
        );
    }
    let animation = match intent {
        LayoutReflowIntent::Animate(animation)
            if finite
                && !child
                    .effective_environment()
                    .get::<crate::environment::ReduceMotionEnvironment>() =>
        {
            animation
        }
        _ => {
            pass.runtime.cancel_layout_reflow(id);
            base.layout_reflow_translation.set(Vector::default());
            return;
        }
    };
    let delta = Vector {
        x: old.x - new.x,
        y: old.y - new.y,
    };
    let weak: Weak<dyn UIElementExt> = Rc::downgrade(child);
    let callback_target = weak.clone();
    let runtime = Rc::downgrade(&pass.runtime);
    let callback = Box::new(move |value: AnimatedValue, finished: bool| {
        let element: Option<Rc<dyn UIElementExt>> = callback_target.upgrade();
        let Some(element) = element else {
            if let Some(runtime) = runtime.upgrade() {
                runtime.cancel_layout_reflow(id);
            }
            return;
        };
        let Some(runtime) = runtime.upgrade() else {
            return;
        };
        if !runtime.has_layout_reflow(id) {
            // Cancelled earlier in this tick (snap, teardown, reduced motion): never write back.
            return;
        }
        let reduce_motion = element
            .effective_environment()
            .get::<crate::environment::ReduceMotionEnvironment>();
        if reduce_motion && !finished {
            runtime.cancel_layout_reflow(id);
        }
        let translation = match value {
            AnimatedValue::Transform(value) if !finished && !reduce_motion => value.translation,
            _ => Vector::default(),
        };
        let base = element.as_ui_element();
        base.layout_reflow_translation.set(translation);
        request_relayout(base, InvalidationKind::Render);
    });
    match pass.runtime.rebase_layout_reflow(
        id,
        base.layout_reflow_translation.get(),
        delta,
        animation,
        Some(weak),
        callback,
    ) {
        Some(start) => {
            base.layout_reflow_translation.set(start);
            pass.started.set(true);
        }
        None => {
            eprintln!("[animation] layout reflow snapped for group {id}: non-finite translation");
            base.layout_reflow_translation.set(Vector::default());
        }
    }
}

/// Reconciles an already-built `RenderGroup` against `elem`'s current layout/children, threading
/// the same `path`/`group_paths`/`visual_index` bookkeeping `build_render_group` does — see that
/// function's own doc comment for why this replaces a separate post-pass over the tree.
pub(crate) fn reconcile_render_group<H: Clone + 'static>(
    elem: &Rc<dyn UIElementExt>,
    group: &mut RenderGroup,
    offset: Point,
    path: &mut Vec<usize>,
    group_paths: &mut HashMap<u64, Vec<usize>>,
    visual_index: &mut HashMap<u64, Weak<dyn UIElementExt>>,
    reflow: Option<&LayoutReflowPass>,
) {
    let size = Size {
        width: elem.arranged_width().unwrap_or(0.0),
        height: elem.arranged_height().unwrap_or(0.0),
    };
    let clip = elem.clip_to_bounds().then_some(Rect {
        x: 0.0,
        y: 0.0,
        width: size.width,
        height: size.height,
    });
    let transform = effective_local_presentation_transform(elem.as_ref(), size);
    let opacity = elem.presentation_opacity();
    let input_enabled = elem.participates_in_layout() && elem.hit_test_visible();
    if group.offset != offset
        || group.size != size
        || group.clip != clip
        || group.transform != transform
        || group.opacity != opacity
        || group.input_enabled != input_enabled
    {
        group.offset = offset;
        group.size = size;
        group.clip = clip;
        group.transform = transform;
        group.opacity = opacity;
        group.input_enabled = input_enabled;
        group.is_dirty = true;
    }
    group_paths.insert(group.id, path.clone());
    visual_index.insert(group.id, Rc::downgrade(elem));

    let old_children = std::mem::take(&mut group.children);
    let mut old_by_id: HashMap<u64, RenderGroup> = old_children
        .into_iter()
        .map(|child| (child.id, child))
        .collect();
    let mut children = Vec::new();
    for child in elem.visual_children() {
        if !child.participates_in_render() {
            continue;
        }
        let child_offset = child.arranged_offset().unwrap_or(Point { x: 0.0, y: 0.0 });
        let id = child.render_group_id();
        // See `build_render_group`'s own comment on `path.push`/`path.pop` — `children.len()`
        // here is this child's dense index in `group.children`-to-be, the same role
        // `group.children.len()` plays there.
        path.push(children.len());
        let child_group = if let Some(mut existing) = old_by_id.remove(&id) {
            // The matched retained group is the only source of the previously rendered position.
            if let (Some(reflow), Some(new_offset)) = (reflow, child.arranged_offset()) {
                apply_layout_reflow(&child, existing.offset, new_offset, reflow);
            }
            reconcile_render_group::<H>(
                &child,
                &mut existing,
                child_offset,
                path,
                group_paths,
                visual_index,
                reflow,
            );
            existing
        } else {
            group.is_dirty = true;
            build_render_group::<H>(&child, child_offset, path, group_paths, visual_index)
                .expect("visible Visual must have a RenderGroup")
        };
        path.pop();
        children.push(child_group);
    }
    if !old_by_id.is_empty() {
        group.is_dirty = true;
    }
    group.children = children;
    if group.is_dirty {
        record_group_commands::<H>(elem, group);
        group.is_dirty = false;
        group.generation += 1;
    }
}

impl RenderTree {
    /// Creates the initial retained tree from a layout-complete content root.
    pub fn new<H: Clone + 'static>(root: &Rc<dyn UIElementExt>) -> Self {
        // A freshly built tree has no retained geometry to animate from: initial mount and host
        // re-creation never reflow, and a stale intent must not leak into the next reconcile.
        if let Some(host) = animation_frame_host(root.as_ui_element()) {
            host.animation_runtime().discard_layout_reflow_intent();
        }
        let offset = root.arranged_offset().unwrap_or(Point { x: 0.0, y: 0.0 });
        let mut group_paths = HashMap::new();
        let mut visual_index = HashMap::new();
        let root_group = build_render_group::<H>(
            root,
            offset,
            &mut Vec::new(),
            &mut group_paths,
            &mut visual_index,
        )
        .unwrap_or_else(|| {
            // A non-participating root still gets its own `group_paths`/`visual_index` entry (at
            // the empty path — it *is* the root) even though `build_render_group` returned `None`
            // before recording one; it just has no descendants to index, since `build_render_group`
            // never got as far as visiting any of them.
            group_paths.insert(root.render_group_id(), Vec::new());
            visual_index.insert(root.render_group_id(), Rc::downgrade(root));
            RenderGroup::new(root.render_group_id(), offset, None)
        });
        Self {
            root: root_group,
            group_paths,
            visual_index,
        }
    }

    /// Reconciles an already retained tree after `layout_root`. Group identities and clean command
    /// buffers survive; only changed or explicitly invalidated groups record commands again.
    pub fn reconcile<H: Clone + 'static>(&mut self, root: &Rc<dyn UIElementExt>) -> bool {
        if self.root.id != root.render_group_id() {
            return false;
        }
        let offset = root.arranged_offset().unwrap_or(Point { x: 0.0, y: 0.0 });
        self.group_paths.clear();
        self.visual_index.clear();
        let frame_host = animation_frame_host(root.as_ui_element());
        let reflow = frame_host.as_ref().map(|host| {
            let runtime = host.animation_runtime();
            let intent = runtime.take_armed_layout_reflow_intent();
            LayoutReflowPass {
                runtime,
                intent,
                started: Cell::new(false),
            }
        });
        if root.participates_in_render() {
            reconcile_render_group::<H>(
                root,
                &mut self.root,
                offset,
                &mut Vec::new(),
                &mut self.group_paths,
                &mut self.visual_index,
                reflow.as_ref(),
            );
        } else {
            // Mirrors `new`'s own non-participating fallback above: the root's `RenderGroup`
            // collapses to the same empty shape a freshly built one would have (no commands, no
            // children) rather than having `reconcile_render_group` — which assumes `elem`
            // participates — re-record an empty-content group's commands and recurse into what
            // would otherwise be stale, now-orphaned former children.
            self.root = RenderGroup::new(self.root.id, offset, None);
            self.group_paths.insert(self.root.id, Vec::new());
            self.visual_index.insert(self.root.id, Rc::downgrade(root));
        }
        // Started reflows already published their first translation above; later ticks need a
        // frame. The host schedules it asynchronously, so no RenderTree borrow is re-entered.
        if let (Some(host), Some(reflow)) = (frame_host, reflow) {
            if reflow.started.get() && reflow.runtime.take_frame_request() {
                host.request_animation_frame();
            }
        }
        true
    }

    pub fn root_id(&self) -> u64 {
        self.root.id
    }
}

pub(crate) fn rect_contains(rect: Rect, at: Point) -> bool {
    at.x >= rect.x && at.x <= rect.x + rect.width && at.y >= rect.y && at.y <= rect.y + rect.height
}

#[derive(Clone, Copy)]
struct HitTestClip {
    local_to_root: AffineTransform,
    size: Size,
}

fn point_in_local_bounds(local_to_root: AffineTransform, size: Size, at: Point) -> bool {
    local_to_root
        .invert()
        .map(|inverse| {
            rect_contains(
                Rect {
                    x: 0.0,
                    y: 0.0,
                    width: size.width,
                    height: size.height,
                },
                inverse.transform_point(at),
            )
        })
        .unwrap_or(false)
}

/// Re-runs the same read-only traversal `collect_render_items` (above) does, without needing to
/// know any backend's native handle type — hit-testing only needs each element's own already-
/// `arrange`d rect, never its handle. Returns the deepest (topmost) element whose rect contains
/// `at`, or `None` if `at` falls outside `elem`'s own bounds entirely.
///
/// Two points where this deliberately mirrors WinUI3/WPF rather than a naive "does the point fall
/// within this element's own rect" test:
///
/// - **`clip_to_bounds` only clips when actually set**, exactly like rendering already does
///   (`build_render_group`/`reconcile_render_group` only attach a `RenderGroup.clip` when
///   `elem.clip_to_bounds()` is `true`, and `elwindui-backend-appkit`'s `replay_group` intersects
///   that clip down through the tree). A child positioned outside its own (non-clipping) parent's
///   rect remains hit-testable — only an ancestor that opted into `clip_to_bounds` bounds its
///   descendants. `inherited_clip` threads the accumulated effective clip (the intersection of
///   every such opted-in ancestor's own rect) down the recursion; `at` falling outside it excludes
///   the element *and* its whole subtree, mirroring `Visibility::Collapsed`'s treatment.
/// - **An element with no visible content of its own isn't a self-hit candidate**
///   (`UIElement::hit_test_content` — WinUI3/WPF's "unset `Background`/`Fill` isn't hit-testable"
///   rule). Children are still searched regardless (they may have their own content), so a click in
///   a `Layout`'s empty space correctly falls through to whatever's behind it rather than being
///   captured by the layout container itself.
///
/// See `elwindui_core::input::PointerDispatcher`'s doc comment (modeled on WinUI3's routed events)
/// — bubbling from the returned element is then just `dispatch_routed` following `visual_parent()`,
/// no path/ancestor computation needed here.
fn hit_test_at(
    elem: &Rc<dyn UIElementExt>,
    local_to_root: AffineTransform,
    at: Point,
    inherited_clips: &[HitTestClip],
) -> Option<Rc<dyn UIElementExt>> {
    // A non-participating element (and its whole subtree) is excluded from hit-testing, matching
    // `build_render_group`'s own treatment — see `UIElementExt::participates_in_layout`'s own doc
    // comment. `hit_test_visible == false` (WinUI3's `IsHitTestVisible`) excludes the subtree the
    // same way, with no layout/render effect at all — see that field's own doc comment.
    if !elem.participates_in_layout() || !elem.hit_test_visible() {
        return None;
    }
    for clip in inherited_clips {
        if !point_in_local_bounds(clip.local_to_root, clip.size, at) {
            return None;
        }
    }

    let size = Size {
        width: elem.arranged_width().unwrap_or(0.0),
        height: elem.arranged_height().unwrap_or(0.0),
    };
    // A singular transform is not hit-testable, and the same inverse is used for both the own
    // bounds and descendant clip checks. RenderTree/backend replay uses the corresponding Core
    // local_transform helper, so there is no matrix-coefficient approximation here.
    let local_point = local_to_root.invert()?.transform_point(at);
    let own_rect = Rect {
        x: 0.0,
        y: 0.0,
        width: size.width,
        height: size.height,
    };

    let mut child_clips = inherited_clips.to_vec();
    if elem.clip_to_bounds() {
        child_clips.push(HitTestClip {
            local_to_root,
            size,
        });
    }

    // Children are searched last-to-first: traversal order paints later children on top of
    // earlier ones (see 付録N's z-order note), so the *last* child whose own rect contains `at`
    // is the topmost, correctly-hit one. Checked regardless of whether `at` falls within `elem`'s
    // *own* rect — a child may render outside a non-clipping parent's bounds (see this function's
    // own doc comment).
    for child in elem.visual_children().iter().rev() {
        let offset = child.arranged_offset().unwrap_or(Point { x: 0.0, y: 0.0 });
        let child_size = Size {
            width: child.arranged_width().unwrap_or(0.0),
            height: child.arranged_height().unwrap_or(0.0),
        };
        let child_layout = AffineTransform::translation(offset.x, offset.y);
        let child_local = effective_local_presentation_transform(child.as_ref(), child_size);
        let child_to_root = local_to_root.concat(&child_layout.concat(&child_local));
        if let Some(hit) = hit_test_at(child, child_to_root, at, &child_clips) {
            return Some(hit);
        }
    }

    if rect_contains(own_rect, local_point) && elem.hit_test_content() {
        Some(Rc::clone(elem))
    } else {
        None
    }
}

/// Hit-tests `root` at `at` (absolute coordinates, e.g. the hosting `TreeHost`'s own local
/// point). Returns the deepest (topmost) hit element, or `None` if `at` falls outside `root`'s own
/// bounds entirely. Requires `root` to have already been laid out (e.g. via `layout_root`) — reads
/// cached `arranged_width`/`arranged_height`/`arranged_offset`, doesn't recompute them.
pub fn hit_test(root: &Rc<dyn UIElementExt>, at: Point) -> Option<Rc<dyn UIElementExt>> {
    // See `layout_root`'s own matching comment — `root`'s own `arranged_offset` (from its margin/
    // alignment against the original allotted rect) must be folded in here too, so hit-testing
    // agrees with `collect_render_items`'s rendered coordinates.
    let root_offset = root.arranged_offset().unwrap_or(Point { x: 0.0, y: 0.0 });
    let root_size = Size {
        width: root.arranged_width().unwrap_or(0.0),
        height: root.arranged_height().unwrap_or(0.0),
    };
    let root_to_parent = AffineTransform::translation(root_offset.x, root_offset.y);
    let root_transform = root_to_parent.concat(&effective_local_presentation_transform(
        root.as_ref(),
        root_size,
    ));
    hit_test_at(root, root_transform, at, &[])
}

/// Invokes only `elem`'s own handlers registered under `name` (via
/// `UIElement::register_routed_handler::<T>`) — no bubbling to `parent()` at all. Factored out of
/// `dispatch_routed` (which loops this over the parent chain) so callers that need to fire a
/// routed event at a *specific* element without also re-firing it at every one of that element's
/// ancestors can do so — e.g. `PointerDispatcher`'s ancestor-chain-diffed `on_pointer_entered`/
/// `on_pointer_exited` (WPF/UWP's non-bubbling `MouseEnter`/`MouseLeave` semantics: an ancestor
/// that's still hovered must not see a spurious re-fire just because a *deeper* descendant's hover
/// state changed). `T` must match the type every handler for `name` was registered with — see
/// `UIElement::routed_handlers`'s doc comment for why the downcast this performs always succeeds in
/// practice.
pub(crate) fn invoke_handlers_at<T: 'static>(
    elem: &Rc<dyn UIElementExt>,
    name: &str,
    payload: &T,
    args: &RoutedEventArgs,
) {
    let handlers = elem.as_ui_element().routed_handlers.borrow();
    if let Some(handlers) = handlers.get(name) {
        for handler in handlers {
            if let Some(handler) = handler.downcast_ref::<Box<dyn Fn(&T, &RoutedEventArgs)>>() {
                // Ordinary handlers never see an already-handled event.
                if !args.handled.get() {
                    handler(payload, args);
                }
            } else if let Some(handler) =
                handler.downcast_ref::<crate::ui::HandledEventsTooHandler<T>>()
            {
                (handler.0)(payload, args);
            } else {
                panic!("elwindui: routed handler registered under a mismatched payload type");
            }
        }
    }
}

/// Bubbles a routed event starting at `target` (e.g. `hit_test`'s return value, or a native leaf's
/// own tree node — see `elwindui-backend-appkit`'s `TreeHost`): calls `target`'s own handlers
/// registered under `name`, then its parent's, and so on up to the root (`UIElement::visual_parent`
/// — matching real WinUI3, where routed events bubble along the Visual tree, not the Logical one),
/// stopping as soon as one sets `args.handled`. Works identically whether `target`'s tree was built
/// by a single static DSL traversal or assembled at runtime by a `for` child range.
pub fn dispatch_routed<T: 'static>(
    target: &Rc<dyn UIElementExt>,
    name: &str,
    payload: &T,
    args: &RoutedEventArgs,
) {
    let mut current = Some(Rc::clone(target));
    while let Some(elem) = current {
        // Once handled, bubbling continues only so `HandledEventsTooHandler`s on ancestors still
        // run; `invoke_handlers_at` skips ordinary handlers for a handled event.
        invoke_handlers_at(&elem, name, payload, args);
        current = elem.visual_parent();
    }
}

/// See `invoke_handlers_at`'s own doc comment — the `pub(crate)` entry point `elwindui_core::input`
/// uses for non-bubbling routed dispatch.
pub(crate) fn dispatch_direct<T: 'static>(
    target: &Rc<dyn UIElementExt>,
    name: &str,
    payload: &T,
    args: &RoutedEventArgs,
) {
    invoke_handlers_at(target, name, payload, args);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::base::Vector;
    use crate::ui::testsupport::*;

    #[test]
    fn single_native_leaf_as_root_fills_available_space() {
        // The root's default alignment is `Stretch`, so it fills `available` regardless of its
        // own measured size — this matters for e.g. `TabView` (a native leaf) as `Window`'s
        // content: it must fill the window, not shrink to its own `fittingSize()`.
        let tree = native("a", size(10.0, 20.0));
        let (natives, paints) = split(layout_tree::<FakeHandle>(&tree, size(200.0, 100.0)));
        assert_eq!(
            natives,
            vec![(
                FakeHandle("a", size(10.0, 20.0)),
                Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 200.0,
                    height: 100.0
                }
            )]
        );
        assert!(paints.is_empty());
    }

    #[test]
    fn nested_stack_accumulates_absolute_offsets() {
        // Vertical outer stack containing a native leaf, then a horizontal inner stack of two
        // native leaves — checks that the inner stack's children get *absolute* coordinates, not
        // coordinates relative to the inner stack alone. Every element here uses `Left`/`Top`
        // alignment explicitly (not the `Stretch` default) so each child keeps its own measured
        // size instead of filling its stack-allocated cross-axis slot.
        fn leaf(name: &'static str, s: Size) -> Rc<dyn UIElementExt> {
            let node = FakeNativeControl::new(FakeHandle(name, s));
            node.as_ui_element()
                .set_horizontal_alignment(HorizontalAlignment::Left);
            node.as_ui_element()
                .set_vertical_alignment(VerticalAlignment::Top);
            node
        }
        fn start_stack(
            orientation: Orientation,
            spacing: f32,
            children: Vec<Rc<dyn UIElementExt>>,
        ) -> Rc<dyn UIElementExt> {
            let node: Rc<dyn UIElementExt> = match orientation {
                Orientation::Vertical => {
                    let stack = VerticalLayout::new();
                    stack.set_spacing(spacing);
                    for child in children {
                        stack.children().add(child);
                    }
                    stack
                }
                Orientation::Horizontal => {
                    let stack = HorizontalLayout::new();
                    stack.set_spacing(spacing);
                    for child in children {
                        stack.children().add(child);
                    }
                    stack
                }
            };
            node.as_ui_element()
                .set_horizontal_alignment(HorizontalAlignment::Left);
            node.as_ui_element()
                .set_vertical_alignment(VerticalAlignment::Top);
            node
        }

        let tree = start_stack(
            Orientation::Vertical,
            5.0,
            vec![
                leaf("top", size(50.0, 10.0)),
                start_stack(
                    Orientation::Horizontal,
                    2.0,
                    vec![
                        leaf("left", size(20.0, 20.0)),
                        leaf("right", size(30.0, 20.0)),
                    ],
                ),
            ],
        );

        let (natives, paints) = split(layout_tree::<FakeHandle>(&tree, size(200.0, 200.0)));
        assert!(paints.is_empty());
        assert_eq!(natives.len(), 3);
        assert_eq!(
            natives[0],
            (
                FakeHandle("top", size(50.0, 10.0)),
                Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 50.0,
                    height: 10.0
                }
            )
        );
        // inner stack starts at y = 10 (top's height) + 5 (spacing) = 15
        assert_eq!(
            natives[1],
            (
                FakeHandle("left", size(20.0, 20.0)),
                Rect {
                    x: 0.0,
                    y: 15.0,
                    width: 20.0,
                    height: 20.0
                }
            )
        );
        assert_eq!(
            natives[2],
            (
                FakeHandle("right", size(30.0, 20.0)),
                Rect {
                    x: 22.0,
                    y: 15.0,
                    width: 30.0,
                    height: 20.0
                }
            )
        );
    }

    #[test]
    fn stretch_default_fills_the_cross_axis_slot() {
        // Unlike the previous test, this one leaves alignment at its `Stretch` default — each
        // leaf should fill the *entire* stack width (the cross axis, for a vertical stack), not
        // just its own measured width.
        let tree = stack(
            Orientation::Vertical,
            0.0,
            vec![native("a", size(10.0, 20.0))],
        );
        let (natives, _) = split(layout_tree::<FakeHandle>(&tree, size(200.0, 100.0)));
        assert_eq!(
            natives[0].1,
            Rect {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 20.0
            }
        );
    }

    // `abstract_shape_has_no_commands_and_no_children` used to live here. It constructed a bare
    // `Shape`, set a fill on it, and asserted the tree still produced no paint commands — i.e. that
    // the base class has no render of its own. `Shape` is now `abstract_class`, so `#[class]` no
    // longer synthesizes `Shape::new()` and that state is unreachable by construction rather than by
    // assertion. The reachable half of what it covered — a shape with neither fill nor stroke paints
    // nothing — is exercised through a concrete `Rectangle` by
    // `shape_is_hit_testable_only_when_fill_or_stroke_is_set` below.

    #[test]
    fn empty_virtual_node_has_zero_size_and_no_leaves() {
        let tree = stack(Orientation::Vertical, 0.0, vec![]);
        let (natives, paints) = split(layout_tree::<FakeHandle>(&tree, size(100.0, 100.0)));
        assert!(natives.is_empty());
        assert!(paints.is_empty());
    }

    #[test]
    fn margin_shrinks_the_slot_an_element_is_arranged_into() {
        let tree: Rc<dyn UIElementExt> = FakeNativeControl::new(FakeHandle("a", size(10.0, 20.0)));
        tree.as_ui_element().set_margin(10.0);
        let (natives, _) = split(layout_tree::<FakeHandle>(&tree, size(100.0, 100.0)));
        assert_eq!(
            natives[0].1,
            Rect {
                x: 10.0,
                y: 10.0,
                width: 80.0,
                height: 80.0
            }
        );
    }

    #[test]
    fn explicit_width_and_height_override_the_elements_own_measured_size() {
        let tree: Rc<dyn UIElementExt> = FakeNativeControl::new(FakeHandle("a", size(10.0, 20.0)));
        tree.as_ui_element().set_width(50.0);
        tree.as_ui_element().set_height(5.0);
        // `Stretch` (the default) still governs slot placement; the explicit width/height above
        // constrains what `measure_override`'s own `available`/`desired` see, not the final
        // stretch-to-slot size — a non-`Stretch` alignment (below) is what actually surfaces the
        // explicit size in the arranged rect.
        tree.as_ui_element()
            .set_horizontal_alignment(HorizontalAlignment::Left);
        tree.as_ui_element()
            .set_vertical_alignment(VerticalAlignment::Top);
        let (natives, _) = split(layout_tree::<FakeHandle>(&tree, size(200.0, 200.0)));
        assert_eq!(
            natives[0].1,
            Rect {
                x: 0.0,
                y: 0.0,
                width: 50.0,
                height: 5.0
            }
        );
    }

    #[test]
    fn explicit_dimensions_remain_centered_inside_the_parent_slot() {
        let tree: Rc<dyn UIElementExt> = FakeNativeControl::new(FakeHandle("a", size(40.0, 50.0)));
        tree.as_ui_element().set_width(10.0);
        tree.as_ui_element().set_height(20.0);
        tree.as_ui_element()
            .set_horizontal_alignment(HorizontalAlignment::Center);
        tree.as_ui_element()
            .set_vertical_alignment(VerticalAlignment::Center);

        let (natives, _) = split(layout_tree::<FakeHandle>(&tree, size(100.0, 100.0)));

        assert_eq!(
            natives[0].1,
            Rect {
                x: 45.0,
                y: 40.0,
                width: 10.0,
                height: 20.0,
            }
        );
    }

    #[test]
    fn min_and_max_clamp_the_elements_own_measured_size() {
        let tree: Rc<dyn UIElementExt> = FakeNativeControl::new(FakeHandle("a", size(10.0, 20.0)));
        tree.as_ui_element().set_min_width(30.0);
        tree.as_ui_element().set_max_height(8.0);
        tree.as_ui_element()
            .set_horizontal_alignment(HorizontalAlignment::Left);
        tree.as_ui_element()
            .set_vertical_alignment(VerticalAlignment::Top);
        let (natives, _) = split(layout_tree::<FakeHandle>(&tree, size(200.0, 200.0)));
        assert_eq!(
            natives[0].1,
            Rect {
                x: 0.0,
                y: 0.0,
                width: 30.0,
                height: 8.0
            }
        );
    }

    #[test]
    fn arranged_width_height_and_offset_are_populated_after_layout() {
        let leaf = native("a", size(10.0, 20.0));
        leaf.as_ui_element()
            .set_horizontal_alignment(HorizontalAlignment::Left);
        leaf.as_ui_element()
            .set_vertical_alignment(VerticalAlignment::Top);
        let root = stack(
            Orientation::Vertical,
            5.0,
            vec![native("top", size(50.0, 10.0)), Rc::clone(&leaf)],
        );
        layout_tree::<FakeHandle>(&root, size(200.0, 200.0));

        assert_eq!(root.arranged_width(), Some(200.0));
        assert_eq!(root.arranged_height(), Some(200.0));
        assert_eq!(
            root.arranged_offset(),
            Some(Point { x: 0.0, y: 0.0 }),
            "root has no parent to set its own offset"
        );
        // second stack child ("top" is 10 tall, spacing is 5) starts at y = 15, relative to the stack
        assert_eq!(leaf.arranged_offset(), Some(Point { x: 0.0, y: 15.0 }));
        assert_eq!(leaf.arranged_width(), Some(10.0));
        assert_eq!(leaf.arranged_height(), Some(20.0));
    }

    #[test]
    fn measured_size_and_arranged_state_are_none_before_layout_and_after_invalidate() {
        let leaf = native("a", size(10.0, 20.0));
        assert_eq!(leaf.measured_size(), None);
        assert_eq!(leaf.arranged_width(), None);
        assert_eq!(leaf.arranged_height(), None);
        assert_eq!(leaf.arranged_offset(), None);

        leaf.measure(size(200.0, 200.0));
        assert_eq!(leaf.measured_size(), Some(size(10.0, 20.0)));
        leaf.arrange(Rect {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 200.0,
        });
        assert!(leaf.arranged_width().is_some());
        assert!(leaf.arranged_height().is_some());
        assert!(leaf.arranged_offset().is_some());

        leaf.invalidate_arrange();
        assert!(
            leaf.measured_size().is_some(),
            "invalidate_arrange must not touch measured_size"
        );
        assert_eq!(leaf.arranged_width(), None);
        assert_eq!(leaf.arranged_height(), None);
        assert_eq!(leaf.arranged_offset(), None);

        leaf.arrange(Rect {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 200.0,
        });
        leaf.invalidate_measure();
        assert_eq!(leaf.measured_size(), None);
        assert_eq!(leaf.arranged_width(), None);
        assert_eq!(leaf.arranged_height(), None);
        assert_eq!(leaf.arranged_offset(), None);
    }

    #[test]
    fn non_stretch_alignment_keeps_the_elements_own_measured_size() {
        let tree: Rc<dyn UIElementExt> = FakeNativeControl::new(FakeHandle("a", size(10.0, 20.0)));
        tree.as_ui_element()
            .set_horizontal_alignment(HorizontalAlignment::Center);
        tree.as_ui_element()
            .set_vertical_alignment(VerticalAlignment::Center);
        let (natives, _) = split(layout_tree::<FakeHandle>(&tree, size(100.0, 100.0)));
        assert_eq!(
            natives[0].1,
            Rect {
                x: 45.0,
                y: 40.0,
                width: 10.0,
                height: 20.0
            }
        );
    }

    #[test]
    fn render_item_ordering_preserves_traversal_order_across_native_and_paint() {
        // A painting container containing a native leaf child: traversal visits the container
        // itself (pushing its `Paint`) before recursing into its child (pushing the child's
        // `Native`), so the combined list must come back `[Paint, Native]` — a backend replaying
        // this list in order therefore places the native leaf *in front of* the container's own
        // paint, matching the source tree's parent-then-child nesting instead of an accidental
        // "all natives first" or "all paints first" batching.
        // `PaintingContainer` isn't `#[class]`-managed (it hand-implements `UIElementExt` directly,
        // above), so it has no auto-generated `new()` to reach `UIElement::construct`'s real,
        // hidden-weak-parameter form through — it calls the internal `__class_construct` directly,
        // via its own hand-rolled `Rc::new_cyclic`, exactly the shape any non-`#[class]` code needing
        // a real self-weak has to use.
        let tree = Rc::<PaintingContainer>::new_cyclic(|weak: &Weak<PaintingContainer>| {
            let weak: Weak<dyn UIElementExt> = weak.clone();
            PaintingContainer {
                base: UIElement::__class_construct(weak),
            }
        });
        tree.as_ui_element()
            .visual_collection
            .add(native("child", size(10.0, 10.0)));
        let tree: Rc<dyn UIElementExt> = tree;
        let render_tree = layout_tree::<FakeHandle>(&tree, size(50.0, 50.0));
        assert!(matches!(
            render_tree.root.commands[0],
            RenderCommand::FillRoundedRect { .. }
        ));
        assert!(matches!(
            render_tree.root.children[0].commands[0],
            RenderCommand::NativeControl { .. }
        ));
    }

    #[test]
    fn layout_background_is_transparent_by_default_and_paints_before_children() {
        let layout = VerticalLayout::new();
        let child = Rectangle::new();
        child.set_fill(Some(Brush::Solid(Color::rgb(10, 20, 30))));
        child.set_width(20.0);
        child.set_height(20.0);
        layout.children().add(child);

        let root: Rc<dyn UIElementExt> = layout.clone();
        let transparent = layout_tree::<FakeHandle>(&root, size(40.0, 40.0));
        assert!(transparent.root.commands.is_empty());
        assert!(matches!(
            transparent.root.children[0].commands[0],
            RenderCommand::FillRoundedRect { .. }
        ));

        layout.set_background(Some(Brush::Solid(Color::rgb(1, 2, 3))));
        let painted = layout_tree::<FakeHandle>(&root, size(40.0, 40.0));
        assert!(matches!(
            painted.root.commands[0],
            RenderCommand::FillRect { .. }
        ));
        assert!(matches!(
            painted.root.children[0].commands[0],
            RenderCommand::FillRoundedRect { .. }
        ));
    }

    #[test]
    fn render_tree_indexes_stable_visual_ids_and_marks_only_target_group_dirty() {
        let child = native("child", size(10.0, 10.0));
        let root = stack(Orientation::Vertical, 0.0, vec![Rc::clone(&child)]);
        let mut render_tree = layout_tree::<FakeHandle>(&root, size(40.0, 40.0));
        let child_id = child.render_group_id();
        assert!(render_tree.group_paths.contains_key(&child_id));
        assert!(render_tree.visual_index[&child_id].upgrade().is_some());
        assert!(!render_tree.root.is_dirty);
        assert!(render_tree.mark_dirty(child_id));
        assert!(!render_tree.root.is_dirty);
        assert!(render_tree.root.children[0].is_dirty);
    }

    #[test]
    fn reconcile_reuses_matching_root_and_discards_removed_visual_indexes() {
        let first = native("first", size(10.0, 10.0));
        let second = native("second", size(10.0, 10.0));
        let root = stack(
            Orientation::Vertical,
            0.0,
            vec![Rc::clone(&first), Rc::clone(&second)],
        );
        layout_root(&root, size(40.0, 40.0));
        let mut render_tree = RenderTree::new::<FakeHandle>(&root);
        let root_address = (&render_tree.root as *const RenderGroup) as usize;
        let first_id = first.render_group_id();
        let second_id = second.render_group_id();

        assert!(render_tree.mark_dirty(first_id));
        layout_root(&root, size(40.0, 40.0));
        assert!(render_tree.reconcile::<FakeHandle>(&root));
        assert_eq!(
            root_address,
            (&render_tree.root as *const RenderGroup) as usize
        );
        assert!(render_tree.group_paths.contains_key(&first_id));
        assert!(render_tree.group_paths.contains_key(&second_id));

        assert!(root.as_ui_element().visual_collection.remove(&second));
        layout_root(&root, size(40.0, 40.0));
        assert!(render_tree.reconcile::<FakeHandle>(&root));
        assert!(!render_tree.group_paths.contains_key(&second_id));
        assert!(!render_tree.mark_dirty(second_id));
    }

    #[test]
    fn reconcile_removes_a_render_group_when_its_element_becomes_collapsed() {
        let child = native("child", size(10.0, 10.0));
        let root = stack(Orientation::Vertical, 0.0, vec![Rc::clone(&child)]);
        layout_root(&root, size(40.0, 40.0));
        let mut render_tree = RenderTree::new::<FakeHandle>(&root);
        let child_id = child.render_group_id();
        assert!(render_tree.group_paths.contains_key(&child_id));
        assert_eq!(render_tree.root.children.len(), 1);

        child.as_ui_element().set_visibility(Visibility::Collapsed);
        layout_root(&root, size(40.0, 40.0));
        assert!(render_tree.reconcile::<FakeHandle>(&root));

        assert!(
            render_tree.root.children.is_empty(),
            "the Collapsed child's RenderGroup must be removed from its parent's children"
        );
        assert!(
            !render_tree.group_paths.contains_key(&child_id),
            "a removed RenderGroup's id must not remain in group_paths"
        );
        assert!(
            !render_tree.mark_dirty(child_id),
            "mark_dirty on a removed group's id must return false, not panic or find a stale entry"
        );
    }

    #[test]
    fn reconcile_recreates_a_render_group_when_its_element_becomes_visible_again() {
        let child = native("child", size(10.0, 10.0));
        child.as_ui_element().set_visibility(Visibility::Collapsed);
        let root = stack(Orientation::Vertical, 0.0, vec![Rc::clone(&child)]);
        layout_root(&root, size(40.0, 40.0));
        let mut render_tree = RenderTree::new::<FakeHandle>(&root);
        let child_id = child.render_group_id();
        assert!(render_tree.root.children.is_empty());
        assert!(!render_tree.group_paths.contains_key(&child_id));

        child.as_ui_element().set_visibility(Visibility::Visible);
        layout_root(&root, size(40.0, 40.0));
        assert!(render_tree.reconcile::<FakeHandle>(&root));

        assert_eq!(
            render_tree.root.children.len(),
            1,
            "the now-Visible child's RenderGroup must be (re)created"
        );
        assert_eq!(render_tree.root.children[0].id, child_id);
        assert!(render_tree.group_paths.contains_key(&child_id));
        assert!(render_tree.mark_dirty(child_id));
    }

    #[test]
    fn reconcile_collapses_the_root_group_itself_when_the_root_becomes_collapsed() {
        // Mirrors `RenderTree::new`'s own non-participating-root fallback (see its doc comment):
        // a Collapsed *root* can't simply vanish (a `RenderTree` always has a root group), so
        // `reconcile` must fold it down to the same empty shape `new` would produce instead of
        // asking `reconcile_render_group` (which assumes its `elem` participates) to reconcile a
        // group that shouldn't exist at all.
        let child = native("child", size(10.0, 10.0));
        let root = stack(Orientation::Vertical, 0.0, vec![Rc::clone(&child)]);
        layout_root(&root, size(40.0, 40.0));
        let mut render_tree = RenderTree::new::<FakeHandle>(&root);
        let root_id = root.render_group_id();
        let child_id = child.render_group_id();
        assert_eq!(render_tree.root.children.len(), 1);

        root.as_ui_element().set_visibility(Visibility::Collapsed);
        layout_root(&root, size(40.0, 40.0));
        assert!(render_tree.reconcile::<FakeHandle>(&root));

        assert_eq!(
            render_tree.root_id(),
            root_id,
            "the root's own group id never changes"
        );
        assert!(render_tree.root.children.is_empty());
        assert!(render_tree.root.commands.is_empty());
        assert!(render_tree.group_paths.contains_key(&root_id));
        assert_eq!(render_tree.group_paths[&root_id], Vec::<usize>::new());
        assert!(
            !render_tree.group_paths.contains_key(&child_id),
            "the collapsed root's former child must not remain indexed"
        );
    }

    #[test]
    fn reconcile_rejects_a_different_content_root() {
        let first = native("first", size(10.0, 10.0));
        let second = native("second", size(10.0, 10.0));
        layout_root(&first, size(20.0, 20.0));
        let mut render_tree = RenderTree::new::<FakeHandle>(&first);
        layout_root(&second, size(20.0, 20.0));
        assert!(!render_tree.reconcile::<FakeHandle>(&second));
        assert_eq!(render_tree.root_id(), first.render_group_id());
    }

    #[test]
    fn reconcile_rerecords_native_commands_when_only_arranged_size_changes() {
        let root = native("root", size(10.0, 10.0));
        layout_root(&root, size(40.0, 30.0));
        let mut render_tree = RenderTree::new::<FakeHandle>(&root);
        let native_rect = |tree: &RenderTree| match &tree.root.commands[0] {
            RenderCommand::NativeControl { rect, .. } => *rect,
            _ => panic!("expected native command"),
        };
        assert_eq!(native_rect(&render_tree).width, 40.0);

        layout_root(&root, size(100.0, 80.0));
        assert!(render_tree.reconcile::<FakeHandle>(&root));
        assert_eq!(native_rect(&render_tree).width, 100.0);
        assert_eq!(native_rect(&render_tree).height, 80.0);
    }

    #[test]
    fn clip_to_bounds_defaults_false_and_inherits_from_visual_parent() {
        let child = native("child", size(10.0, 10.0));
        let root = stack(Orientation::Vertical, 0.0, vec![Rc::clone(&child)]);
        assert!(!child.clip_to_bounds());
        root.set_clip_to_bounds(Some(true));
        assert!(child.clip_to_bounds());
        let render_tree = layout_tree::<FakeHandle>(&root, size(40.0, 40.0));
        assert!(render_tree.root.clip.is_some());
        assert!(render_tree.root.children[0].clip.is_some());
        child.set_clip_to_bounds(Some(false));
        let render_tree = layout_tree::<FakeHandle>(&root, size(40.0, 40.0));
        assert!(render_tree.root.children[0].clip.is_none());
    }

    #[test]
    fn dispatch_routed_bubbles_and_stops_at_handled() {
        let leaf = native("a", size(10.0, 20.0));
        let root = stack(Orientation::Vertical, 0.0, vec![Rc::clone(&leaf)]);

        let leaf_calls = Rc::new(RefCell::new(0));
        let root_calls = Rc::new(RefCell::new(0));
        {
            let leaf_calls = Rc::clone(&leaf_calls);
            leaf.as_ui_element().register_routed_handler::<()>(
                "on_click",
                Box::new(move |_, _| *leaf_calls.borrow_mut() += 1),
            );
        }
        {
            let root_calls = Rc::clone(&root_calls);
            root.as_ui_element().register_routed_handler::<()>(
                "on_click",
                Box::new(move |_, args| {
                    *root_calls.borrow_mut() += 1;
                    args.handled.set(true);
                }),
            );
        }

        let args = RoutedEventArgs::default();
        dispatch_routed(&leaf, "on_click", &(), &args);
        assert_eq!(*leaf_calls.borrow(), 1);
        assert_eq!(*root_calls.borrow(), 1);
        assert!(args.handled.get());
    }

    #[test]
    fn handled_events_too_handlers_still_run_after_a_descendant_handles_the_event() {
        let leaf = native("a", size(10.0, 20.0));
        let middle = stack(Orientation::Vertical, 0.0, vec![Rc::clone(&leaf)]);
        let root = stack(Orientation::Vertical, 0.0, vec![Rc::clone(&middle)]);

        let ordinary_calls = Rc::new(RefCell::new(0));
        let handled_too_calls = Rc::new(RefCell::new(Vec::new()));
        leaf.as_ui_element()
            .register_routed_handler::<()>("on_click", Box::new(|_, args| args.handled.set(true)));
        {
            let ordinary_calls = Rc::clone(&ordinary_calls);
            middle.as_ui_element().register_routed_handler::<()>(
                "on_click",
                Box::new(move |_, _| *ordinary_calls.borrow_mut() += 1),
            );
        }
        {
            let handled_too_calls = Rc::clone(&handled_too_calls);
            root.as_ui_element()
                .register_routed_handler_handled_too::<()>(
                    "on_click",
                    Box::new(move |_, args| {
                        handled_too_calls.borrow_mut().push(args.handled.get())
                    }),
                );
        }

        let args = RoutedEventArgs::default();
        dispatch_routed(&leaf, "on_click", &(), &args);
        // The ordinary ancestor handler is skipped; the handled-too one runs and sees `handled`.
        assert_eq!(*ordinary_calls.borrow(), 0);
        assert_eq!(*handled_too_calls.borrow(), vec![true]);

        // An unhandled event reaches both kinds.
        let free_args = RoutedEventArgs::default();
        dispatch_routed(&middle, "on_click", &(), &free_args);
        assert_eq!(*ordinary_calls.borrow(), 1);
        assert_eq!(*handled_too_calls.borrow(), vec![true, false]);
    }

    #[test]
    fn dispatch_routed_bubbles_via_visual_parent_even_without_a_logical_parent() {
        // `leaf` is added straight to `root`'s `visual_collection`, bypassing
        // `UIElementCollection` — matching `logical_and_visual_collections_keep_their_parent_relationships_separate`'s
        // `visual_only` pattern, `leaf` ends up with a `visual_parent` but no logical `parent()` at
        // all. `dispatch_routed` must still reach `root`'s handler, since it bubbles via
        // `visual_parent` (real WinUI3 semantics), not the Logical `parent` chain.
        let leaf = native("a", size(10.0, 20.0));
        let root = stack(Orientation::Vertical, 0.0, vec![]);
        root.as_ui_element().visual_collection.add(leaf.clone());
        assert!(leaf.parent().is_none());

        let root_calls = Rc::new(RefCell::new(0));
        {
            let root_calls = Rc::clone(&root_calls);
            root.as_ui_element().register_routed_handler::<()>(
                "on_click",
                Box::new(move |_, _| *root_calls.borrow_mut() += 1),
            );
        }

        let args = RoutedEventArgs::default();
        dispatch_routed(&leaf, "on_click", &(), &args);
        assert_eq!(*root_calls.borrow(), 1);
    }

    #[test]
    fn collapsed_leaf_has_zero_size_and_produces_no_render_item() {
        let tree = native("a", size(10.0, 20.0));
        tree.as_ui_element().set_visibility(Visibility::Collapsed);
        let (natives, paints) = split(layout_tree::<FakeHandle>(&tree, size(100.0, 100.0)));
        assert!(natives.is_empty());
        assert!(paints.is_empty());
        assert_eq!(tree.arranged_width(), Some(0.0));
        assert_eq!(tree.arranged_height(), Some(0.0));
    }

    #[test]
    fn collapsed_child_is_excluded_from_stack_layout() {
        let collapsed = native("collapsed", size(50.0, 50.0));
        collapsed
            .as_ui_element()
            .set_visibility(Visibility::Collapsed);
        let visible = native("visible", size(30.0, 10.0));
        visible
            .as_ui_element()
            .set_horizontal_alignment(HorizontalAlignment::Left);
        visible
            .as_ui_element()
            .set_vertical_alignment(VerticalAlignment::Top);
        let tree = stack(
            Orientation::Vertical,
            5.0,
            vec![Rc::clone(&collapsed), Rc::clone(&visible)],
        );

        let (natives, _) = split(layout_tree::<FakeHandle>(&tree, size(200.0, 200.0)));
        // `VerticalLayout::measure_override`/`arrange_override` exclude non-participating children
        // from `stack_natural_size`/`stack_arrange`'s own inputs, so the collapsed child doesn't
        // strand a `spacing` gap around itself — `visible` starts at y = 0.0, as if `collapsed`
        // weren't in the stack at all.
        assert_eq!(
            natives,
            vec![(
                FakeHandle("visible", size(30.0, 10.0)),
                Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 30.0,
                    height: 10.0
                }
            )]
        );
        // The collapsed child is still `arrange`d (its own `arranged_*` reset to zero), just
        // excluded from the participating-children rect list above.
        assert_eq!(collapsed.arranged_width(), Some(0.0));
        assert_eq!(collapsed.arranged_height(), Some(0.0));
        assert_eq!(collapsed.arranged_offset(), Some(Point { x: 0.0, y: 0.0 }));
    }

    #[test]
    fn collapsed_containers_subtree_is_entirely_excluded() {
        let leaf = native("child", size(10.0, 10.0));
        let container = stack(Orientation::Vertical, 0.0, vec![Rc::clone(&leaf)]);
        container
            .as_ui_element()
            .set_visibility(Visibility::Collapsed);

        let (natives, paints) = split(layout_tree::<FakeHandle>(&container, size(100.0, 100.0)));
        assert!(natives.is_empty());
        assert!(paints.is_empty());
        assert_eq!(
            leaf.visibility(),
            Visibility::Visible,
            "the child itself was never made Collapsed"
        );
    }

    #[test]
    fn collapsed_element_is_excluded_from_hit_test() {
        let tree = native("a", size(10.0, 20.0));
        tree.as_ui_element().set_visibility(Visibility::Collapsed);
        layout_tree::<FakeHandle>(&tree, size(100.0, 100.0));
        assert!(hit_test(&tree, Point { x: 5.0, y: 5.0 }).is_none());
    }

    #[test]
    fn layout_containers_are_transparent_to_hit_testing() {
        let leaf = native("leaf", size(10.0, 10.0));
        leaf.as_ui_element()
            .set_horizontal_alignment(HorizontalAlignment::Left);
        leaf.as_ui_element()
            .set_vertical_alignment(VerticalAlignment::Top);
        let root = stack(Orientation::Vertical, 0.0, vec![Rc::clone(&leaf)]);
        layout_tree::<FakeHandle>(&root, size(100.0, 100.0));

        assert!(Rc::ptr_eq(
            &hit_test(&root, Point { x: 5.0, y: 5.0 }).expect("leaf should be hit"),
            &leaf
        ));
        assert!(
            hit_test(&root, Point { x: 50.0, y: 50.0 }).is_none(),
            "VerticalLayout has no Background/Fill concept, so its own empty space must not be \
             hit-testable — a click there falls through instead of hitting the container itself"
        );
    }

    #[test]
    fn shape_is_hit_testable_only_when_fill_or_stroke_is_set() {
        let transparent = rectangle(None, None);
        transparent.as_ui_element().set_width(20.0);
        transparent.as_ui_element().set_height(20.0);
        layout_tree::<FakeHandle>(&transparent, size(100.0, 100.0));
        assert!(
            hit_test(&transparent, Point { x: 5.0, y: 5.0 }).is_none(),
            "a Shape with neither fill nor stroke set paints nothing, so it must not be hit"
        );

        let filled = rectangle(Some("#ffffff"), None);
        filled.as_ui_element().set_width(20.0);
        filled.as_ui_element().set_height(20.0);
        layout_tree::<FakeHandle>(&filled, size(100.0, 100.0));
        assert!(hit_test(&filled, Point { x: 5.0, y: 5.0 }).is_some());
    }

    #[test]
    fn hit_test_uses_presentation_transform_and_keeps_zero_opacity_hit_testable() {
        let leaf = native("leaf", size(20.0, 20.0));
        leaf.as_ui_element()
            .set_horizontal_alignment(HorizontalAlignment::Left);
        leaf.as_ui_element()
            .set_vertical_alignment(VerticalAlignment::Top);
        let root = stack(Orientation::Vertical, 0.0, vec![Rc::clone(&leaf)]);
        leaf.as_ui_element()
            .set_visual_transform(VisualTransform::new(Vector { x: 30.0, y: 0.0 }, 1.0, 0.0));
        layout_tree::<FakeHandle>(&root, size(100.0, 100.0));

        assert!(Rc::ptr_eq(
            &hit_test(&root, Point { x: 35.0, y: 5.0 }).expect("translated leaf should be hit"),
            &leaf
        ));
        assert!(hit_test(&root, Point { x: 5.0, y: 5.0 }).is_none());

        leaf.as_ui_element().set_opacity(0.0);
        assert!(hit_test(&root, Point { x: 35.0, y: 5.0 }).is_some());

        leaf.as_ui_element()
            .set_visual_transform(VisualTransform::new(Vector::default(), 0.0, 0.0));
        assert!(hit_test(&root, Point { x: 5.0, y: 5.0 }).is_none());
    }

    #[test]
    fn exiting_visual_is_rendered_but_not_laid_out_or_hit_tested() {
        let leaf = native("leaf", size(20.0, 20.0));
        let root = stack(Orientation::Vertical, 0.0, vec![Rc::clone(&leaf)]);
        let _ = layout_tree::<FakeHandle>(&root, size(100.0, 100.0));
        assert!(hit_test(&root, Point { x: 5.0, y: 5.0 }).is_some());

        assert!(begin_exit(&leaf));
        let visual_collection = root.as_ui_element().visual_collection.clone();
        assert!(!visual_collection.remove(&leaf));
        let replacement = native("replacement", size(20.0, 20.0));
        visual_collection.insert(0, Rc::clone(&replacement));
        let visual = root.visual_children();
        assert_eq!(visual.len(), 2);
        assert!(
            Rc::ptr_eq(&visual[0], &leaf),
            "outgoing child keeps its visual slot"
        );
        assert!(Rc::ptr_eq(&visual[1], &replacement));
        let tree = layout_tree::<FakeHandle>(&root, size(100.0, 100.0));
        assert_eq!(
            leaf.arranged_width(),
            Some(100.0),
            "an exiting Visual retains its last arranged geometry for rendering"
        );
        let hit = hit_test(&root, Point { x: 5.0, y: 5.0 });
        assert!(hit.is_some_and(|hit| !Rc::ptr_eq(&hit, &leaf)));
        assert_eq!(tree.root.children.len(), 2, "exit node remains renderable");

        assert!(finish_exit(&leaf));
        let visual = root.visual_children();
        assert_eq!(visual.len(), 1);
        assert!(Rc::ptr_eq(&visual[0], &replacement));
        assert!(!finish_exit(&leaf), "exit completion must be exactly once");
    }

    #[test]
    fn hit_test_visible_false_excludes_the_element_and_its_whole_subtree() {
        let leaf = native("leaf", size(10.0, 10.0));
        let root = stack(Orientation::Vertical, 0.0, vec![Rc::clone(&leaf)]);
        layout_tree::<FakeHandle>(&root, size(100.0, 100.0));
        assert!(hit_test(&root, Point { x: 5.0, y: 5.0 }).is_some());

        root.as_ui_element().set_hit_test_visible(false);
        assert!(
            hit_test(&root, Point { x: 5.0, y: 5.0 }).is_none(),
            "IsHitTestVisible=false must exclude descendants too, not just the element itself"
        );
    }

    #[test]
    fn hit_test_respects_clip_to_bounds_only_when_actually_set() {
        // Manually wired (not `stack`/`layout_tree`) so the child's own arranged rect can be made
        // to genuinely overflow its parent's — exactly the case `clip_to_bounds` distinguishes.
        let child = native("child", size(50.0, 50.0));
        let parent = native("parent", size(20.0, 20.0));
        parent.as_ui_element().visual_collection.add(child.clone());
        parent.arrange(Rect {
            x: 0.0,
            y: 0.0,
            width: 20.0,
            height: 20.0,
        });
        child.arrange(Rect {
            x: 0.0,
            y: 0.0,
            width: 50.0,
            height: 50.0,
        });

        // Outside `parent`'s own 20x20 rect but inside the child's own (overflowing) 50x50 one.
        let outside_parent = Point { x: 30.0, y: 30.0 };
        assert!(
            Rc::ptr_eq(
                &hit_test(&parent, outside_parent).expect("the overflowing child should be hit"),
                &child
            ),
            "clip_to_bounds defaults to false, so a child rendering outside its parent's own \
             bounds must remain hit-testable there"
        );

        parent.as_ui_element().set_clip_to_bounds(Some(true));
        assert!(
            hit_test(&parent, outside_parent).is_none(),
            "once the parent opts into clip_to_bounds, the overflowing child must be excluded too"
        );
    }

    #[test]
    fn pointer_entered_exited_do_not_refire_on_a_still_hovered_shared_ancestor() {
        let leaf_a = native("a", size(10.0, 10.0));
        let leaf_b = native("b", size(10.0, 10.0));
        let root = stack(
            Orientation::Vertical,
            0.0,
            vec![Rc::clone(&leaf_a), Rc::clone(&leaf_b)],
        );
        layout_tree::<FakeHandle>(&root, size(100.0, 100.0));

        let root_entered =
            count_calls::<crate::input::PointerEventArgs>(&root, "on_pointer_entered");
        let root_exited = count_calls::<crate::input::PointerEventArgs>(&root, "on_pointer_exited");
        let a_entered =
            count_calls::<crate::input::PointerEventArgs>(&leaf_a, "on_pointer_entered");
        let a_exited = count_calls::<crate::input::PointerEventArgs>(&leaf_a, "on_pointer_exited");
        let b_entered =
            count_calls::<crate::input::PointerEventArgs>(&leaf_b, "on_pointer_entered");
        let b_exited = count_calls::<crate::input::PointerEventArgs>(&leaf_b, "on_pointer_exited");

        let dispatcher = crate::input::PointerDispatcher::new();
        let focus = crate::focus::FocusTracker::new();
        dispatcher.handle(&root, &focus, move_event(5.0, 5.0));
        assert_eq!((*root_entered.borrow(), *a_entered.borrow()), (1, 1));

        // Moving from `a` to `b` (both under the same `root`) must not re-fire `root`'s own
        // Entered/Exited — it was, and remains, hovered throughout.
        dispatcher.handle(&root, &focus, move_event(5.0, 15.0));
        assert_eq!(*a_exited.borrow(), 1);
        assert_eq!(*b_entered.borrow(), 1);
        assert_eq!((*root_entered.borrow(), *root_exited.borrow()), (1, 0));

        // Moving off the tree entirely (into the layout's own transparent empty space) exits
        // everything, `root` included.
        dispatcher.handle(&root, &focus, move_event(5.0, 90.0));
        assert_eq!(*b_exited.borrow(), 1);
        assert_eq!(*root_exited.borrow(), 1);
    }

    #[test]
    fn pointer_dispatch_preserves_backend_screen_position_during_capture() {
        let leaf = native("a", size(50.0, 50.0));
        layout_tree::<FakeHandle>(&leaf, size(50.0, 50.0));
        let observed = Rc::new(RefCell::new(Vec::<crate::input::PointerEventArgs>::new()));
        for event_name in [
            "on_pointer_pressed",
            "on_pointer_moved",
            "on_pointer_released",
        ] {
            let observed = Rc::clone(&observed);
            leaf.as_ui_element()
                .register_routed_handler::<crate::input::PointerEventArgs>(
                    event_name,
                    Box::new(move |args, _| observed.borrow_mut().push(*args)),
                );
        }

        let dispatcher = crate::input::PointerDispatcher::new();
        let focus = crate::focus::FocusTracker::new();
        let event = |kind, position, screen_position, timestamp_ms| crate::input::RawPointerEvent {
            kind,
            position,
            screen_position: Some(screen_position),
            modifiers: crate::input::KeyModifiers::default(),
            timestamp_ms,
        };
        dispatcher.handle(
            &leaf,
            &focus,
            event(
                crate::input::RawPointerEventKind::Pressed(crate::input::MouseButton::Left),
                Point { x: 5.0, y: 5.0 },
                Point { x: 105.0, y: 205.0 },
                0.0,
            ),
        );
        dispatcher.handle(
            &leaf,
            &focus,
            event(
                crate::input::RawPointerEventKind::Moved,
                Point { x: 500.0, y: 500.0 },
                Point { x: 600.0, y: 700.0 },
                1.0,
            ),
        );
        dispatcher.handle(
            &leaf,
            &focus,
            event(
                crate::input::RawPointerEventKind::Released(crate::input::MouseButton::Left),
                Point { x: 500.0, y: 500.0 },
                Point { x: 600.0, y: 700.0 },
                2.0,
            ),
        );

        let observed = observed.borrow();
        assert_eq!(observed.len(), 3);
        assert_eq!(
            observed
                .iter()
                .map(|args| args.screen_position)
                .collect::<Vec<_>>(),
            vec![
                Some(Point { x: 105.0, y: 205.0 }),
                Some(Point { x: 600.0, y: 700.0 }),
                Some(Point { x: 600.0, y: 700.0 }),
            ]
        );
    }

    #[test]
    fn pointer_cancel_uses_latest_payload_once_and_suppresses_release_and_tap() {
        let leaf = native("a", size(50.0, 50.0));
        layout_tree::<FakeHandle>(&leaf, size(50.0, 50.0));
        let canceled = Rc::new(RefCell::new(Vec::<crate::input::PointerEventArgs>::new()));
        let observed = Rc::clone(&canceled);
        leaf.as_ui_element()
            .register_routed_handler::<crate::input::PointerEventArgs>(
                "on_pointer_canceled",
                Box::new(move |args, _| observed.borrow_mut().push(*args)),
            );
        let released = count_calls::<crate::input::PointerEventArgs>(&leaf, "on_pointer_released");
        let tapped = count_calls::<crate::input::TappedEventArgs>(&leaf, "on_tapped");

        let dispatcher = crate::input::PointerDispatcher::new();
        let focus = crate::focus::FocusTracker::new();
        dispatcher.handle(
            &leaf,
            &focus,
            press_event(5.0, 5.0, crate::input::MouseButton::Left, 0.0),
        );
        dispatcher.handle(
            &leaf,
            &focus,
            crate::input::RawPointerEvent {
                kind: crate::input::RawPointerEventKind::Moved,
                position: Point { x: 60.0, y: 70.0 },
                screen_position: Some(Point { x: 160.0, y: 270.0 }),
                modifiers: crate::input::KeyModifiers {
                    shift: true,
                    ..Default::default()
                },
                timestamp_ms: 1.0,
            },
        );
        dispatcher.handle(
            &leaf,
            &focus,
            crate::input::RawPointerEvent {
                kind: crate::input::RawPointerEventKind::Canceled,
                position: Point { x: 65.0, y: 75.0 },
                screen_position: Some(Point { x: 165.0, y: 275.0 }),
                modifiers: crate::input::KeyModifiers {
                    alt: true,
                    ..Default::default()
                },
                timestamp_ms: 2.0,
            },
        );
        dispatcher.handle(&leaf, &focus, cancel_event(80.0, 90.0));

        assert_eq!(
            *canceled.borrow(),
            vec![crate::input::PointerEventArgs {
                position: Point { x: 65.0, y: 75.0 },
                screen_position: Some(Point { x: 165.0, y: 275.0 }),
                button: None,
                modifiers: crate::input::KeyModifiers {
                    alt: true,
                    ..Default::default()
                },
            }]
        );
        assert_eq!(*released.borrow(), 0);
        assert_eq!(*tapped.borrow(), 0);
    }

    #[test]
    fn pointer_cancel_restores_fresh_hit_testing() {
        let leaf_a = native("a", size(10.0, 10.0));
        let leaf_b = native("b", size(10.0, 10.0));
        let root = stack(
            Orientation::Vertical,
            0.0,
            vec![Rc::clone(&leaf_a), Rc::clone(&leaf_b)],
        );
        layout_tree::<FakeHandle>(&root, size(20.0, 20.0));
        let moved_a = count_calls::<crate::input::PointerEventArgs>(&leaf_a, "on_pointer_moved");
        let moved_b = count_calls::<crate::input::PointerEventArgs>(&leaf_b, "on_pointer_moved");
        let canceled_root =
            count_calls::<crate::input::PointerEventArgs>(&root, "on_pointer_canceled");

        let dispatcher = crate::input::PointerDispatcher::new();
        let focus = crate::focus::FocusTracker::new();
        dispatcher.handle(
            &root,
            &focus,
            press_event(5.0, 5.0, crate::input::MouseButton::Left, 0.0),
        );
        dispatcher.handle(&root, &focus, cancel_event(5.0, 5.0));
        dispatcher.handle(&root, &focus, move_event(5.0, 15.0));

        assert_eq!(*moved_a.borrow(), 0);
        assert_eq!(*moved_b.borrow(), 1);
        assert_eq!(*canceled_root.borrow(), 1, "cancellation must bubble");
    }

    #[test]
    fn pointer_cancel_breaks_the_double_tap_streak() {
        let leaf = native("a", size(50.0, 50.0));
        layout_tree::<FakeHandle>(&leaf, size(50.0, 50.0));
        let double_tapped = count_calls::<crate::input::TappedEventArgs>(&leaf, "on_double_tapped");
        let dispatcher = crate::input::PointerDispatcher::new();
        let focus = crate::focus::FocusTracker::new();

        dispatcher.handle(
            &leaf,
            &focus,
            press_event(5.0, 5.0, crate::input::MouseButton::Left, 0.0),
        );
        dispatcher.handle(
            &leaf,
            &focus,
            release_event(5.0, 5.0, crate::input::MouseButton::Left, 10.0),
        );
        // Native capture-loss can arrive after the normal release path. With no active Core
        // capture it is an idempotent no-op and must not erase the completed tap streak.
        dispatcher.handle(&leaf, &focus, cancel_event(5.0, 5.0));
        dispatcher.handle(
            &leaf,
            &focus,
            press_event(5.0, 5.0, crate::input::MouseButton::Left, 100.0),
        );
        dispatcher.handle(&leaf, &focus, cancel_event(5.0, 5.0));
        dispatcher.handle(
            &leaf,
            &focus,
            press_event(5.0, 5.0, crate::input::MouseButton::Left, 200.0),
        );
        dispatcher.handle(
            &leaf,
            &focus,
            release_event(5.0, 5.0, crate::input::MouseButton::Left, 210.0),
        );

        assert_eq!(*double_tapped.borrow(), 0);
    }

    #[test]
    fn pointer_cancel_is_reentrant_safe_from_pressed_handler() {
        let leaf = native("a", size(50.0, 50.0));
        layout_tree::<FakeHandle>(&leaf, size(50.0, 50.0));
        let dispatcher = Rc::new(crate::input::PointerDispatcher::new());
        let weak_dispatcher = Rc::downgrade(&dispatcher);
        leaf.as_ui_element()
            .register_routed_handler::<crate::input::PointerEventArgs>(
                "on_pointer_pressed",
                Box::new(move |_, _| {
                    weak_dispatcher
                        .upgrade()
                        .expect("dispatcher should outlive dispatch")
                        .cancel();
                }),
            );
        let canceled = count_calls::<crate::input::PointerEventArgs>(&leaf, "on_pointer_canceled");
        let focus = crate::focus::FocusTracker::new();

        dispatcher.handle(
            &leaf,
            &focus,
            press_event(5.0, 5.0, crate::input::MouseButton::Left, 0.0),
        );

        assert_eq!(*canceled.borrow(), 1);
        assert!(!dispatcher.cancel());
    }

    #[test]
    fn subtree_cancel_during_release_clears_all_retained_state_without_active_press() {
        let leaf = native("a", size(50.0, 50.0));
        layout_tree::<FakeHandle>(&leaf, size(50.0, 50.0));
        let dispatcher = Rc::new(crate::input::PointerDispatcher::new());
        let focus = crate::focus::FocusTracker::new();
        let tapped = count_calls::<crate::input::TappedEventArgs>(&leaf, "on_tapped");

        // Seed both the completed-tap record and hover chain with references to `leaf`.
        dispatcher.handle(
            &leaf,
            &focus,
            press_event(5.0, 5.0, crate::input::MouseButton::Left, 0.0),
        );
        dispatcher.handle(
            &leaf,
            &focus,
            release_event(5.0, 5.0, crate::input::MouseButton::Left, 10.0),
        );
        assert_eq!(*tapped.borrow(), 1);
        *tapped.borrow_mut() = 0;

        let weak_dispatcher = Rc::downgrade(&dispatcher);
        let weak_subtree = Rc::downgrade(&leaf);
        leaf.as_ui_element()
            .register_routed_handler::<crate::input::PointerEventArgs>(
                "on_pointer_released",
                Box::new(move |_, _| {
                    let dispatcher = weak_dispatcher
                        .upgrade()
                        .expect("dispatcher should outlive dispatch");
                    let subtree = weak_subtree
                        .upgrade()
                        .expect("subtree should remain alive during release dispatch");
                    // `prepare_release` has already removed the active press. The subtree cleanup
                    // must still remove the pending tap, prior tap record, and hover chain.
                    assert!(!dispatcher.cancel_for_subtree(&subtree));
                }),
            );

        dispatcher.handle(
            &leaf,
            &focus,
            press_event(5.0, 5.0, crate::input::MouseButton::Left, 100.0),
        );
        dispatcher.handle(
            &leaf,
            &focus,
            release_event(5.0, 5.0, crate::input::MouseButton::Left, 110.0),
        );

        assert_eq!(*tapped.borrow(), 0);
        assert!(!dispatcher.cancel_for_subtree(&leaf));
        let weak_leaf = Rc::downgrade(&leaf);
        drop(leaf);
        assert!(
            weak_leaf.upgrade().is_none(),
            "pending_tap, last_tap, and last_hover must not retain the removed subtree"
        );
    }

    #[test]
    fn tap_fires_even_after_dragging_out_and_back_within_threshold() {
        let leaf = native("a", size(50.0, 50.0));
        layout_tree::<FakeHandle>(&leaf, size(50.0, 50.0));
        let tapped = count_calls::<crate::input::TappedEventArgs>(&leaf, "on_tapped");

        let dispatcher = crate::input::PointerDispatcher::new();
        let focus = crate::focus::FocusTracker::new();
        dispatcher.handle(
            &leaf,
            &focus,
            press_event(5.0, 5.0, crate::input::MouseButton::Left, 0.0),
        );
        // Wanders far outside `leaf`'s own bounds mid-drag — implicit capture must keep routing
        // `Moved`/`Released` to `leaf` regardless.
        dispatcher.handle(&leaf, &focus, move_event(500.0, 500.0));
        dispatcher.handle(
            &leaf,
            &focus,
            release_event(6.0, 6.0, crate::input::MouseButton::Left, 10.0),
        );

        assert_eq!(*tapped.borrow(), 1);
    }

    #[test]
    fn tap_does_not_fire_when_release_moves_past_the_threshold() {
        let leaf = native("a", size(50.0, 50.0));
        layout_tree::<FakeHandle>(&leaf, size(50.0, 50.0));
        let tapped = count_calls::<crate::input::TappedEventArgs>(&leaf, "on_tapped");
        let pressed = count_calls::<crate::input::PointerEventArgs>(&leaf, "on_pointer_pressed");
        let released = count_calls::<crate::input::PointerEventArgs>(&leaf, "on_pointer_released");

        let dispatcher = crate::input::PointerDispatcher::new();
        let focus = crate::focus::FocusTracker::new();
        dispatcher.handle(
            &leaf,
            &focus,
            press_event(5.0, 5.0, crate::input::MouseButton::Left, 0.0),
        );
        dispatcher.handle(
            &leaf,
            &focus,
            release_event(20.0, 20.0, crate::input::MouseButton::Left, 10.0),
        );

        assert_eq!(*pressed.borrow(), 1);
        assert_eq!(*released.borrow(), 1);
        assert_eq!(*tapped.borrow(), 0);
    }

    #[test]
    fn double_tap_fires_on_a_second_nearby_tap_within_the_time_window() {
        let leaf = native("a", size(50.0, 50.0));
        layout_tree::<FakeHandle>(&leaf, size(50.0, 50.0));
        let tapped = count_calls::<crate::input::TappedEventArgs>(&leaf, "on_tapped");
        let double_tapped = count_calls::<crate::input::TappedEventArgs>(&leaf, "on_double_tapped");

        let dispatcher = crate::input::PointerDispatcher::new();
        let focus = crate::focus::FocusTracker::new();
        dispatcher.handle(
            &leaf,
            &focus,
            press_event(5.0, 5.0, crate::input::MouseButton::Left, 0.0),
        );
        dispatcher.handle(
            &leaf,
            &focus,
            release_event(5.0, 5.0, crate::input::MouseButton::Left, 10.0),
        );
        // WinUI3 can report capture loss after a normal release relinquishes native capture. With
        // no active Core press this must be a no-op, preserving the completed first tap.
        dispatcher.handle(&leaf, &focus, cancel_event(5.0, 5.0));
        dispatcher.handle(
            &leaf,
            &focus,
            press_event(6.0, 6.0, crate::input::MouseButton::Left, 100.0),
        );
        dispatcher.handle(
            &leaf,
            &focus,
            release_event(6.0, 6.0, crate::input::MouseButton::Left, 110.0),
        );

        assert_eq!(*tapped.borrow(), 2);
        assert_eq!(*double_tapped.borrow(), 1);

        // A third tap right after pairs with nothing (the second tap's own record was consumed).
        dispatcher.handle(
            &leaf,
            &focus,
            press_event(6.0, 6.0, crate::input::MouseButton::Left, 150.0),
        );
        dispatcher.handle(
            &leaf,
            &focus,
            release_event(6.0, 6.0, crate::input::MouseButton::Left, 155.0),
        );
        assert_eq!(*tapped.borrow(), 3);
        assert_eq!(*double_tapped.borrow(), 1);
    }

    #[test]
    fn right_button_fires_right_tapped_not_tapped() {
        let leaf = native("a", size(50.0, 50.0));
        layout_tree::<FakeHandle>(&leaf, size(50.0, 50.0));
        let tapped = count_calls::<crate::input::TappedEventArgs>(&leaf, "on_tapped");
        let right_tapped = count_calls::<crate::input::TappedEventArgs>(&leaf, "on_right_tapped");

        let dispatcher = crate::input::PointerDispatcher::new();
        let focus = crate::focus::FocusTracker::new();
        dispatcher.handle(
            &leaf,
            &focus,
            press_event(5.0, 5.0, crate::input::MouseButton::Right, 0.0),
        );
        dispatcher.handle(
            &leaf,
            &focus,
            release_event(5.0, 5.0, crate::input::MouseButton::Right, 10.0),
        );

        assert_eq!(*tapped.borrow(), 0);
        assert_eq!(*right_tapped.borrow(), 1);
    }
}

#[cfg(test)]
mod layout_reflow_tests {
    use super::*;
    use crate::environment::{EnvironmentContext, ReduceMotionEnvironment};
    use crate::ui::testsupport::*;
    use std::time::Duration;

    fn linear(ms: u64) -> Animation {
        Animation::linear(Duration::from_millis(ms))
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1.0e-3,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn rf01_animated_removal_moves_the_survivor_from_its_old_rendered_position() {
        let mut fx = ReflowFixture::new(&["a", "b", "c"]);
        let (b, c) = (fx.item(1), fx.item(2));
        let before = visible_y(&c);
        assert_eq!(before, 40.0);

        with_animation(linear(100), || assert!(fx.children.remove(&b)));
        fx.relayout();

        assert!(b.visual_parent().is_none(), "B leaves layout immediately");
        assert_eq!(c.arranged_offset().map(|offset| offset.y), Some(20.0));
        assert_eq!(visible_y(&c), before, "first projected frame must not jump");
        assert_close(fx.group(&c).transform.dy, 20.0);
        assert_eq!(fx.host.frame_requests.get(), 1);

        assert!(fx.tick_ms(50));
        fx.relayout();
        let middle = visible_y(&c);
        assert!(middle > 20.0 && middle < 40.0, "middle frame {middle}");
        assert_close(fx.group(&c).transform.dy, middle - 20.0);

        assert!(!fx.tick_ms(100));
        fx.relayout();
        assert_eq!(reflow_of(&c), Vector::default());
        assert_eq!(visible_y(&c), 20.0);
        assert_eq!(fx.host.runtime.layout_reflow_count(), 0);
        assert_eq!(fx.group(&c).transform, AffineTransform::IDENTITY);
    }

    #[test]
    fn rf02_exiting_child_keeps_its_position_without_input_while_the_survivor_moves() {
        let mut fx = ReflowFixture::new(&["a", "b", "c"]);
        let (b, c) = (fx.item(1), fx.item(2));
        let unmounts = Rc::new(Cell::new(0));
        let observed = Rc::clone(&unmounts);
        b.add_unmount_hook(Box::new(move || observed.set(observed.get() + 1)));
        let c_id = c.render_group_id();

        with_animation(linear(100), || {
            assert!(start_exit_transition(
                &b,
                &Transition::opacity().combined(Transition::scale(0.5)),
                linear(100),
            ));
            assert!(fx.children.remove(&b));
        });
        fx.relayout();

        assert_eq!(b.visual_participation(), VisualParticipation::Exiting);
        assert_eq!(b.arranged_offset().map(|offset| offset.y), Some(20.0));
        assert!(!fx.group(&b).input_enabled);
        assert_eq!(reflow_of(&b), Vector::default(), "exiting never reflows");
        assert_eq!(visible_y(&c), 40.0);
        let hit = hit_test(&fx.root, Point { x: 5.0, y: 25.0 });
        assert!(hit.is_none_or(|hit| !Rc::ptr_eq(&hit, &b)));

        fx.tick_ms(50);
        fx.relayout();
        assert!(visible_y(&c) < 40.0);
        fx.tick_ms(100);
        fx.relayout();
        fx.tick_ms(150);
        assert_eq!(unmounts.get(), 1, "exit completes exactly once");
        assert_eq!(c.render_group_id(), c_id);
        assert_eq!(visible_y(&c), 20.0);
    }

    #[test]
    fn rf03_inserted_child_uses_only_its_transition_and_pushes_existing_siblings() {
        let mut fx = ReflowFixture::new(&["a", "b"]);
        let (a, b) = (fx.item(0), fx.item(1));
        let x = reflow_leaf("x");
        with_animation(linear(100), || {
            fx.children.insert(0, Rc::clone(&x));
            start_enter_transition(&x, &Transition::opacity(), linear(100));
        });
        fx.relayout();

        assert!(!fx.host.runtime.has_layout_reflow(x.render_group_id()));
        assert_eq!(reflow_of(&x), Vector::default());
        assert_eq!(reflow_of(&a).y, -20.0);
        assert_eq!(reflow_of(&b).y, -20.0);
        assert_eq!(visible_y(&a), 0.0);
        assert_eq!(visible_y(&b), 20.0);
        assert_eq!(fx.host.runtime.layout_reflow_count(), 2);
    }

    #[test]
    fn rf04_reorder_interpolates_every_surviving_identity_and_skips_replacements() {
        let mut fx = ReflowFixture::new(&["a", "b", "c"]);
        let (a, b, c) = (fx.item(0), fx.item(1), fx.item(2));
        with_animation(linear(100), || {
            assert!(fx.children.remove(&c));
            fx.children.insert(0, Rc::clone(&c));
        });
        fx.relayout();
        assert_eq!(reflow_of(&c).y, 40.0);
        assert_eq!(reflow_of(&a).y, -20.0);
        assert_eq!(reflow_of(&b).y, -20.0);
        for (item, old) in [(&c, 40.0), (&a, 0.0), (&b, 20.0)] {
            assert_eq!(visible_y(item), old);
        }
        fx.tick_ms(100);
        fx.relayout();

        let replacement = reflow_leaf("b2");
        with_animation(linear(100), || {
            fx.children.remove_at(2);
            fx.children.insert(0, Rc::clone(&replacement));
        });
        fx.relayout();
        assert_eq!(reflow_of(&replacement), Vector::default());
        assert!(
            !fx.host
                .runtime
                .has_layout_reflow(replacement.render_group_id())
        );
        assert_eq!(reflow_of(&c).y, -20.0);
        assert_eq!(reflow_of(&a).y, -20.0);
    }

    #[test]
    fn rf05_visibility_changes_reflow_only_the_surrounding_siblings() {
        let mut fx = ReflowFixture::new(&["a", "b", "c"]);
        let (b, c) = (fx.item(1), fx.item(2));
        with_animation(linear(100), || b.set_visibility(Visibility::Collapsed));
        fx.relayout();
        assert!(!b.participates_in_layout());
        assert!(!fx.tree.group_paths.contains_key(&b.render_group_id()));
        assert_eq!(reflow_of(&c).y, 20.0);
        assert_eq!(visible_y(&c), 40.0);
        fx.tick_ms(100);
        fx.relayout();
        assert_eq!(visible_y(&c), 20.0);

        with_animation(linear(100), || b.set_visibility(Visibility::Visible));
        fx.relayout();
        assert_eq!(
            reflow_of(&b),
            Vector::default(),
            "reappearing element snaps"
        );
        assert!(!fx.host.runtime.has_layout_reflow(b.render_group_id()));
        assert_eq!(visible_y(&b), 20.0);
        assert_eq!(reflow_of(&c).y, -20.0);
        assert_eq!(visible_y(&c), 20.0);
    }

    #[test]
    fn rf06_retarget_rebases_continuously_and_keeps_spring_velocity() {
        let spring = Animation::spring(Duration::from_millis(300), 0.5);
        let mut fx = ReflowFixture::new(&["a", "b", "c"]);
        let (a, b, c) = (fx.item(0), fx.item(1), fx.item(2));
        with_animation(spring, || assert!(fx.children.remove(&b)));
        fx.relayout();
        fx.tick_ms(60);
        fx.relayout();
        let (expected_current, velocity) = spring.sample(20.0, 0.0, 0.0, Duration::from_millis(60));
        let current = reflow_of(&c);
        assert_close(current.y, expected_current);
        assert!(velocity.abs() > 1.0, "spring must be moving at t1");
        let old_layout = c.arranged_offset().unwrap();

        with_animation(spring, || assert!(fx.children.remove(&a)));
        fx.relayout();
        let new_layout = c.arranged_offset().unwrap();
        let rebased = reflow_of(&c);
        assert_eq!(new_layout.y, 0.0);
        assert_close(old_layout.y + current.y, new_layout.y + rebased.y);
        assert_close(old_layout.x + current.x, new_layout.x + rebased.x);
        assert_close(fx.group(&c).transform.dy, rebased.y);

        fx.tick_ms(80);
        let (expected_next, _) = spring.sample(rebased.y, 0.0, velocity, Duration::from_millis(20));
        assert_close(reflow_of(&c).y, expected_next);
    }

    #[test]
    fn rf06_contract_example_rebases_to_sixty() {
        let runtime = AnimationRuntime::new();
        let start = runtime
            .rebase_layout_reflow(
                7,
                Vector { x: 0.0, y: 30.0 },
                Vector {
                    x: 0.0,
                    y: 50.0 - 20.0,
                },
                linear(100),
                None,
                Box::new(|_, _| {}),
            )
            .expect("finite");
        assert_eq!(start, Vector { x: 0.0, y: 60.0 });
        assert_eq!(50.0 + 30.0, 20.0 + start.y);
    }

    #[test]
    fn rf07_reflow_translates_in_parent_axes_before_user_and_transition_transforms() {
        let mut fx = ReflowFixture::new(&["a", "b", "c"]);
        let (b, c) = (fx.item(1), fx.item(2));
        let user =
            VisualTransform::new(Vector { x: 3.0, y: 4.0 }, 2.0, std::f32::consts::FRAC_PI_2);
        c.as_ui_element().set_visual_transform(user);
        let transition = VisualTransform::new(Vector { x: 0.0, y: 5.0 }, 0.5, 0.0);
        c.as_ui_element()
            .transition_visual_transform
            .set(transition);
        with_animation(linear(100), || assert!(fx.children.remove(&b)));
        fx.relayout();

        let local = local_transform(
            c.presentation_visual_transform(),
            c.transform_origin(),
            size(100.0, 20.0),
        );
        let group = fx.group(&c).transform;
        for point in [Point { x: 0.0, y: 0.0 }, Point { x: 10.0, y: 7.0 }] {
            let with = group.transform_point(point);
            let without = local.transform_point(point);
            assert_close(with.x - without.x, 0.0);
            assert_close(with.y - without.y, 20.0);
        }
        assert_eq!(c.as_ui_element().presentation_visual_transform.get(), user);
        assert_eq!(
            c.as_ui_element().transition_visual_transform.get(),
            transition
        );
    }

    #[test]
    fn rf08_hit_testing_and_clip_follow_the_presentation_position() {
        let mut fx = ReflowFixture::new(&["a", "b", "c"]);
        let (b, c) = (fx.item(1), fx.item(2));
        fx.root.as_ui_element().set_clip_to_bounds(Some(true));
        with_animation(linear(100), || assert!(fx.children.remove(&b)));
        fx.relayout();
        // C is laid out at y=20..40 but still shown at y=40..60.
        let hit = hit_test(&fx.root, Point { x: 5.0, y: 45.0 });
        assert!(hit.is_some_and(|hit| Rc::ptr_eq(&hit, &c)));
        let stale = hit_test(&fx.root, Point { x: 5.0, y: 25.0 });
        assert!(stale.is_none_or(|hit| !Rc::ptr_eq(&hit, &c)));

        fx.root.as_ui_element().set_height(50.0);
        fx.relayout();
        assert_eq!(visible_y(&c), 40.0);
        let clipped = hit_test(&fx.root, Point { x: 5.0, y: 55.0 });
        assert!(
            clipped.is_none(),
            "clip is tested in the parent's local space"
        );
        let inside = hit_test(&fx.root, Point { x: 5.0, y: 45.0 });
        assert!(inside.is_some_and(|hit| Rc::ptr_eq(&hit, &c)));
    }

    fn assert_snapped(fx: &ReflowFixture, c: &Rc<dyn UIElementExt>) {
        assert_eq!(reflow_of(c), Vector::default());
        assert_eq!(visible_y(c), 20.0);
        assert_eq!(fx.host.runtime.layout_reflow_count(), 0);
        assert_eq!(fx.host.frame_requests.get(), 0);
        assert!(!fx.host.runtime.take_frame_request());
    }

    #[test]
    fn rf09_disabled_immediate_untransacted_and_reduced_motion_mutations_snap() {
        let disabled = Transaction {
            animation: Some(linear(100)),
            disables_animations: true,
        };
        type Case = Box<dyn Fn(&ReflowFixture, &Rc<dyn UIElementExt>)>;
        let cases: Vec<Case> = vec![
            Box::new(move |fx, b| with_transaction(disabled, || assert!(fx.children.remove(b)))),
            Box::new(|fx, b| with_animation(linear(0), || assert!(fx.children.remove(b)))),
            Box::new(|fx, b| assert!(fx.children.remove(b))),
            Box::new(|fx, b| {
                let env = EnvironmentContext::root();
                env.set::<ReduceMotionEnvironment>(true);
                fx.root.set_environment_context(env);
                with_animation(linear(100), || assert!(fx.children.remove(b)));
            }),
        ];
        for case in cases {
            let mut fx = ReflowFixture::new(&["a", "b", "c"]);
            let (b, c) = (fx.item(1), fx.item(2));
            case(&fx, &b);
            fx.relayout();
            assert_snapped(&fx, &c);
        }
    }

    #[test]
    fn rf10_reduce_motion_mid_flight_cancels_reflow_but_not_unrelated_channels() {
        let mut fx = ReflowFixture::new(&["a", "b", "c"]);
        let (a, b, c) = (fx.item(0), fx.item(1), fx.item(2));
        let env = EnvironmentContext::root();
        fx.root.set_environment_context(env.clone());
        with_animation(linear(100), || {
            assert!(fx.children.remove(&b));
            a.as_ui_element().set_opacity(0.0);
        });
        fx.relayout();
        fx.tick_ms(50);
        assert_close(reflow_of(&c).y, 10.0);

        env.set::<ReduceMotionEnvironment>(true);
        assert!(
            fx.tick_ms(60),
            "the unrelated opacity channel keeps running"
        );
        assert_eq!(reflow_of(&c), Vector::default());
        assert_eq!(fx.host.runtime.layout_reflow_count(), 0);
        fx.tick_ms(100);
        assert_eq!(
            reflow_of(&c),
            Vector::default(),
            "no stale completion write-back"
        );
        assert!(fx.host.runtime.is_idle());
    }

    #[test]
    fn rf10_unanimated_structural_change_cancels_the_running_reflow() {
        let mut fx = ReflowFixture::new(&["a", "b", "c"]);
        let (a, b, c) = (fx.item(0), fx.item(1), fx.item(2));
        with_animation(linear(100), || assert!(fx.children.remove(&b)));
        fx.relayout();
        fx.tick_ms(50);
        assert!(reflow_of(&c).y > 0.0);

        assert!(fx.children.remove(&a));
        fx.relayout();
        assert_eq!(reflow_of(&c), Vector::default());
        assert_eq!(visible_y(&c), 0.0);
        assert!(!fx.tick_ms(100));
        assert_eq!(reflow_of(&c), Vector::default());
    }

    #[test]
    fn rf12_last_intent_wins_and_the_baseline_is_the_last_rendered_group() {
        let mut fx = ReflowFixture::new(&["a", "b", "c", "d"]);
        let (a, b, c, d) = (fx.item(0), fx.item(1), fx.item(2), fx.item(3));
        // Several mutations and an extra layout before the next reconcile animate once, from the
        // retained render positions to the final target.
        with_animation(linear(100), || {
            assert!(fx.children.remove(&a));
            fx.children.insert(1, reflow_leaf("x"));
        });
        layout_root(&fx.root, size(100.0, 200.0));
        with_animation(linear(100), || assert!(fx.children.remove(&b)));
        fx.relayout();
        assert_eq!(c.arranged_offset().unwrap().y, 20.0);
        assert_eq!(visible_y(&c), 40.0);
        assert_eq!(visible_y(&d), 60.0);
        assert_eq!(fx.host.frame_requests.get(), 1);

        // A nested disabling transaction recorded last turns the whole pending change into a snap.
        let mut fx = ReflowFixture::new(&["a", "b", "c"]);
        let (a, b, c) = (fx.item(0), fx.item(1), fx.item(2));
        with_animation(linear(100), || {
            assert!(fx.children.remove(&a));
            with_transaction(
                Transaction {
                    animation: None,
                    disables_animations: true,
                },
                || assert!(fx.children.remove(&b)),
            );
        });
        fx.relayout();
        assert_eq!(reflow_of(&c), Vector::default());
        assert_eq!(fx.host.runtime.layout_reflow_count(), 0);
    }

    #[test]
    fn rf12_intent_waits_for_a_completed_layout() {
        let mut fx = ReflowFixture::new(&["a", "b", "c"]);
        let (b, c) = (fx.item(1), fx.item(2));
        with_animation(linear(100), || assert!(fx.children.remove(&b)));
        // A render-only pass (no layout_root) must not consume the intent.
        assert!(fx.tree.reconcile::<FakeHandle>(&fx.root));
        assert!(fx.host.runtime.pending_layout_reflow_intent().is_some());
        fx.relayout();
        assert!(fx.host.runtime.pending_layout_reflow_intent().is_none());
        assert_eq!(visible_y(&c), 40.0);
        assert_eq!(reflow_of(&c).y, 20.0);
    }

    struct BorrowCheckingRelayoutHost {
        tree: Rc<RefCell<Option<RenderTree>>>,
        requests: Cell<usize>,
        conflicts: Cell<usize>,
    }

    impl RelayoutHost for BorrowCheckingRelayoutHost {
        fn request_relayout(&self, _dirty_group_id: u64, _kind: InvalidationKind) {
            self.requests.set(self.requests.get() + 1);
            if self.tree.try_borrow_mut().is_err() {
                self.conflicts.set(self.conflicts.get() + 1);
            }
        }
    }

    #[test]
    fn rf13_reconcile_never_reenters_invalidation_and_tick_callbacks_may_mutate() {
        let fx = ReflowFixture::new(&["a", "b", "c", "d"]);
        let (a, b, c, d) = (fx.item(0), fx.item(1), fx.item(2), fx.item(3));
        let ReflowFixture {
            root,
            children,
            host,
            tree,
            ..
        } = fx;
        let tree = Rc::new(RefCell::new(Some(tree)));
        let relayout = Rc::new(BorrowCheckingRelayoutHost {
            tree: Rc::clone(&tree),
            requests: Cell::new(0),
            conflicts: Cell::new(0),
        });
        root.set_invalidate_host(Some(relayout.clone() as Rc<dyn RelayoutHost>));
        let pass = |root: &Rc<dyn UIElementExt>| {
            layout_root(root, size(100.0, 200.0));
            let mut guard = tree.borrow_mut();
            assert!(guard.as_mut().unwrap().reconcile::<FakeHandle>(root));
        };

        with_animation(linear(100), || assert!(children.remove(&b)));
        pass(&root);
        assert_eq!(relayout.conflicts.get(), 0);
        assert_eq!(reflow_of(&c).y, 20.0);

        // A tick callback cancels C's reflow and mutates the tree under an animation.
        let mutated = Rc::new(Cell::new(false));
        let flag = Rc::clone(&mutated);
        let runtime = Rc::clone(&host.runtime);
        let children_in_callback = children.clone();
        let a_in_callback = Rc::clone(&a);
        let c_id = c.render_group_id();
        host.runtime.animate(
            a.render_group_id(),
            AnimationChannel::Opacity,
            AnimatedValue::Scalar(1.0),
            AnimatedValue::Scalar(0.5),
            linear(100),
            Box::new(move |_| {
                if !flag.replace(true) {
                    runtime.cancel_layout_reflow(c_id);
                    with_animation(linear(100), || {
                        assert!(children_in_callback.remove(&a_in_callback));
                    });
                }
            }),
        );
        host.runtime.tick(Duration::from_millis(50));
        assert!(mutated.get());
        assert_eq!(
            host.runtime.layout_reflow_count(),
            1,
            "only D's reflow survives the in-callback cancel"
        );
        assert_eq!(
            reflow_of(&c),
            Vector::default(),
            "a cancelled reflow resets and its pending callback never writes back"
        );
        pass(&root);
        assert_eq!(relayout.conflicts.get(), 0);
        assert_eq!(
            reflow_of(&c).y,
            20.0,
            "fresh reflow after the callback mutation"
        );
        assert_eq!(host.runtime.layout_reflow_count(), 2);
        host.runtime.tick(Duration::from_millis(200));
        assert_eq!(host.runtime.layout_reflow_count(), 0);
        assert_eq!(reflow_of(&c), Vector::default());
        assert_eq!(reflow_of(&d), Vector::default());
        assert!(relayout.requests.get() > 0);
    }

    #[test]
    fn rf14_mount_unhosted_teardown_and_remount_never_leak_or_animate() {
        // Unhosted: no runtime, no intent, nothing animates.
        let layout = VerticalLayout::new();
        let children = layout.children().clone();
        let (a, b) = (reflow_leaf("a"), reflow_leaf("b"));
        children.add(Rc::clone(&a));
        children.add(Rc::clone(&b));
        let root: Rc<dyn UIElementExt> = layout;
        let mut tree = layout_tree::<FakeHandle>(&root, size(100.0, 200.0));
        with_animation(linear(100), || assert!(children.remove(&a)));
        layout_root(&root, size(100.0, 200.0));
        tree.reconcile::<FakeHandle>(&root);
        assert_eq!(reflow_of(&b), Vector::default());

        // Initial mount after an animated mutation never reflows.
        let host = install_animation_host(&root);
        with_animation(linear(100), || children.insert(0, Rc::clone(&a)));
        let tree = layout_tree::<FakeHandle>(&root, size(100.0, 200.0));
        assert!(host.runtime.pending_layout_reflow_intent().is_none());
        assert_eq!(reflow_of(&b), Vector::default());
        drop(tree);

        // Host teardown during a reflow resets the translation and drops the channel.
        let mut fx = ReflowFixture::new(&["a", "b", "c"]);
        let (b, c) = (fx.item(1), fx.item(2));
        with_animation(linear(100), || assert!(fx.children.remove(&b)));
        fx.relayout();
        assert!(reflow_of(&c).y > 0.0);
        let runtime = Rc::clone(&fx.host.runtime);
        fx.root.set_animation_frame_host(None);
        assert_eq!(reflow_of(&c), Vector::default());
        assert_eq!(runtime.layout_reflow_count(), 0);
        assert!(runtime.pending_layout_reflow_intent().is_none());
        assert!(!runtime.tick(Duration::from_millis(50)));

        // Re-mount on a fresh host: the first build never reflows.
        fx.host = install_animation_host(&fx.root);
        fx.tree = RenderTree::new::<FakeHandle>(&fx.root);
        fx.relayout();
        assert_eq!(reflow_of(&c), Vector::default());

        // A dropped element does not stay alive through its reflow channel.
        let mut fx = ReflowFixture::new(&["a", "b", "c"]);
        let (b, c) = (fx.item(1), fx.item(2));
        with_animation(linear(100), || assert!(fx.children.remove(&b)));
        fx.relayout();
        let weak = Rc::downgrade(&c);
        assert!(fx.children.remove(&c));
        fx.relayout();
        fx.items.clear();
        drop(c);
        assert!(
            weak.upgrade().is_none(),
            "reflow state holds only a weak element"
        );
        fx.tick_ms(50);
        assert_eq!(fx.host.runtime.layout_reflow_count(), 0);
    }

    struct CountingAccessibilityHost(Cell<usize>);

    impl crate::accessibility::AccessibilityHost for CountingAccessibilityHost {
        fn request_accessibility_update(&self) {
            self.0.set(self.0.get() + 1);
        }
    }

    #[test]
    fn rf08_each_reflow_tick_refreshes_semantic_bounds_once() {
        let mut fx = ReflowFixture::new(&["a", "b", "c", "d"]);
        let (a, c, d) = (fx.item(0), fx.item(2), fx.item(3));
        let accessibility = Rc::new(CountingAccessibilityHost(Cell::new(0)));
        fx.root.set_accessibility_host(Some(
            accessibility.clone() as Rc<dyn crate::accessibility::AccessibilityHost>
        ));
        with_animation(linear(100), || assert!(fx.children.remove(&a)));
        fx.relayout();
        assert_eq!(fx.host.runtime.layout_reflow_count(), 3);
        let before = accessibility.0.get();
        fx.tick_ms(50);
        assert_eq!(
            accessibility.0.get(),
            before + 1,
            "one refresh for three reflows"
        );
        assert!(reflow_of(&c).y > 0.0 && reflow_of(&d).y > 0.0);
        fx.tick_ms(100);
        assert_eq!(
            accessibility.0.get(),
            before + 2,
            "the final position is refreshed too"
        );
        fx.tick_ms(150);
        assert_eq!(
            accessibility.0.get(),
            before + 2,
            "no refresh without reflow"
        );
    }

    #[test]
    fn rf15_property_animation_moves_siblings_without_a_reflow_channel() {
        let mut fx = ReflowFixture::new(&["a", "b", "c"]);
        let (b, c) = (fx.item(1), fx.item(2));
        b.as_ui_element().set_height(20.0);
        fx.relayout();
        with_animation(linear(100), || b.as_ui_element().set_height(60.0));
        assert!(fx.host.runtime.pending_layout_reflow_intent().is_none());
        fx.relayout();
        fx.tick_ms(50);
        fx.relayout();
        let middle = c.arranged_offset().unwrap().y;
        assert!(middle > 40.0 && middle < 80.0, "layout moves C: {middle}");
        assert_eq!(reflow_of(&c), Vector::default());
        assert_eq!(fx.host.runtime.layout_reflow_count(), 0);
    }
}
