//! Reconciling the native (non-drawn) children a render pass asks for against the XAML
//! children actually parented under the host panel.
//!
//! Lives under `host` rather than `render` because it operates on `TreeHostPanel`'s own child
//! bookkeeping — it is this panel's rendering pass, not stateless translation.


use crate::render::{solid_color_brush, xaml_text_alignment};
use crate::ffi::AnyView;
use super::*;

use crate::bindings;
use crate::render::composition::{
    CompositionClipSpec, CompositionPrimitive, CompositionRenderer, DesiredCompositionIsland,
    DesiredCompositionNode, IslandId,
};
use crate::bindings::Microsoft::UI::Input::InputKeyboardSource;
use crate::bindings::Microsoft::UI::Xaml::Controls::{
    Button as XamlButton, Canvas, MenuFlyoutItem, MenuFlyoutItemBase, PasswordBox as XamlPasswordBox,
    ScrollMode, ScrollViewer, TabView as XamlTabView, TabViewCloseButtonOverlayMode, TabViewItem,
    TabViewTabCloseRequestedEventArgs, TextBlock, TextBox as XamlTextBox,
};
use crate::bindings::Microsoft::UI::Xaml::Input::{
    CharacterReceivedRoutedEventArgs, KeyEventHandler, KeyboardAccelerator,
};
use crate::bindings::Microsoft::UI::Xaml::Media::SolidColorBrush;
use crate::bindings::Microsoft::UI::Xaml::{
    FrameworkElement, RoutedEventHandler, SizeChangedEventHandler, UIElement, Window as XamlWindow,
};
use crate::bindings::Microsoft::Graphics::Canvas::UI::Composition::CanvasComposition;
use crate::bindings::Microsoft::Graphics::Canvas::{
    CanvasActiveLayer, CanvasAntialiasing, CanvasBitmap, CanvasBlend, CanvasEdgeBehavior, CanvasImageInterpolation,
    ICanvasResourceCreator,
};
use crate::bindings::Microsoft::UI::Composition::CompositionDrawingSurface;
use crate::bindings::Microsoft::Graphics::Canvas::Brushes::{
    CanvasGradientStop, CanvasImageBrush, CanvasLinearGradientBrush, CanvasRadialGradientBrush,
    CanvasSolidColorBrush, ICanvasBrush,
};
use crate::bindings::Microsoft::Graphics::Canvas::Geometry::{
    CanvasArcSize, CanvasFigureLoop, CanvasFilledRegionDetermination, CanvasGeometry,
    CanvasPathBuilder, CanvasSweepDirection,
};
use crate::bindings::Microsoft::UI::Xaml::Controls::{
    SelectionChangedEventHandler, TextChangedEventHandler,
};
use windows::Foundation::{PropertyValue, Size, TypedEventHandler};
use windows::Graphics::{PointInt32, SizeInt32};
use windows::System::{VirtualKey, VirtualKeyModifiers};
use windows::UI::{Color, Core::CoreVirtualKeyStates};
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream, IRandomAccessStream};
use elwindui_core::input::{
    FocusState, Key, KeyModifiers, KeyboardDispatcher, RawKeyEvent, RawKeyEventKind,
    RawTextInputEvent, ShortcutRegistry,
};
use elwindui_core::ui::{FocusHost, UIElementExt as _};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicUsize, Ordering};
use windows::core::{HSTRING, Interface, Result};

/// A `RenderCommand::Text`/`NativeControl` command's reflection as a real XAML child, kept across
/// relayout passes so it can be updated in place instead of torn down and recreated — see
/// `reconcile_native_children`'s own doc comment for why.
#[derive(Clone)]
pub(crate) enum NativeChildElement {
    Text(TextBlock),
    Native(AnyView),
}

impl NativeChildElement {
    fn framework_element(&self) -> FrameworkElement {
        match self {
            NativeChildElement::Text(t) => t.clone().cast().expect("TextBlock is a FrameworkElement"),
            NativeChildElement::Native(v) => v.as_element(),
        }
    }
}

/// Keyed by `(originating RenderGroup id, index of the command within that group's own
/// `commands`)` — stable across relayout passes for the common case of a UIElement's `render()`
/// always emitting the same shape of commands, so a `Text`/`NativeControl` producer that's merely
/// being updated in place (content, position, size) is told apart from one that's genuinely new or
/// gone. Reused directly as `HashMap` keys by both `TreeHostPanel` (owns it) and `WinUI3RelayoutHost`
/// (holds a `Weak` reference, same pattern as `tree`/`render_tree`).
pub(crate) type NativeChildKey = (u64, usize);

pub(crate) type NativeChildMap = HashMap<NativeChildKey, NativeChildElement>;

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
) {
    let Ok(children) = canvas.Children() else {
        return;
    };
    let mut existing = existing.borrow_mut();
    let mut still_wanted: std::collections::HashSet<NativeChildKey> = std::collections::HashSet::new();
    for (key, wanted_child) in wanted {
        still_wanted.insert(key);
        match (existing.get(&key), wanted_child) {
            (Some(NativeChildElement::Text(text_block)), RenderedNativeChild::Text { content, rect, color, alignment }) => {
                let _ = text_block.SetText(&HSTRING::from(content.as_str()));
                if let Ok(brush) = solid_color_brush(color.unwrap_or(elwindui_core::graphics::Color::black())) {
                    let _ = text_block.SetForeground(&brush);
                }
                let _ = text_block.SetTextAlignment(xaml_text_alignment(alignment));
                let fe: FrameworkElement = text_block.clone().cast().expect("TextBlock is a FrameworkElement");
                let _ = fe.SetWidth(rect.width as f64);
                let _ = fe.SetHeight(rect.height as f64);
                let _ = Canvas::SetLeft(&fe, rect.x as f64);
                let _ = Canvas::SetTop(&fe, rect.y as f64);
            }
            (Some(NativeChildElement::Native(view)), RenderedNativeChild::Native { view: new_view, rect }) => {
                let _ = new_view; // same underlying handle identity as `view` — see the key match above
                let mut view = view.clone();
                view.arrange(rect);
            }
            (_, wanted_child) => {
                // Either genuinely new (no `existing` entry) or the command's *kind* changed at
                // this exact key (rare — only if a UIElement's own `render()` emits a different
                // shape of commands than last time); either way, build fresh and attach once.
                let element = match wanted_child {
                    RenderedNativeChild::Text { content, rect, color, alignment } => {
                        let text_block = TextBlock::new().expect("TextBlock::new");
                        let _ = text_block.SetText(&HSTRING::from(content.as_str()));
                        if let Ok(brush) = solid_color_brush(color.unwrap_or(elwindui_core::graphics::Color::black())) {
                            let _ = text_block.SetForeground(&brush);
                        }
                        let _ = text_block.SetTextAlignment(xaml_text_alignment(alignment));
                        let fe: FrameworkElement = text_block.clone().cast().expect("TextBlock is a FrameworkElement");
                        let _ = fe.SetWidth(rect.width as f64);
                        let _ = fe.SetHeight(rect.height as f64);
                        let _ = Canvas::SetLeft(&fe, rect.x as f64);
                        let _ = Canvas::SetTop(&fe, rect.y as f64);
                        NativeChildElement::Text(text_block)
                    }
                    RenderedNativeChild::Native { view, rect } => {
                        let mut view = view;
                        view.arrange(rect);
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
                        let got_focus_id = register_ui_event_callback(Rc::new(move || {
                            if let (Some(render_tree), Some(keyboard)) =
                                (render_tree_for_gained.upgrade(), keyboard_for_gained.upgrade())
                            {
                                let target = render_tree.borrow().as_ref().and_then(|rt| {
                                    elwindui_core::focus::resolve_native_focus_target(rt, owner_id)
                                });
                                if let Some(target) = target {
                                    elwindui_core::focus::native_focus_gained(
                                        &target,
                                        &keyboard.focus,
                                        FocusState::Pointer,
                                    );
                                }
                            }
                        }));
                        let _ = element.GotFocus(&RoutedEventHandler::new(move |_, _| {
                            invoke_ui_event_callback(got_focus_id);
                            Ok(())
                        }));
                        let keyboard_for_lost = Rc::downgrade(keyboard);
                        let lost_focus_id = register_ui_event_callback(Rc::new(move || {
                            if let Some(keyboard) = keyboard_for_lost.upgrade() {
                                elwindui_core::focus::native_focus_lost(&keyboard.focus, owner_id);
                            }
                        }));
                        let _ = element.LostFocus(&RoutedEventHandler::new(move |_, _| {
                            invoke_ui_event_callback(lost_focus_id);
                            Ok(())
                        }));
                        NativeChildElement::Native(view)
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
                let _ = children.Append(&element.framework_element());
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
        color: Option<elwindui_core::graphics::Color>,
        alignment: elwindui_core::graphics::TextAlignment,
    },
    Native {
        view: AnyView,
        rect: elwindui_core::base::Rect,
    },
}
