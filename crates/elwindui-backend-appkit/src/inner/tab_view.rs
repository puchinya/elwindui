//! A hand-built tab strip (chips + content area) rather than `NSTabView`, so the close button
//! and "new tab" affordance can be laid out the way the DSL describes them.

use super::InnerButton;
use crate::ffi::{AnyView, mtm, new_stack};
use crate::host::TreeHostView;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AnyThread, DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAccessibility, NSBezierPath, NSBox, NSBoxType, NSButton, NSColor, NSControlSize, NSStackView,
    NSTrackingArea, NSTrackingAreaOptions, NSUserInterfaceLayoutOrientation, NSView,
};
use objc2_foundation::{NSEdgeInsets, NSObjectProtocol, NSPoint, NSRect};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Sole source of truth for a chip's close-button visibility/enablement/alpha/accessibility —
/// see `close_presentation`'s own doc comment. Kept as a plain data type (rather than applying
/// the four fields directly from inline booleans at each call site) so the state matrix is
/// unit-testable without a screenshot.
#[derive(Debug, Clone, Copy, PartialEq)]
struct ClosePresentation {
    hidden: bool,
    enabled: bool,
    alpha: f64,
    accessibility_hidden: bool,
}

/// A non-closable tab must not reserve close-button layout space at all (`hidden = true` — an
/// `NSStackView` excludes a hidden arranged subview from layout, unlike an ordinary `NSView`).
/// A closable-but-inactive tab, by contrast, keeps the space reserved (`hidden = false`) so
/// hovering doesn't shift neighboring tabs, but hides the affordance itself via `alpha = 0` plus
/// disabling it for both mouse and assistive-technology interaction.
fn close_presentation(closable: bool, selected: bool, hovered: bool) -> ClosePresentation {
    if !closable {
        return ClosePresentation {
            hidden: true,
            enabled: false,
            alpha: 0.0,
            accessibility_hidden: true,
        };
    }
    if selected || hovered {
        ClosePresentation {
            hidden: false,
            enabled: true,
            alpha: 1.0,
            accessibility_hidden: false,
        }
    } else {
        ClosePresentation {
            hidden: false,
            enabled: false,
            alpha: 0.0,
            accessibility_hidden: true,
        }
    }
}

/// A rectangle rounded only at its top-left/top-right corners (square bottom) — the Safari/
/// Xcode-style tab shape, so a selected/hovered chip's fill flows straight into the content
/// area below it instead of reading as a fully-enclosed pill. `rect` is in the view's own
/// (non-flipped, bottom-left-origin) coordinate space, so "top" is the larger-`y` edge.
fn top_rounded_rect_path(rect: NSRect, radius: f64) -> Retained<NSBezierPath> {
    let min_x = rect.origin.x;
    let min_y = rect.origin.y;
    let max_x = rect.origin.x + rect.size.width;
    let max_y = rect.origin.y + rect.size.height;

    let path = NSBezierPath::bezierPath();
    path.moveToPoint(NSPoint::new(min_x, min_y));
    path.lineToPoint(NSPoint::new(min_x, max_y - radius));
    path.appendBezierPathWithArcWithCenter_radius_startAngle_endAngle_clockwise(
        NSPoint::new(min_x + radius, max_y - radius),
        radius,
        180.0,
        90.0,
        true,
    );
    path.lineToPoint(NSPoint::new(max_x - radius, max_y));
    path.appendBezierPathWithArcWithCenter_radius_startAngle_endAngle_clockwise(
        NSPoint::new(max_x - radius, max_y - radius),
        radius,
        90.0,
        0.0,
        true,
    );
    path.lineToPoint(NSPoint::new(max_x, min_y));
    path.closePath();
    path
}

/// Visual/native-interaction state owned by a single chip's own backing view — never
/// `TabViewItem`, `TreeHostView`, selected index, or ElwindUI callbacks, which stay with their
/// existing owners (`TabChipImpl`/`native_ui::TabView`).
pub(crate) struct TabChipViewIvars {
    selected: Cell<bool>,
    hovered: Cell<bool>,
    closable: Cell<bool>,
    /// The actual close `NSButton`, retained here (not just via `TabChipImpl`'s `InnerButton`) so
    /// `mouseEntered:`/`mouseExited:`/`set_selected`/`set_closable` can apply `close_presentation`
    /// without reaching back out through `TabChipImpl`.
    close_button: Retained<NSButton>,
    /// The single `NSTrackingArea` this chip keeps registered for itself — `updateTrackingAreas`
    /// removes the previous one before installing a freshly-sized replacement rather than
    /// accumulating a new one on every resize, mirroring `host::TreeHostView`'s own tracking area.
    tracking_area: RefCell<Option<Retained<NSTrackingArea>>>,
    /// Fired by `mouseDown:` below — lets clicking anywhere on the chip's own background (the
    /// padding around `title_button`, not just `title_button`'s own bounds) select the tab too,
    /// matching Safari/Xcode. `title_button` keeps its own separate `on_click` wiring
    /// (`InnerTabView::insert_tab`) for clicks that land squarely on it, since AppKit's hit
    /// testing gives that subview the event directly and this view's own `mouseDown:` never
    /// fires for it.
    on_select: RefCell<Option<Box<dyn Fn()>>>,
}

define_class!(
    #[unsafe(super(NSStackView))]
    #[thread_kind = objc2::MainThreadOnly]
    #[ivars = TabChipViewIvars]
    pub(crate) struct TabChipView;

    unsafe impl NSObjectProtocol for TabChipView {}

    impl TabChipView {
        /// Layerless Safari/Xcode-style tab rendering: adjacent tabs (the strip has zero
        /// inter-chip spacing — see `create_tab_strip`), a top-rounded-only fill for the
        /// selected/hovered chip (flat bottom edge, so the selected tab reads as flowing
        /// straight into the content area below rather than sitting in a fully-bordered pill),
        /// and a plain 1pt leading hairline divider between ordinary (non-selected,
        /// non-hovered) neighbors, since with zero spacing there is otherwise no visual
        /// boundary between two adjacent unselected tabs. Selected wins over hovered. No
        /// stroke/border is drawn around the selected tab itself — Safari/Xcode distinguish it
        /// by fill alone. Every color is resolved fresh on each call rather than cached, so
        /// Light/Dark appearance keeps resolving through AppKit.
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, dirty_rect: NSRect) {
            unsafe {
                let _: () = msg_send![super(self), drawRect: dirty_rect];
            }
            let selected = self.ivars().selected.get();
            let hovered = self.ivars().hovered.get();
            let bounds = self.bounds();
            if selected {
                let path = top_rounded_rect_path(bounds, 5.0);
                NSColor::controlBackgroundColor().setFill();
                path.fill();
            } else if hovered {
                let path = top_rounded_rect_path(bounds, 5.0);
                NSColor::unemphasizedSelectedContentBackgroundColor().setFill();
                path.fill();
            } else {
                let divider = NSBezierPath::bezierPath();
                divider.moveToPoint(NSPoint::new(bounds.origin.x + 0.5, bounds.origin.y));
                divider.lineToPoint(NSPoint::new(
                    bounds.origin.x + 0.5,
                    bounds.origin.y + bounds.size.height,
                ));
                divider.setLineWidth(1.0);
                NSColor::separatorColor().setStroke();
                divider.stroke();
            }
        }

        /// Background-of-the-chip click-to-select — see `on_select`'s own doc comment. Never
        /// fires for a click on `title_button`/`close_button` themselves (AppKit routes those
        /// directly to the subview under the cursor).
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, _event: &objc2_app_kit::NSEvent) {
            if let Some(callback) = self.ivars().on_select.borrow().as_ref() {
                callback();
            }
        }

        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking_areas(&self) {
            unsafe {
                let _: () = msg_send![super(self), updateTrackingAreas];
            }
            if let Some(old) = self.ivars().tracking_area.borrow_mut().take() {
                self.removeTrackingArea(&old);
            }
            let area = unsafe {
                NSTrackingArea::initWithRect_options_owner_userInfo(
                    NSTrackingArea::alloc(),
                    self.bounds(),
                    NSTrackingAreaOptions::MouseEnteredAndExited
                        | NSTrackingAreaOptions::ActiveInKeyWindow
                        | NSTrackingAreaOptions::InVisibleRect,
                    Some(self as &AnyObject),
                    None,
                )
            };
            self.addTrackingArea(&area);
            *self.ivars().tracking_area.borrow_mut() = Some(area);
        }

        /// Chrome-only hover — never routed through `elwindui_core`'s pointer dispatch and never
        /// touches `selected_index`.
        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, _event: &objc2_app_kit::NSEvent) {
            self.ivars().hovered.set(true);
            self.sync_close_presentation();
            self.setNeedsDisplay(true);
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &objc2_app_kit::NSEvent) {
            self.ivars().hovered.set(false);
            self.sync_close_presentation();
            self.setNeedsDisplay(true);
        }
    }
);

impl TabChipView {
    fn new(
        title_view: &NSView,
        close_view: &NSView,
        close_button: Retained<NSButton>,
        closable: bool,
    ) -> Retained<Self> {
        let m = mtm();
        let ivars = TabChipViewIvars {
            selected: Cell::new(false),
            hovered: Cell::new(false),
            closable: Cell::new(closable),
            close_button,
            tracking_area: RefCell::new(None),
            on_select: RefCell::new(None),
        };
        let this = Self::alloc(m).set_ivars(ivars);
        let this: Retained<Self> =
            unsafe { msg_send![super(this), initWithFrame: NSRect::default()] };
        this.setOrientation(NSUserInterfaceLayoutOrientation::Horizontal);
        this.setSpacing(4.0);
        this.setEdgeInsets(NSEdgeInsets {
            top: 2.0,
            left: 8.0,
            bottom: 2.0,
            right: 8.0,
        });
        this.addArrangedSubview(title_view);
        this.addArrangedSubview(close_view);
        // Safari/Xcode-style overflow: `title_view` gets low horizontal compression resistance
        // so it (not `close_view`, which stays at its own natural small fixed size) is what
        // actually shrinks/truncates as `chips_stack`'s `.Fill` distribution compresses every
        // chip to fit once their combined natural width exceeds the available strip width. A
        // hard floor on the *chip's own* width keeps a compressed tab from ever shrinking to
        // the point of being unreadable or unclickable.
        title_view.setContentCompressionResistancePriority_forOrientation(
            1.0,
            objc2_app_kit::NSLayoutConstraintOrientation::Horizontal,
        );
        let min_width = this
            .widthAnchor()
            .constraintGreaterThanOrEqualToConstant(70.0);
        // One below `NSLayoutPriorityRequired` (1000): `new_tab_button` (`create_tab_strip`)
        // gets *required* horizontal compression resistance specifically so that in a genuine
        // conflict — a window narrow enough that even every chip at this floor still doesn't
        // leave room for it — this floor is what yields (shrinking a chip below 70pt) rather
        // than `new_tab_button` being pushed out of the strip and made unreachable.
        min_width.setPriority(999.0);
        min_width.setActive(true);
        this.sync_close_presentation();
        this
    }

    /// Delegates to this chip's own layerless drawing state instead of `CALayer` — see
    /// `draw_rect`'s own doc comment.
    pub(crate) fn set_selected(&self, selected: bool) {
        self.ivars().selected.set(selected);
        self.sync_close_presentation();
        self.setNeedsDisplay(true);
    }

    pub(crate) fn set_on_select(&self, callback: Box<dyn Fn()>) {
        *self.ivars().on_select.borrow_mut() = Some(callback);
    }

    pub(crate) fn set_closable(&self, closable: bool) {
        self.ivars().closable.set(closable);
        self.sync_close_presentation();
    }

    fn sync_close_presentation(&self) {
        let p = close_presentation(
            self.ivars().closable.get(),
            self.ivars().selected.get(),
            self.ivars().hovered.get(),
        );
        let button = &self.ivars().close_button;
        button.setHidden(p.hidden);
        button.setEnabled(p.enabled);
        button.setAlphaValue(p.alpha);
        button.setAccessibilityHidden(p.accessibility_hidden);
    }
}

/// See docs/specs/ui_spec.md#tabs. A single tab's header: a title button (click to
/// select) plus a small close button, packed into one row so `TabStripImpl` can insert/remove it as
/// one unit. Purely an internal composition helper (never a real DSL-declared element), so
/// its two buttons are plain `InnerButton`s, not `native_ui::Button` — no use-site margin/alignment
/// ever applies to them.
pub(crate) struct TabChipImpl {
    ns: Retained<TabChipView>,
    pub(crate) title_button: InnerButton,
    pub(crate) close_button: InnerButton,
}

fn create_tab_chip(title: &str, closable: bool) -> TabChipImpl {
    let title_button = InnerButton::new();
    title_button.set_text(title);
    // Borderless: an `NSButton`'s default bezel is opaque and would otherwise cover almost the
    // entire chip row, hiding the chip's own selection/hover fill underneath it.
    title_button.set_bordered(false);

    let close_button = InnerButton::new();
    close_button.set_system_symbol_or_text("xmark", "×", "Close Tab");
    close_button.set_bordered(false);
    close_button.set_control_size(NSControlSize::Small);

    let ns = TabChipView::new(
        &title_button.handle.as_nsview(),
        &close_button.handle.as_nsview(),
        close_button.native_button(),
        closable,
    );
    TabChipImpl {
        ns,
        title_button,
        close_button,
    }
}

impl TabChipImpl {
    pub(crate) fn set_title(&self, title: &str) {
        self.title_button.set_text(title);
    }

    /// Highlights this chip's own row when it's the selected tab — delegates to `TabChipView`'s
    /// layerless `drawRect:`-based rendering (semantic AppKit colors), not a `CALayer` tint.
    pub(crate) fn set_selected(&self, selected: bool) {
        self.ns.set_selected(selected);
    }

    pub(crate) fn set_closable(&self, closable: bool) {
        self.ns.set_closable(closable);
    }

    /// Selects the tab when the chip's own background (not just `title_button`) is clicked —
    /// see `TabChipView::on_select`'s own doc comment.
    pub(crate) fn set_on_select(&self, callback: Box<dyn Fn()>) {
        self.ns.set_on_select(callback);
    }
}

/// The row of `TabChipImpl`s plus a trailing "+" button. `InnerTabView` owns one of these and the
/// content area below it; kept as a separate type since 付録Y's backend table describes it as its
/// own piece (a custom `NSStackView`-based strip, not `NSTabViewController`).
///
/// `chips_stack` is an ordinary (not scrolling) arranged subview of `ns` — each `TabChipView` gets
/// low horizontal compression resistance (`TabChipView::new`) plus a hard floor (its own minimum-
/// width constraint), so once enough tabs accumulate to overflow the available width, `chips_stack`'s
/// `.Fill` distribution shrinks every chip together (down to that floor) rather than either
/// scrolling or silently clipping tabs past the window's edge — matching Safari/Xcode's own real
/// overflow behavior (which shrinks tabs, not a scrollbar). `new_tab_button` stays outside
/// `chips_stack`, always at its own natural size.
pub(crate) struct TabStripImpl {
    ns: Retained<NSStackView>,
    chips_stack: Retained<NSStackView>,
    pub(crate) new_tab_button: InnerButton,
}

fn create_tab_strip() -> TabStripImpl {
    let new_tab_button = InnerButton::new();
    new_tab_button.set_system_symbol_or_text("plus", "+", "New Tab");
    new_tab_button.set_bordered(false);
    new_tab_button.set_control_size(NSControlSize::Small);
    // Required resistance: pairs with the one-below-required priority on each chip's own
    // minimum-width constraint (`TabChipView::new`) so this button is structurally the last
    // thing to ever get compressed away — see that constraint's own doc comment.
    new_tab_button
        .handle
        .as_nsview()
        .setContentCompressionResistancePriority_forOrientation(
            1000.0,
            objc2_app_kit::NSLayoutConstraintOrientation::Horizontal,
        );

    let chips_stack = new_stack(Vec::new(), NSUserInterfaceLayoutOrientation::Horizontal);
    // Zero inter-chip spacing plus zero top/bottom insets: Safari/Xcode-style tabs sit
    // directly adjacent to each other and fill the full height of the tab bar, rather than
    // floating as separated pills — `TabChipView::draw_rect`'s per-chip leading hairline
    // divider is what distinguishes ordinary (non-selected, non-hovered) neighbors instead.
    chips_stack.setSpacing(0.0);
    chips_stack.setEdgeInsets(NSEdgeInsets {
        top: 0.0,
        left: 4.0,
        bottom: 0.0,
        right: 0.0,
    });
    // `.Fill`: each chip gets its own natural width while there's room; once the row's total
    // natural width would exceed what's actually available, this is what makes Auto Layout
    // compress every chip (down to `TabChipView::new`'s own minimum-width floor) instead of
    // just letting them overflow unclipped past `chips_stack`'s own bounds.
    chips_stack.setDistribution(objc2_app_kit::NSStackViewDistribution::Fill);
    // Lowest possible hugging priority: under the outer strip's own `.Fill` distribution below,
    // this is the arranged subview that absorbs the strip's leftover width, leaving
    // `new_tab_button` pinned to its natural size at the trailing edge — mirrors
    // `content_container`'s own vertical hugging priority in `InnerTabView::new`.
    chips_stack.setContentHuggingPriority_forOrientation(
        1.0,
        objc2_app_kit::NSLayoutConstraintOrientation::Horizontal,
    );

    let ns = new_stack(
        vec![
            AnyView::from(chips_stack.clone()),
            new_tab_button.handle.clone(),
        ],
        NSUserInterfaceLayoutOrientation::Horizontal,
    );
    ns.setSpacing(4.0);
    ns.setEdgeInsets(NSEdgeInsets {
        top: 0.0,
        left: 0.0,
        bottom: 0.0,
        right: 6.0,
    });
    ns.setDistribution(objc2_app_kit::NSStackViewDistribution::Fill);

    TabStripImpl {
        ns,
        chips_stack,
        new_tab_button,
    }
}

impl TabStripImpl {
    /// Inserts a chip at position `index` within the chip row (never touching `new_tab_button`,
    /// which lives outside `chips_stack` entirely).
    fn insert_tab(&self, index: usize, title: &str, closable: bool) -> TabChipImpl {
        let chip = create_tab_chip(title, closable);
        let view: Retained<NSView> = Retained::into_super(Retained::into_super(chip.ns.clone()));
        self.chips_stack
            .insertArrangedSubview_atIndex(&view, index as isize);
        chip
    }

    fn remove_tab(&self, chip: &TabChipImpl) {
        let view: Retained<NSView> = Retained::into_super(Retained::into_super(chip.ns.clone()));
        self.chips_stack.removeArrangedSubview(&view);
        view.removeFromSuperview();
    }
}

/// See docs/specs/ui_spec.md#tabs. Vertical stack of `[TabStripImpl, separator, content_container]`
/// — composed by `native_ui::TabView`, which owns the mapping from its `children` collection's
/// `TabViewItem`s to `TabChipImpl`s + content hosts. This type only holds the widget areas — it has
/// no notion of "the list of tabs" on its own.
///
/// Each tab gets its own persistent `TreeHostView` (created once, in `insert_tab`), added as an
/// overlaid subview of `content_container` and shown/hidden via `set_tab_content_visible` rather
/// than destroyed and rebuilt — a single shared pane would have no way to restore a previously-
/// shown-then-hidden tab's content after switching away from it.
pub(crate) struct InnerTabView {
    handle: AnyView,
    pub(crate) strip: TabStripImpl,
    content_container: Retained<NSView>,
}

impl InnerTabView {
    pub(crate) fn new() -> Self {
        let m = mtm();
        let strip = create_tab_strip();
        let content_container = NSView::initWithFrame(NSView::alloc(m), NSRect::default());
        let strip_view: Retained<NSView> = Retained::into_super(strip.ns.clone());
        let separator = NSBox::initWithFrame(NSBox::alloc(m), NSRect::default());
        separator.setBoxType(NSBoxType::Separator);
        let separator_view: Retained<NSView> = Retained::into_super(separator);
        let root = NSStackView::stackViewWithViews(
            &objc2_foundation::NSArray::from_retained_slice(&[
                strip_view,
                separator_view,
                content_container.clone(),
            ]),
            m,
        );
        root.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
        // Chip/strip/separator carry their own spacing; the root stack itself stays flush so the
        // native separator sits directly against both neighbors.
        root.setSpacing(0.0);
        // `NSStackView`'s default `distribution` (`GravityAreas`) leaves each arranged subview at
        // its own intrinsic size unless hugging priorities say otherwise — `.Fill` makes the stack
        // actually consume its *entire* stacking-axis extent, matching the expected "chips row at
        // natural height, content area fills the rest" shape. `content_container`'s own vertical
        // hugging priority is dropped to (near-)zero so it — not the also-low-priority-by-default
        // `strip` — is the one that absorbs whatever space `Fill` distributes.
        content_container.setContentHuggingPriority_forOrientation(
            1.0,
            objc2_app_kit::NSLayoutConstraintOrientation::Vertical,
        );
        root.setDistribution(objc2_app_kit::NSStackViewDistribution::Fill);
        let handle = AnyView::from(root);
        Self {
            handle,
            strip,
            content_container,
        }
    }

    pub(crate) fn handle(&self) -> AnyView {
        self.handle.clone()
    }

    pub(crate) fn set_on_new_tab(&self, callback: Box<dyn Fn()>) {
        self.strip.new_tab_button.set_on_click(callback);
    }

    /// Inserts a new tab chip at `index` (wiring `on_select`/`on_close` to the given callbacks)
    /// plus a fresh, persistent content host — added to `content_container`, initially hidden.
    pub(crate) fn insert_tab(
        &self,
        index: usize,
        title: &str,
        closable: bool,
        on_select: Box<dyn Fn()>,
        on_close: Box<dyn Fn()>,
    ) -> (TabChipImpl, Retained<TreeHostView>) {
        let chip = self.strip.insert_tab(index, title, closable);
        // Shared between two click paths — `title_button`'s own `on_click` (AppKit routes a
        // click on its exact bounds straight to it) and the chip's own background `mouseDown:`
        // (everything else within the chip, e.g. the padding around the title) — so clicking
        // anywhere on the tab selects it, not just its literal text.
        let on_select: Rc<dyn Fn()> = Rc::from(on_select);
        chip.title_button.set_on_click({
            let on_select = Rc::clone(&on_select);
            Box::new(move || on_select())
        });
        chip.set_on_select(Box::new(move || on_select()));
        chip.close_button.set_on_click(on_close);

        let host = TreeHostView::new();
        // Classic pre-Auto-Layout "fill the parent" technique instead of `NSLayoutConstraint`s:
        // `translatesAutoresizingMaskIntoConstraints(true)` (this container has no Auto Layout
        // constraints of its own, so this is the default anyway, made explicit) plus a
        // `.width | .height` autoresizing mask makes AppKit stretch `host` to match
        // `content_container`'s bounds on every resize, with no custom `NSView` subclass or
        // constraint bookkeeping needed here.
        host.setTranslatesAutoresizingMaskIntoConstraints(true);
        host.setAutoresizingMask(
            objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
                | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        host.setFrame(self.content_container.bounds());
        host.setHidden(true);
        // Every new host starts suppressed (docs/design/runtime/layout_design.md) — only
        // `set_tab_content_visible(host, true)` below ever reactivates one, including the very
        // first time a tab becomes selected (see that method's own doc comment). Suppressing here
        // rather than leaving the default `active: true` matters even for a tab that never gets
        // selected at all: without it, the `host.set_tree(content)` call `native_ui::TabView::
        // rebuild` makes right after `insert_tab` returns would run a full, wasted `relayout()`
        // (measure/arrange/RenderTree/CALayer build) before this tab's actual visibility is even
        // decided.
        host.set_active(false);
        self.content_container.addSubview(&host);
        (chip, host)
    }

    /// Removes a tab's chip and its persistent content host together.
    pub(crate) fn remove_tab(&self, chip: &TabChipImpl, host: &TreeHostView) {
        self.strip.remove_tab(chip);
        // Explicit rather than relying on the last `Retained<TreeHostView>` being dropped by the
        // caller (which happens to be immediate today, but isn't guaranteed by anything at this
        // call site) — releases `render_tree`/every retained CALayer/native island deterministically
        // right here, matching `set_active`'s own doc comment.
        host.set_active(false);
        host.removeFromSuperview();
    }

    /// Shows or hides a tab's content host — selecting a tab means showing its host and hiding the
    /// previously-selected one, never touching either one's actual content. Activating before
    /// unhiding (and hiding before deactivating) avoids ever presenting a suppressed host's empty
    /// frame for even one paint.
    pub(crate) fn set_tab_content_visible(&self, host: &TreeHostView, visible: bool) {
        if visible {
            host.set_active(true);
            host.setHidden(false);
        } else {
            host.setHidden(true);
            host.set_active(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_presentation_matches_tab_state() {
        assert_eq!(
            close_presentation(false, false, false),
            ClosePresentation {
                hidden: true,
                enabled: false,
                alpha: 0.0,
                accessibility_hidden: true,
            },
            "non-closable must never reserve or expose the close affordance"
        );
        assert_eq!(
            close_presentation(true, true, false),
            ClosePresentation {
                hidden: false,
                enabled: true,
                alpha: 1.0,
                accessibility_hidden: false,
            }
        );
        assert_eq!(
            close_presentation(true, false, true),
            ClosePresentation {
                hidden: false,
                enabled: true,
                alpha: 1.0,
                accessibility_hidden: false,
            }
        );
        assert_eq!(
            close_presentation(true, false, false),
            ClosePresentation {
                hidden: false,
                enabled: false,
                alpha: 0.0,
                accessibility_hidden: true,
            },
            "closable-but-inactive must still reserve layout space (hidden=false) so hover \
             doesn't shift neighboring tabs, while keeping the affordance non-interactive"
        );
    }
}
