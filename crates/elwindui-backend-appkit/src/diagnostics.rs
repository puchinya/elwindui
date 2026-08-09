//! Opt-in process and hierarchy diagnostics for AppKit memory investigations.
//!
//! This module is available only with the `render-stats` feature. All functions must run on
//! AppKit's main thread because they inspect live `NSApplication`, `NSWindow`, `NSView`, and
//! `CALayer` objects.

use crate::ffi::mtm;
use crate::host::TreeHostView;
use crate::render;
use crate::render::stats;
use dispatch2::{DispatchQueue, DispatchTime};
use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadOnly, msg_send};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSButton, NSStackView,
    NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use objc2_quartz_core::{CAGradientLayer, CALayer, CAShapeLayer, CATextLayer};
use std::cell::RefCell;
use std::collections::HashSet;
use std::time::Duration;

thread_local! {
    static BASELINE_WINDOW: RefCell<Option<Retained<NSWindow>>> = const { RefCell::new(None) };
}

/// A process-wide AppKit memory and hierarchy snapshot.
///
/// The live object counts cover the `NSView` trees attached to windows returned by
/// `NSApplication.windows()`. `CALayer` counts additionally include retained render-group roots
/// owned by attached `TreeHostView`s, with object identities de-duplicated across both sources.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AppKitMemorySnapshot {
    /// macOS `phys_footprint`, the primary value shown by Activity Monitor's Memory column.
    pub physical_footprint_bytes: u64,
    /// Current resident bytes from `TASK_VM_INFO`; this is a supplementary metric.
    pub resident_bytes: u64,
    /// Number of attached `TreeHostView` objects reachable from application windows.
    pub attached_tree_host_count: u32,
    /// Attached tree hosts whose own `hidden` flag is set.
    pub hidden_tree_host_count: u32,
    /// Number of reachable AppKit `NSView` objects, including hosts and native controls.
    pub native_nsview_count: u32,
    /// Number of reachable views for which AppKit reports a backing layer.
    pub layer_backed_nsview_count: u32,
    /// Number of reachable native `NSStackView` objects.
    pub native_nsstackview_count: u32,
    /// Number of reachable native `NSButton` objects.
    pub native_nsbutton_count: u32,
    /// Number of unique reachable Core Animation layers, including mask-only layers.
    pub live_calayer_count: u32,
    /// Number of reachable layers that are `CAShapeLayer` instances.
    pub live_shape_layer_count: u32,
    /// Number of reachable layers that are `CATextLayer` instances.
    pub live_text_layer_count: u32,
    /// Number of reachable layers that are `CAGradientLayer` instances.
    pub live_gradient_layer_count: u32,
    /// Number of unique layers referenced through a `CALayer.mask` edge.
    pub live_mask_layer_count: u32,
    /// Sum of attached hosts' raster image-cache allocation estimates.
    pub image_cache_bytes: u64,
    /// Sum of attached hosts' vector-raster-cache allocation estimates.
    pub vector_raster_cache_bytes: u64,
    /// Existing render-stat count of manually created layers on the current AppKit thread.
    pub layers_created: u32,
    /// Existing render-stat count of manually created text layers on the current AppKit thread.
    pub text_layers_created: u32,
}

impl AppKitMemorySnapshot {
    /// Serializes the snapshot as a compact JSON object for line-oriented measurement scripts.
    pub fn to_json(self) -> String {
        format!(
            concat!(
                "{{\"physical_footprint_bytes\":{},\"resident_bytes\":{},",
                "\"attached_tree_host_count\":{},\"hidden_tree_host_count\":{},",
                "\"native_nsview_count\":{},\"layer_backed_nsview_count\":{},",
                "\"native_nsstackview_count\":{},\"native_nsbutton_count\":{},",
                "\"live_calayer_count\":{},\"live_shape_layer_count\":{},",
                "\"live_text_layer_count\":{},\"live_gradient_layer_count\":{},",
                "\"live_mask_layer_count\":{},\"image_cache_bytes\":{},",
                "\"vector_raster_cache_bytes\":{},\"layers_created\":{},",
                "\"text_layers_created\":{}}}"
            ),
            self.physical_footprint_bytes,
            self.resident_bytes,
            self.attached_tree_host_count,
            self.hidden_tree_host_count,
            self.native_nsview_count,
            self.layer_backed_nsview_count,
            self.native_nsstackview_count,
            self.native_nsbutton_count,
            self.live_calayer_count,
            self.live_shape_layer_count,
            self.live_text_layer_count,
            self.live_gradient_layer_count,
            self.live_mask_layer_count,
            self.image_cache_bytes,
            self.vector_raster_cache_bytes,
            self.layers_created,
            self.text_layers_created,
        )
    }
}

/// Captures the current process and attached AppKit view/layer hierarchy.
///
/// Call this only from AppKit's main thread, after the windows under test have been shown and had
/// time to settle. The function never mutates the hierarchy.
pub fn capture_memory_snapshot() -> AppKitMemorySnapshot {
    let app = NSApplication::sharedApplication(mtm());
    let mut snapshot = AppKitMemorySnapshot::default();
    let mut seen_layers = HashSet::new();
    let mut seen_masks = HashSet::new();
    for window in app.windows().iter() {
        if let Some(view) = window.contentView() {
            collect_view(view, &mut snapshot, &mut seen_layers, &mut seen_masks);
        }
    }
    let memory = stats::process_memory();
    let render_stats = stats::snapshot();
    snapshot.physical_footprint_bytes = memory.physical_footprint_bytes;
    snapshot.resident_bytes = memory.resident_bytes;
    snapshot.layers_created = render_stats.layers_created;
    snapshot.text_layers_created = render_stats.text_layers_created;
    snapshot
}

/// The fixed-content baseline cases supported by [`show_memory_baseline`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryBaselineCase {
    /// A plain empty `NSView` installed as the sole window content view.
    EmptyNsView,
    /// An empty `TreeHostView` with no explicitly requested backing layer.
    EmptyTreeHost,
    /// An empty `TreeHostView` forced to create an AppKit backing layer.
    LayerBackedTreeHost,
}

/// Shows a fixed 800×600 baseline window for the requested case.
///
/// Call this inside [`crate::application::run`]'s startup closure. The window is retained for the
/// rest of the process so that a delayed [`capture_memory_snapshot`] sees the requested hierarchy.
pub fn show_memory_baseline(case: MemoryBaselineCase) {
    let mtm = mtm();
    let content_rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(800.0, 600.0));
    let window: Retained<NSWindow> = unsafe {
        let alloc = NSWindow::alloc(mtm);
        msg_send![
            alloc,
            initWithContentRect: content_rect,
            styleMask: NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
            backing: NSBackingStoreType::Buffered,
            defer: false,
        ]
    };
    let content: Retained<NSView> = match case {
        MemoryBaselineCase::EmptyNsView => NSView::new(mtm),
        MemoryBaselineCase::EmptyTreeHost => Retained::into_super(TreeHostView::new()),
        MemoryBaselineCase::LayerBackedTreeHost => {
            let host = TreeHostView::new();
            host.setWantsLayer(true);
            Retained::into_super(host)
        }
    };
    window.setContentView(Some(&content));
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    window.makeKeyAndOrderFront(None);
    app.activate();
    BASELINE_WINDOW.with(|slot| *slot.borrow_mut() = Some(window));
}

pub(crate) fn schedule_env_report() {
    let Some(delay_ms) = std::env::var("ELWINDUI_APPKIT_MEMORY_REPORT_AFTER_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
    else {
        return;
    };
    let when = DispatchTime::try_from(Duration::from_millis(delay_ms))
        .expect("measurement delay must fit DispatchTime");
    let exit_after_report = std::env::var_os("ELWINDUI_APPKIT_MEMORY_EXIT_AFTER_REPORT").is_some();
    DispatchQueue::main()
        .after(when, move || {
            eprintln!(
                "elwindui-appkit-memory {}",
                capture_memory_snapshot().to_json()
            );
            if exit_after_report {
                NSApplication::sharedApplication(mtm()).terminate(None);
            }
        })
        .expect("main dispatch queue must accept a delayed memory report");
}

fn collect_view(
    view: Retained<NSView>,
    snapshot: &mut AppKitMemorySnapshot,
    seen_layers: &mut HashSet<usize>,
    seen_masks: &mut HashSet<usize>,
) {
    let host = view.clone().downcast::<TreeHostView>().ok();
    let layer = view.layer();
    record_view_metadata(
        snapshot,
        host.as_ref().is_some(),
        host.as_ref().is_some_and(|host| host.isHidden()),
        layer.is_some(),
        view.downcast_ref::<NSStackView>().is_some(),
        view.downcast_ref::<NSButton>().is_some(),
    );
    if let Some(layer) = layer {
        collect_layer(&layer, snapshot, seen_layers, seen_masks);
    }
    if let Some(host) = host {
        let state = host.ivars().replay_state.borrow();
        snapshot.image_cache_bytes += state
            .image_cache
            .values()
            .map(|image| render::cgimage_bytes(image))
            .sum::<u64>();
        snapshot.vector_raster_cache_bytes += state
            .vector_raster_cache
            .values()
            .map(|(_, _, _, image)| render::cgimage_bytes(image))
            .sum::<u64>();
        for layer in state.group_layers.values() {
            collect_layer(layer, snapshot, seen_layers, seen_masks);
        }
    }
    for child in view.subviews().iter() {
        collect_view(child.clone(), snapshot, seen_layers, seen_masks);
    }
}

fn record_view_metadata(
    snapshot: &mut AppKitMemorySnapshot,
    is_tree_host: bool,
    is_hidden: bool,
    is_layer_backed: bool,
    is_stack_view: bool,
    is_button: bool,
) {
    snapshot.native_nsview_count += 1;
    if is_layer_backed {
        snapshot.layer_backed_nsview_count += 1;
    }
    if is_stack_view {
        snapshot.native_nsstackview_count += 1;
    }
    if is_button {
        snapshot.native_nsbutton_count += 1;
    }
    if is_tree_host {
        snapshot.attached_tree_host_count += 1;
        if is_hidden {
            snapshot.hidden_tree_host_count += 1;
        }
    }
}

fn collect_layer(
    layer: &CALayer,
    snapshot: &mut AppKitMemorySnapshot,
    seen_layers: &mut HashSet<usize>,
    seen_masks: &mut HashSet<usize>,
) {
    let identity = std::ptr::from_ref(layer).cast::<()>() as usize;
    if !seen_layers.insert(identity) {
        return;
    }
    snapshot.live_calayer_count += 1;
    if layer.downcast_ref::<CAShapeLayer>().is_some() {
        snapshot.live_shape_layer_count += 1;
    }
    if layer.downcast_ref::<CATextLayer>().is_some() {
        snapshot.live_text_layer_count += 1;
    }
    if layer.downcast_ref::<CAGradientLayer>().is_some() {
        snapshot.live_gradient_layer_count += 1;
    }
    if let Some(mask) = layer.mask() {
        let mask_identity = std::ptr::from_ref(&*mask).cast::<()>() as usize;
        if seen_masks.insert(mask_identity) {
            snapshot.live_mask_layer_count += 1;
        }
        collect_layer(&mask, snapshot, seen_layers, seen_masks);
    }
    if let Some(sublayers) = unsafe { layer.sublayers() } {
        for child in sublayers.iter() {
            collect_layer(&child, snapshot, seen_layers, seen_masks);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_output_contains_every_measurement_field() {
        let json = AppKitMemorySnapshot::default().to_json();
        for key in [
            "physical_footprint_bytes",
            "resident_bytes",
            "attached_tree_host_count",
            "hidden_tree_host_count",
            "native_nsview_count",
            "layer_backed_nsview_count",
            "native_nsstackview_count",
            "native_nsbutton_count",
            "live_calayer_count",
            "live_shape_layer_count",
            "live_text_layer_count",
            "live_gradient_layer_count",
            "live_mask_layer_count",
            "image_cache_bytes",
            "vector_raster_cache_bytes",
            "layers_created",
            "text_layers_created",
        ] {
            assert!(json.contains(key), "missing {key}");
        }
    }

    #[test]
    fn layer_traversal_counts_types_masks_and_duplicate_references_once() {
        let root = CALayer::new();
        let shape = CAShapeLayer::new();
        let text = CATextLayer::new();
        let gradient = CAGradientLayer::new();
        root.addSublayer(&shape);
        root.addSublayer(&text);
        root.addSublayer(&gradient);
        // The retained shape outlives `root`; deliberately reusing it exercises identity
        // de-duplication across a sublayer edge and the non-sublayer mask edge.
        unsafe { root.setMask(Some(&shape)) };

        let mut snapshot = AppKitMemorySnapshot::default();
        collect_layer(
            &root,
            &mut snapshot,
            &mut HashSet::new(),
            &mut HashSet::new(),
        );

        assert_eq!(snapshot.live_calayer_count, 4);
        assert_eq!(snapshot.live_shape_layer_count, 1);
        assert_eq!(snapshot.live_text_layer_count, 1);
        assert_eq!(snapshot.live_gradient_layer_count, 1);
        assert_eq!(snapshot.live_mask_layer_count, 1);
    }

    #[test]
    fn view_metadata_counts_backing_layers_and_hidden_tree_hosts() {
        let mut snapshot = AppKitMemorySnapshot::default();
        record_view_metadata(&mut snapshot, false, false, true, true, false);
        record_view_metadata(&mut snapshot, true, false, true, true, true);
        record_view_metadata(&mut snapshot, true, true, true, false, true);

        assert_eq!(snapshot.native_nsview_count, 3);
        assert_eq!(snapshot.attached_tree_host_count, 2);
        assert_eq!(snapshot.hidden_tree_host_count, 1);
        assert_eq!(snapshot.layer_backed_nsview_count, 3);
        assert_eq!(snapshot.native_nsstackview_count, 2);
        assert_eq!(snapshot.native_nsbutton_count, 2);
    }
}
