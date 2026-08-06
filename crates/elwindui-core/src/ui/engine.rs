//! Layout, render-group building/reconciliation, hit-testing, and routed-event dispatch — the
//! tree-walking engine that operates *over* the element classes rather than being part of any one
//! of them.
//!
//! Split out of this module's former single-file form unchanged; `use super::*` keeps the shared
//! import block in `mod.rs` so that split stayed a pure code move.

use super::*;

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
        width: elem.width().unwrap_or(size.width),
        height: elem.height().unwrap_or(size.height),
    };
    apply_size_constraints(
        overridden,
        elem.min_width(),
        elem.max_width(),
        elem.min_height(),
        elem.max_height(),
    )
}

/// This element's natural (unconstrained) size — e.g. for a container that must report an
/// `intrinsicContentSize` to an Auto-Layout-managed ancestor (see `elwindui-backend-appkit`'s
/// `TreeHostView`) before it has ever actually been given a frame to lay out into.
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

/// Builds one retained RenderGroup for every arranged, visible Visual.
pub(crate) fn build_render_group<H: Clone + 'static>(
    elem: &Rc<dyn UIElementExt>,
    offset: Point,
) -> Option<RenderGroup> {
    if elem.visibility() == Visibility::Collapsed {
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
    record_group_commands::<H>(elem, &mut group);
    group.generation += 1;
    for child in elem.visual_children() {
        let child_offset = child.arranged_offset().unwrap_or(Point { x: 0.0, y: 0.0 });
        if let Some(child_group) = build_render_group::<H>(&child, child_offset) {
            group.children.push(child_group);
        }
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
}

pub(crate) fn index_render_groups(
    elem: &Rc<dyn UIElementExt>,
    group: &RenderGroup,
    path: Vec<usize>,
    group_paths: &mut HashMap<u64, Vec<usize>>,
    visual_index: &mut HashMap<u64, Weak<dyn UIElementExt>>,
) {
    group_paths.insert(group.id, path.clone());
    visual_index.insert(group.id, Rc::downgrade(elem));
    let mut group_children = group.children.iter().enumerate();
    for child in elem.visual_children() {
        if child.visibility() == Visibility::Collapsed {
            continue;
        }
        let Some((child_index, child_group)) = group_children.next() else {
            break;
        };
        let mut child_path = path.clone();
        child_path.push(child_index);
        index_render_groups(&child, child_group, child_path, group_paths, visual_index);
    }
}

pub(crate) fn reconcile_render_group<H: Clone + 'static>(
    elem: &Rc<dyn UIElementExt>,
    group: &mut RenderGroup,
    offset: Point,
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
    if group.offset != offset || group.size != size || group.clip != clip {
        group.offset = offset;
        group.size = size;
        group.clip = clip;
        group.is_dirty = true;
    }

    let old_children = std::mem::take(&mut group.children);
    let mut old_by_id: HashMap<u64, RenderGroup> = old_children
        .into_iter()
        .map(|child| (child.id, child))
        .collect();
    let mut children = Vec::new();
    for child in elem.visual_children() {
        if child.visibility() == Visibility::Collapsed {
            continue;
        }
        let child_offset = child.arranged_offset().unwrap_or(Point { x: 0.0, y: 0.0 });
        let id = child.render_group_id();
        let child_group = if let Some(mut existing) = old_by_id.remove(&id) {
            reconcile_render_group::<H>(&child, &mut existing, child_offset);
            existing
        } else {
            group.is_dirty = true;
            build_render_group::<H>(&child, child_offset)
                .expect("visible Visual must have a RenderGroup")
        };
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
        let offset = root.arranged_offset().unwrap_or(Point { x: 0.0, y: 0.0 });
        let root_group = build_render_group::<H>(root, offset)
            .unwrap_or_else(|| RenderGroup::new(root.render_group_id(), offset, None));
        let mut tree = Self::with_root(root_group);
        index_render_groups(
            root,
            &tree.root,
            Vec::new(),
            &mut tree.group_paths,
            &mut tree.visual_index,
        );
        tree
    }

    /// Reconciles an already retained tree after `layout_root`. Group identities and clean command
    /// buffers survive; only changed or explicitly invalidated groups record commands again.
    pub fn reconcile<H: Clone + 'static>(&mut self, root: &Rc<dyn UIElementExt>) -> bool {
        if self.root.id != root.render_group_id() {
            return false;
        }
        let offset = root.arranged_offset().unwrap_or(Point { x: 0.0, y: 0.0 });
        reconcile_render_group::<H>(root, &mut self.root, offset);
        self.group_paths.clear();
        self.visual_index.clear();
        index_render_groups(
            root,
            &self.root,
            Vec::new(),
            &mut self.group_paths,
            &mut self.visual_index,
        );
        true
    }

    pub fn root_id(&self) -> u64 {
        self.root.id
    }
}

pub(crate) fn rect_contains(rect: Rect, at: Point) -> bool {
    at.x >= rect.x && at.x <= rect.x + rect.width && at.y >= rect.y && at.y <= rect.y + rect.height
}

/// Intersection of two absolute-coordinate rects — `Rect`'s `width`/`height` go negative (never
/// clamped to 0) when they don't overlap at all, which `rect_contains` already correctly treats as
/// "contains nothing" (`at.x <= rect.x + rect.width` can't hold for any real `at.x` once `width` is
/// negative). Used by `hit_test_at` to fold each `clip_to_bounds`-opted-in ancestor's own rect into
/// the effective clip a point must fall within to reach its descendants at all.
pub(crate) fn intersect_rect(a: Rect, b: Rect) -> Rect {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let right = (a.x + a.width).min(b.x + b.width);
    let bottom = (a.y + a.height).min(b.y + b.height);
    Rect {
        x,
        y,
        width: right - x,
        height: bottom - y,
    }
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
pub(crate) fn hit_test_at(
    elem: &Rc<dyn UIElementExt>,
    absolute_origin: Point,
    at: Point,
    inherited_clip: Option<Rect>,
) -> Option<Rc<dyn UIElementExt>> {
    // A `Collapsed` element (and its whole subtree) is excluded from hit-testing, matching
    // `collect_render_items`'s own treatment — see `Visibility`'s own doc comment. `hit_test_visible
    // == false` (WinUI3's `IsHitTestVisible`) excludes the subtree the same way, with no layout/
    // render effect at all — see that field's own doc comment.
    if elem.visibility() == Visibility::Collapsed || !elem.hit_test_visible() {
        return None;
    }
    let width = elem.arranged_width().unwrap_or(0.0);
    let height = elem.arranged_height().unwrap_or(0.0);
    let own_rect = Rect {
        x: absolute_origin.x,
        y: absolute_origin.y,
        width,
        height,
    };
    let own_clip = elem.clip_to_bounds().then_some(own_rect);
    let effective_clip = match (inherited_clip, own_clip) {
        (Some(a), Some(b)) => Some(intersect_rect(a, b)),
        (Some(clip), None) | (None, Some(clip)) => Some(clip),
        (None, None) => None,
    };
    if let Some(clip) = effective_clip {
        if !rect_contains(clip, at) {
            return None;
        }
    }

    // Children are searched last-to-first: traversal order paints later children on top of
    // earlier ones (see 付録N's z-order note), so the *last* child whose own rect contains `at`
    // is the topmost, correctly-hit one. Checked regardless of whether `at` falls within `elem`'s
    // *own* rect — a child may render outside a non-clipping parent's bounds (see this function's
    // own doc comment).
    for child in elem.visual_children().iter().rev() {
        let offset = child.arranged_offset().unwrap_or(Point { x: 0.0, y: 0.0 });
        let child_origin = Point {
            x: absolute_origin.x + offset.x,
            y: absolute_origin.y + offset.y,
        };
        if let Some(hit) = hit_test_at(child, child_origin, at, effective_clip) {
            return Some(hit);
        }
    }

    if rect_contains(own_rect, at) && elem.hit_test_content() {
        Some(Rc::clone(elem))
    } else {
        None
    }
}

/// Hit-tests `root` at `at` (absolute coordinates, e.g. the hosting `TreeHostView`'s own local
/// point). Returns the deepest (topmost) hit element, or `None` if `at` falls outside `root`'s own
/// bounds entirely. Requires `root` to have already been laid out (e.g. via `layout_root`) — reads
/// cached `arranged_width`/`arranged_height`/`arranged_offset`, doesn't recompute them.
pub fn hit_test(root: &Rc<dyn UIElementExt>, at: Point) -> Option<Rc<dyn UIElementExt>> {
    // See `layout_root`'s own matching comment — `root`'s own `arranged_offset` (from its margin/
    // alignment against the original allotted rect) must be folded in here too, so hit-testing
    // agrees with `collect_render_items`'s rendered coordinates.
    let root_offset = root.arranged_offset().unwrap_or(Point { x: 0.0, y: 0.0 });
    hit_test_at(root, root_offset, at, None)
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
            let handler = handler
                .downcast_ref::<Box<dyn Fn(&T, &RoutedEventArgs)>>()
                .expect("elwindui: routed handler registered under a mismatched payload type");
            handler(payload, args);
            if args.handled.get() {
                return;
            }
        }
    }
}

/// Bubbles a routed event starting at `target` (e.g. `hit_test`'s return value, or a native leaf's
/// own tree node — see `elwindui-backend-appkit`'s `TreeHostView`): calls `target`'s own handlers
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
        invoke_handlers_at(&elem, name, payload, args);
        if args.handled.get() {
            return;
        }
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
