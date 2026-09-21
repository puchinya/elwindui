//! Reconciling the native (non-drawn) children a render pass asks for against the XAML
//! children actually parented under the host panel.
//!
//! Lives under `host` rather than `render` because it operates on `TreeHost`'s own child
//! bookkeeping — it is this panel's rendering pass, not stateless translation.

use super::*;
use crate::ffi::{AnyView, UiCallbackRegistryOwner};
use crate::render::xaml_text_alignment;

use crate::bindings::Microsoft::UI::Xaml::Automation::AutomationProperties;
use crate::bindings::Microsoft::UI::Xaml::Automation::Peers::AccessibilityView;
use crate::bindings::Microsoft::UI::Xaml::Controls::{Canvas, Control, TextBlock};
use crate::bindings::Microsoft::UI::Xaml::Media::{CompositeTransform, Transform};
use crate::bindings::Microsoft::UI::Xaml::{FrameworkElement, RoutedEventHandler, UIElement};
use crate::render::composition::IslandId;
use elwindui_core::input::{FocusState, KeyboardDispatcher};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use windows::core::{HSTRING, Interface};

/// A `RenderCommand::Text`/`NativeControl` command's reflection as a real XAML child, kept across
/// relayout passes so it can be updated in place instead of torn down and recreated — see
/// `reconcile_native_children`'s own doc comment for why.
pub(crate) enum NativeChildElement {
    Text(TextBlock),
    Native(NativeChildState),
}

/// One reconciliation batch's Loaded readiness barrier. The root is deliberately weak: native
/// projection bookkeeping must never keep a Core tree alive after a host is replaced or cleared.
struct NativeLoadBatch {
    remaining: Cell<usize>,
    member_count: usize,
    saw_loaded: Cell<bool>,
    completed: Cell<bool>,
    host_id: Option<u64>,
    root: Option<Weak<dyn elwindui_core::ui::UIElementExt>>,
    #[cfg(test)]
    ready_completion_count: Cell<u32>,
}

impl NativeLoadBatch {
    fn new(
        remaining: usize,
        root: Option<Weak<dyn elwindui_core::ui::UIElementExt>>,
        host_id: Option<u64>,
    ) -> Self {
        Self {
            remaining: Cell::new(remaining),
            member_count: remaining,
            saw_loaded: Cell::new(false),
            completed: Cell::new(false),
            host_id,
            root,
            #[cfg(test)]
            ready_completion_count: Cell::new(0),
        }
    }
}

/// A NativeChildState-owned participation handle. The inner option is shared with the Loaded
/// callback so either the callback or teardown can resolve it exactly once, and resolution drops
/// the ticket's strong batch reference immediately.
#[derive(Clone)]
struct NativeLoadTicket(Rc<RefCell<Option<Rc<NativeLoadBatch>>>>);

impl NativeLoadTicket {
    fn new(batch: &Rc<NativeLoadBatch>) -> Self {
        Self(Rc::new(RefCell::new(Some(Rc::clone(batch)))))
    }

    fn resolve_loaded(&self) {
        self.resolve(true);
    }

    fn resolve_cancelled(&self) {
        self.resolve(false);
    }

    fn resolve(&self, loaded: bool) {
        let batch = self.0.borrow_mut().take();
        let Some(batch) = batch else {
            return;
        };
        let remaining_before = batch.remaining.get();
        if std::env::var_os("ELWINDUI_WINUI3_DIAGNOSTICS").is_some() {
            eprintln!(
                "[elwindui-winui3] native_load_member_resolve host={} loaded={} remaining_before={}",
                batch
                    .host_id
                    .map(|host_id| host_id.to_string())
                    .unwrap_or_else(|| "none".to_string()),
                loaded,
                remaining_before
            );
        }
        if loaded {
            batch.saw_loaded.set(true);
        }
        let remaining = batch.remaining.get().saturating_sub(1);
        batch.remaining.set(remaining);
        if remaining != 0 || batch.completed.replace(true) {
            return;
        }
        if std::env::var_os("ELWINDUI_WINUI3_DIAGNOSTICS").is_some() {
            eprintln!(
                "[elwindui-winui3] native_load_batch_complete host={} members={} saw_loaded={}",
                batch
                    .host_id
                    .map(|host_id| host_id.to_string())
                    .unwrap_or_else(|| "none".to_string()),
                batch.member_count,
                batch.saw_loaded.get()
            );
        }
        if !batch.saw_loaded.get() {
            return;
        }
        #[cfg(test)]
        batch
            .ready_completion_count
            .set(batch.ready_completion_count.get() + 1);
        let Some(root) = batch.root.as_ref().and_then(Weak::upgrade) else {
            return;
        };
        // This is intentionally the Core RelayoutHost path, not a direct TreeHost call. It
        // preserves per-host pending/ticket coalescing and turns a reentrant completion into the
        // existing RelayoutCycleState rerun.
        root.invalidate_measure();
        root.flush_interactive_relayout();
    }
}

/// One host-created focus subscription attached to a retained native control.
///
/// The control itself belongs to the core UI tree and survives host suppression, but these
/// host-specific routed handlers must follow the reflected native-child entry: dropping the entry
/// removes the WinRT event tokens and then releases its callback-registry ids. Reactivation can
/// therefore wire one fresh pair without accumulating duplicate focus dispatches.
pub(crate) struct NativeChildState {
    view: AnyView,
    loaded_token: Option<i64>,
    loaded_ticket: Option<NativeLoadTicket>,
    got_focus_token: Option<i64>,
    lost_focus_token: Option<i64>,
    callback_owner: UiCallbackRegistryOwner,
}

impl Drop for NativeChildState {
    fn drop(&mut self) {
        let element = self.view.as_element();
        if let Some(token) = self.loaded_token.take() {
            let _ = element.RemoveLoaded(token);
        }
        if let Some(ticket) = self.loaded_ticket.take() {
            ticket.resolve_cancelled();
        }
        if let Some(token) = self.got_focus_token.take() {
            let _ = element.RemoveGotFocus(token);
        }
        if let Some(token) = self.lost_focus_token.take() {
            let _ = element.RemoveLostFocus(token);
        }
        // `callback_owner` drops immediately after this method returns, once the native delegates
        // that could still contain their numeric ids have been detached above.
        let _ = &self.callback_owner;
    }
}

impl NativeChildElement {
    pub(crate) fn framework_element(&self) -> FrameworkElement {
        match self {
            NativeChildElement::Text(t) => {
                t.clone().cast().expect("TextBlock is a FrameworkElement")
            }
            NativeChildElement::Native(state) => state.view.as_element(),
        }
    }
}

/// Returns the projected native control for a Core owner, if this host currently has one.
/// Semantic accessibility peers intentionally do not expose these XAML children, but the host's
/// Core focus request still needs to synchronize a native control's real keyboard focus with the
/// same owner. The map borrow ends before the caller invokes XAML `Focus`, since that notification
/// can synchronously re-enter Core and request another relayout.
pub(crate) fn native_focus_element(
    native_children: &Rc<RefCell<NativeChildMap>>,
    owner_id: u64,
) -> Option<FrameworkElement> {
    native_children
        .borrow()
        .iter()
        .find_map(|((id, _), child)| {
            if *id != owner_id {
                return None;
            }
            match child {
                NativeChildElement::Native(state) => Some(state.view.as_element()),
                NativeChildElement::Text(_) => None,
            }
        })
}

/// Keyed by `(originating RenderGroup id, index of the command within that group's own
/// `commands`)` — stable across relayout passes for the common case of a UIElement's `render()`
/// always emitting the same shape of commands, so a `Text`/`NativeControl` producer that's merely
/// being updated in place (content, position, size) is told apart from one that's genuinely new or
/// gone. Reused directly as `HashMap` keys by both `TreeHost` (owns it) and `WinUI3RelayoutHost`
/// (holds a `Weak` reference, same pattern as `tree`/`render_tree`).
pub(crate) type NativeChildKey = (u64, usize);

pub(crate) type NativeChildMap = HashMap<NativeChildKey, NativeChildElement>;

type DiagnosticNativeRect = (NativeChildKey, f32, f32, f32, f32);

thread_local! {
    static LAST_DIAGNOSTIC_NATIVE_RECTS: RefCell<HashMap<u64, Vec<DiagnosticNativeRect>>> =
        RefCell::new(HashMap::new());
}

fn emit_diagnostic_native_rects(
    host_id: Option<u64>,
    wanted: &[(NativeChildKey, RenderedNativeChild)],
) {
    let Some(host_id) = host_id else {
        return;
    };
    if std::env::var_os("ELWINDUI_WINUI3_DIAGNOSTICS").is_none() {
        return;
    }
    let mut rects = wanted
        .iter()
        .filter_map(|(key, child)| match child {
            RenderedNativeChild::Native { rect, .. } => {
                Some((*key, rect.x, rect.y, rect.width, rect.height))
            }
            RenderedNativeChild::Text { .. } => None,
        })
        .collect::<Vec<_>>();
    rects.sort_by_key(|(key, ..)| *key);
    let changed = LAST_DIAGNOSTIC_NATIVE_RECTS.with(|last| {
        let mut last = last.borrow_mut();
        if last.get(&host_id) == Some(&rects) {
            false
        } else {
            last.insert(host_id, rects.clone());
            true
        }
    });
    if !changed {
        return;
    }
    eprintln!(
        "[elwindui-winui3] native_projection_snapshot host={} count={}",
        host_id,
        rects.len()
    );
    for (key, x, y, width, height) in rects {
        eprintln!(
            "[elwindui-winui3] native_projection_rect host={} group={} index={} x={} y={} width={} height={}",
            host_id, key.0, key.1, x, y, width, height
        );
    }
}

/// Projects the same presentation state used by the Composition island onto a XAML child.
/// `AffineTransform` is currently produced by Core from uniform scale/rotation/translation, so
/// the decomposition below is lossless for the public visual-transform surface. The matrix is
/// intentionally projected after arrange: layout remains target-state geometry and the native
/// island receives only the presentation transform and opacity.
fn apply_visual_projection(
    element: &FrameworkElement,
    transform: elwindui_core::base::AffineTransform,
    opacity: f32,
    input_enabled: bool,
) {
    if let Ok(ui) = element.clone().cast::<UIElement>() {
        // Native controls and paint TextBlocks are implementation details of the Core semantic
        // tree. Keep them available to the raw XAML view for input/rendering diagnostics, but out
        // of the public control view so the custom host peer cannot expose duplicate controls.
        let _ = AutomationProperties::SetAccessibilityView(&ui, AccessibilityView::Raw);
        let _ = ui.SetIsHitTestVisible(input_enabled);
    }
    if let Ok(control) = element.clone().cast::<Control>() {
        let _ = control.SetIsTabStop(input_enabled);
    }
    let _ = element.SetOpacity(opacity.clamp(0.0, 1.0) as f64);
    if transform == elwindui_core::base::AffineTransform::IDENTITY {
        let _ = element.SetRenderTransform(None);
        return;
    }

    let scale_x = (transform.m11 * transform.m11 + transform.m12 * transform.m12).sqrt();
    let scale_y = (transform.m21 * transform.m21 + transform.m22 * transform.m22).sqrt();
    let rotation = transform.m12.atan2(transform.m11).to_degrees();
    let Ok(composite) = CompositeTransform::new() else {
        return;
    };
    let _ = composite.SetScaleX(scale_x as f64);
    let _ = composite.SetScaleY(scale_y as f64);
    let _ = composite.SetRotation(rotation as f64);
    let _ = composite.SetTranslateX(transform.dx as f64);
    let _ = composite.SetTranslateY(transform.dy as f64);
    let Ok(composite): windows::core::Result<Transform> = composite.cast() else {
        return;
    };
    let _ = element.SetRenderTransform(Some(&composite));
}

#[derive(Clone, Copy)]
pub(crate) enum RenderLayerKey {
    Composition(IslandId),
    Native(NativeChildKey),
}

/// Reconciles `canvas`'s native `Text`/`NativeControl` children against `wanted` (this pass's
/// fresh set, in paint order) by diffing against `existing` (the previous pass's set) — added ones
/// are `Append`ed once, removed ones are individually detached, and anything present in both passes
/// is updated *in place* (content/position/size) without ever touching `canvas.Children()` at all.
///
/// This deliberately never does a wholesale `Children.Clear()`. Composition islands and native
/// children are both reconciled independently, so visual-tree structure changes only when a
/// `Text`/`NativeControl` command or composition island is genuinely new or gone.
pub(crate) fn reconcile_native_children(
    canvas: &Canvas,
    existing: &RefCell<NativeChildMap>,
    wanted: Vec<(NativeChildKey, RenderedNativeChild)>,
    render_tree: &Rc<RefCell<Option<elwindui_core::graphics::RenderTree>>>,
    keyboard: &Rc<KeyboardDispatcher>,
    host_id: Option<u64>,
) {
    let Ok(children) = canvas.Children() else {
        return;
    };
    let mut existing = existing.borrow_mut();
    // Count before mutating/appending so every genuinely new NativeControl in this one replay
    // shares exactly one readiness barrier. TextBlock projections deliberately do not participate.
    let new_native_count = wanted
        .iter()
        .filter(|(key, wanted_child)| {
            matches!(wanted_child, RenderedNativeChild::Native { .. })
                && !matches!(existing.get(key), Some(NativeChildElement::Native(_)))
        })
        .count();
    let native_load_batch = if new_native_count == 0 {
        None
    } else {
        if std::env::var_os("ELWINDUI_WINUI3_DIAGNOSTICS").is_some() {
            eprintln!(
                "[elwindui-winui3] native_load_batch host={} members={}",
                host_id
                    .map(|host_id| host_id.to_string())
                    .unwrap_or_else(|| "none".to_string()),
                new_native_count
            );
        }
        let root = {
            let render_tree = render_tree.borrow();
            render_tree.as_ref().and_then(|render_tree| {
                render_tree
                    .visual_index
                    .get(&render_tree.root.id)
                    .and_then(Weak::upgrade)
                    .map(|root| Rc::downgrade(&root))
            })
        };
        Some(Rc::new(NativeLoadBatch::new(
            new_native_count,
            root,
            host_id,
        )))
    };
    emit_diagnostic_native_rects(host_id, &wanted);
    let mut still_wanted: std::collections::HashSet<NativeChildKey> =
        std::collections::HashSet::new();
    for (key, wanted_child) in wanted {
        still_wanted.insert(key);
        match (existing.get(&key), wanted_child) {
            (
                Some(NativeChildElement::Text(text_block)),
                RenderedNativeChild::Text {
                    content,
                    rect,
                    style,
                    foreground,
                    alignment,
                    transform,
                    opacity,
                },
            ) => {
                let _ = text_block.SetText(&HSTRING::from(content.as_str()));
                // Font metrics use the same helper as measurement; an absent foreground clears
                // the local XAML value so the active ThemeResource keeps driving text paint.
                // `WinUi3TextBackend::measure_text` used to measure this same content — see that
                // function's own doc comment for why measurement and drawing must never diverge.
                let _ = crate::render::apply_text_style_to_text_block_with_foreground(
                    text_block,
                    &style,
                    foreground.as_ref(),
                );
                let _ = text_block.SetTextAlignment(xaml_text_alignment(alignment));
                let fe: FrameworkElement = text_block
                    .clone()
                    .cast()
                    .expect("TextBlock is a FrameworkElement");
                let _ = fe.SetWidth(rect.width as f64);
                let _ = fe.SetHeight(rect.height as f64);
                let _ = Canvas::SetLeft(&fe, rect.x as f64);
                let _ = Canvas::SetTop(&fe, rect.y as f64);
                apply_visual_projection(&fe, transform, opacity, false);
            }
            (
                Some(NativeChildElement::Native(state)),
                RenderedNativeChild::Native {
                    view: new_view,
                    rect,
                    transform,
                    opacity,
                    input_enabled,
                },
            ) => {
                let _ = new_view; // same underlying handle identity as `view` — see the key match above
                let mut view = state.view.clone();
                view.arrange(rect);
                apply_visual_projection(&view.as_element(), transform, opacity, input_enabled);
            }
            (_, wanted_child) => {
                // Either genuinely new (no `existing` entry) or the command's *kind* changed at
                // this exact key (rare — only if a UIElement's own `render()` emits a different
                // shape of commands than last time); either way, build fresh and attach once.
                let element = match wanted_child {
                    RenderedNativeChild::Text {
                        content,
                        rect,
                        style,
                        foreground,
                        alignment,
                        transform,
                        opacity,
                    } => {
                        let text_block = TextBlock::new().expect("TextBlock::new");
                        // This XAML child is a paint projection of a self-drawn ElwindUI node,
                        // not an input owner. Let native hit testing pass through to the host
                        // Canvas so the common render-tree hit test chooses the actual target.
                        let ui: UIElement =
                            text_block.clone().cast().expect("TextBlock is a UIElement");
                        let _ = ui.SetIsHitTestVisible(false);
                        let _ = text_block.SetText(&HSTRING::from(content.as_str()));
                        let _ = crate::render::apply_text_style_to_text_block_with_foreground(
                            &text_block,
                            &style,
                            foreground.as_ref(),
                        );
                        let _ = text_block.SetTextAlignment(xaml_text_alignment(alignment));
                        let fe: FrameworkElement = text_block
                            .clone()
                            .cast()
                            .expect("TextBlock is a FrameworkElement");
                        let _ = fe.SetWidth(rect.width as f64);
                        let _ = fe.SetHeight(rect.height as f64);
                        let _ = Canvas::SetLeft(&fe, rect.x as f64);
                        let _ = Canvas::SetTop(&fe, rect.y as f64);
                        apply_visual_projection(&fe, transform, opacity, false);
                        NativeChildElement::Text(text_block)
                    }
                    RenderedNativeChild::Native {
                        view,
                        rect,
                        transform,
                        opacity,
                        input_enabled,
                    } => {
                        let mut view = view;
                        view.arrange(rect);
                        apply_visual_projection(
                            &view.as_element(),
                            transform,
                            opacity,
                            input_enabled,
                        );
                        // Wired exactly once, right here (this whole match arm only runs for a
                        // genuinely new native child — an existing one takes the sibling arm above,
                        // which only calls `view.arrange(rect)`), mirroring
                        // `elwindui_backend_appkit::inner::ElwinduiWindow::make_first_responder`'s
                        // own one-time-wiring shape. `key.0` (the owner element's own
                        // `render_group_id` — see `NativeChildKey`'s own doc comment and
                        // `elwindui_core::ui::record_group_commands`, which always emits a
                        // `NativeControl` command as its owning element's *own* group) is the
                        // `owner_id` `elwindui_core::focus::native_focus_gained`/`native_focus_lost`
                        // need. See `elwindui_backend_appkit::inner::resolve_focus_owner`'s own doc
                        // comment for why AppKit needs a window-level responder-chain walk to get
                        // this same information WinUI3 already has for free here: `GotFocus`/
                        // `LostFocus` are ordinary bubbling routed events on any `FrameworkElement`,
                        // no subclassing needed.
                        let owner_id = key.0;
                        let element = view.as_element();
                        let render_tree_for_gained = Rc::downgrade(render_tree);
                        let keyboard_for_gained = Rc::downgrade(keyboard);
                        // Resolves through `render_tree.borrow()` in its own `let` statement, ending
                        // that borrow *before* calling `native_focus_gained` — mirrors
                        // `elwindui_backend_appkit::inner::ElwinduiWindow::make_first_responder`'s
                        // own fix; see that method's doc comment for the concrete double-borrow
                        // crash this avoids (`native_focus_gained` dispatches `on_got_focus`, which
                        // can run user code that synchronously re-enters this same `render_tree` via
                        // `RelayoutHost::request_relayout`).
                        let callback_owner = UiCallbackRegistryOwner::default();
                        let got_focus_id = callback_owner.register_event(Rc::new(move || {
                            let render_tree: Option<
                                Rc<RefCell<Option<elwindui_core::graphics::RenderTree>>>,
                            > = render_tree_for_gained.upgrade();
                            let keyboard: Option<Rc<KeyboardDispatcher>> =
                                keyboard_for_gained.upgrade();
                            if let (Some(render_tree), Some(keyboard)) = (render_tree, keyboard) {
                                let target = render_tree.borrow().as_ref().and_then(|rt| {
                                    elwindui_core::focus::resolve_native_focus_target(rt, owner_id)
                                });
                                if let Some(target) = target {
                                    elwindui_core::focus::native_focus_gained(
                                        &target,
                                        &keyboard.as_ref().focus,
                                        FocusState::Pointer,
                                    );
                                }
                            }
                        }));
                        let got_focus_token = element
                            .GotFocus(&RoutedEventHandler::new(move |_, _| {
                                invoke_ui_event_callback(got_focus_id);
                                Ok(())
                            }))
                            .ok();
                        let keyboard_for_lost = Rc::downgrade(keyboard);
                        let lost_focus_id = callback_owner.register_event(Rc::new(move || {
                            let keyboard: Option<Rc<KeyboardDispatcher>> =
                                keyboard_for_lost.upgrade();
                            if let Some(keyboard) = keyboard {
                                elwindui_core::focus::native_focus_lost(
                                    &keyboard.as_ref().focus,
                                    owner_id,
                                );
                            }
                        }));
                        let lost_focus_token = element
                            .LostFocus(&RoutedEventHandler::new(move |_, _| {
                                invoke_ui_event_callback(lost_focus_id);
                                Ok(())
                            }))
                            .ok();
                        let loaded_ticket = native_load_batch
                            .as_ref()
                            .map(|batch| NativeLoadTicket::new(batch));
                        let loaded_callback_id = loaded_ticket.as_ref().map(|ticket| {
                            let ticket_for_loaded = ticket.clone();
                            callback_owner.register_one_shot_event(Rc::new(move || {
                                ticket_for_loaded.resolve_loaded();
                            }))
                        });
                        let loaded_token = match loaded_callback_id {
                            Some(loaded_callback_id) => {
                                let loaded_callback = RoutedEventHandler::new(move |_, _| {
                                    invoke_ui_event_callback(loaded_callback_id);
                                    Ok(())
                                });
                                match element.Loaded(&loaded_callback) {
                                    Ok(token) => Some(token),
                                    Err(error) => {
                                        callback_owner.unregister_event(loaded_callback_id);
                                        if let Some(ticket) = loaded_ticket.as_ref() {
                                            ticket.resolve_cancelled();
                                        }
                                        if std::env::var_os("ELWINDUI_WINUI3_DIAGNOSTICS").is_some()
                                        {
                                            eprintln!(
                                                "[elwindui-winui3] FrameworkElement.Loaded registration failed: {error:?}"
                                            );
                                        }
                                        None
                                    }
                                }
                            }
                            None => None,
                        };
                        NativeChildElement::Native(NativeChildState {
                            view,
                            loaded_token,
                            loaded_ticket,
                            got_focus_token,
                            lost_focus_token,
                            callback_owner,
                        })
                    }
                };
                // Only reached with a stale `existing` entry if the command's *kind* changed at
                // this key (Text <-> NativeControl) — detach the old element first in that case.
                if let Some(old) = existing.remove(&key) {
                    let old_fe = old.framework_element();
                    let mut index = 0u32;
                    if children.IndexOf(&old_fe, &mut index).unwrap_or(false) {
                        let _ = children.RemoveAt(index);
                    }
                }
                let element_handle = element.framework_element();
                if let Err(error) = children.Append(&element_handle) {
                    if std::env::var_os("ELWINDUI_WINUI3_DIAGNOSTICS").is_some() {
                        eprintln!(
                            "[elwindui-winui3] native child append failed; cancelling Loaded participation: {error:?}"
                        );
                    }
                    drop(element);
                    continue;
                }
                // Loaded may have fired synchronously during Append. The explicit check closes
                // the already-loaded race; NativeLoadTicket makes either order idempotent.
                if element_handle.IsLoaded().unwrap_or(false) {
                    if let NativeChildElement::Native(state) = &element {
                        if let Some(ticket) = state.loaded_ticket.as_ref() {
                            ticket.resolve_loaded();
                        }
                    }
                }
                existing.insert(key, element);
            }
        }
    }
    existing.retain(|key, element| {
        if still_wanted.contains(key) {
            return true;
        }
        let fe = element.framework_element();
        let mut index = 0u32;
        if children.IndexOf(&fe, &mut index).unwrap_or(false) {
            let _ = children.RemoveAt(index);
        }
        false
    });
}

/// One `RenderCommand::Text`/`NativeControl`, resolved to absolute (origin-adjusted) coordinates —
/// the value half of `reconcile_native_children`'s diff (the key half is `NativeChildKey`).
pub(crate) enum RenderedNativeChild {
    Text {
        content: String,
        rect: elwindui_core::base::Rect,
        style: elwindui_core::graphics::ComputedTextStyle,
        foreground: Option<elwindui_core::graphics::Brush>,
        alignment: elwindui_core::graphics::TextAlignment,
        transform: elwindui_core::base::AffineTransform,
        opacity: f32,
    },
    Native {
        view: AnyView,
        rect: elwindui_core::base::Rect,
        transform: elwindui_core::base::AffineTransform,
        opacity: f32,
        input_enabled: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn batch(remaining: usize) -> Rc<NativeLoadBatch> {
        Rc::new(NativeLoadBatch::new(remaining, None, None))
    }

    fn ticket(batch: &Rc<NativeLoadBatch>) -> NativeLoadTicket {
        NativeLoadTicket::new(batch)
    }

    #[test]
    fn native_load_ticket_resolves_duplicate_loaded_delivery_once() {
        let batch = batch(1);
        let ticket = ticket(&batch);

        ticket.resolve_loaded();
        ticket.resolve_loaded();

        assert_eq!(batch.remaining.get(), 0);
        assert!(batch.saw_loaded.get());
        assert!(batch.completed.get());
        assert_eq!(batch.ready_completion_count.get(), 1);
    }

    #[test]
    fn native_load_batch_mixed_loaded_and_cancelled_members_completes_once() {
        let batch = batch(3);
        let loaded = ticket(&batch);
        let cancelled_a = ticket(&batch);
        let cancelled_b = ticket(&batch);

        loaded.resolve_loaded();
        cancelled_a.resolve_cancelled();
        cancelled_b.resolve_cancelled();
        loaded.resolve_loaded();

        assert_eq!(batch.remaining.get(), 0);
        assert!(batch.saw_loaded.get());
        assert!(batch.completed.get());
        assert_eq!(batch.ready_completion_count.get(), 1);
    }

    #[test]
    fn native_load_batch_all_cancelled_members_do_not_request_readiness_relayout() {
        let batch = batch(2);
        let cancelled_a = ticket(&batch);
        let cancelled_b = ticket(&batch);

        cancelled_a.resolve_cancelled();
        cancelled_b.resolve_cancelled();

        assert_eq!(batch.remaining.get(), 0);
        assert!(!batch.saw_loaded.get());
        assert!(batch.completed.get());
        assert_eq!(batch.ready_completion_count.get(), 0);
    }

    #[test]
    fn native_load_batch_one_or_many_members_have_one_readiness_completion() {
        for member_count in [1, 20] {
            let batch = batch(member_count);
            let tickets: Vec<_> = (0..member_count).map(|_| ticket(&batch)).collect();
            for ticket in tickets {
                ticket.resolve_loaded();
            }
            assert_eq!(batch.ready_completion_count.get(), 1);
        }
    }

    #[test]
    fn resolved_native_load_ticket_releases_its_batch_reference() {
        let batch = batch(1);
        let weak_batch = Rc::downgrade(&batch);
        let ticket = ticket(&batch);
        drop(batch);
        assert!(weak_batch.upgrade().is_some());

        ticket.resolve_cancelled();

        assert!(weak_batch.upgrade().is_none());
    }
}
