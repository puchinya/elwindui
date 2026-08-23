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
    if !elem.participates_in_layout() {
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
    group_paths.insert(group.id, path.clone());
    visual_index.insert(group.id, Rc::downgrade(elem));

    let old_children = std::mem::take(&mut group.children);
    let mut old_by_id: HashMap<u64, RenderGroup> = old_children
        .into_iter()
        .map(|child| (child.id, child))
        .collect();
    let mut children = Vec::new();
    for child in elem.visual_children() {
        if !child.participates_in_layout() {
            continue;
        }
        let child_offset = child.arranged_offset().unwrap_or(Point { x: 0.0, y: 0.0 });
        let id = child.render_group_id();
        // See `build_render_group`'s own comment on `path.push`/`path.pop` — `children.len()`
        // here is this child's dense index in `group.children`-to-be, the same role
        // `group.children.len()` plays there.
        path.push(children.len());
        let child_group = if let Some(mut existing) = old_by_id.remove(&id) {
            reconcile_render_group::<H>(
                &child,
                &mut existing,
                child_offset,
                path,
                group_paths,
                visual_index,
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
        if root.participates_in_layout() {
            reconcile_render_group::<H>(
                root,
                &mut self.root,
                offset,
                &mut Vec::new(),
                &mut self.group_paths,
                &mut self.visual_index,
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
    // A non-participating element (and its whole subtree) is excluded from hit-testing, matching
    // `build_render_group`'s own treatment — see `UIElementExt::participates_in_layout`'s own doc
    // comment. `hit_test_visible == false` (WinUI3's `IsHitTestVisible`) excludes the subtree the
    // same way, with no layout/render effect at all — see that field's own doc comment.
    if !elem.participates_in_layout() || !elem.hit_test_visible() {
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

#[cfg(test)]
mod tests {
    use super::*;
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
