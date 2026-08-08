//! Cheap, cfg-gated counters for the AppKit replay pass's Core Animation traffic and memory
//! footprint (`docs/status/implementation_status.md`-adjacent optimization work). Every counter
//! update goes through [`bump`], a closure-taking function rather than a macro — in the disabled
//! configuration `bump`'s body is empty, so a call site like
//! `crate::render::stats::bump(|s| s.cgpaths_created += 1)` compiles away entirely under normal
//! inlining, with no macro needed to achieve that.
//!
//! Enabled under `cfg(test)` (so `cargo test` sees real counters with no feature flag to
//! remember), `cfg(debug_assertions)` (so `cargo run` sees them too), or the explicit
//! `render-stats` Cargo feature (an escape hatch for measuring an optimized build).

/// One relayout pass's Core Animation traffic, resource creation, and memory footprint. Reset with
/// [`reset`] and read with [`snapshot`] — see this module's own doc comment for the gating rule.
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RenderStats {
    // Core Animation tree mutation
    pub(crate) groups_visited: u32,
    pub(crate) groups_rebuilt: u32,
    pub(crate) groups_cache_hit: u32,
    pub(crate) layers_created: u32,
    pub(crate) layers_removed: u32,
    pub(crate) add_sublayer_calls: u32,
    pub(crate) subview_added: u32,
    pub(crate) subview_removed: u32,
    /// Property setters actually issued (`set_frame_if_changed`/`set_contents_scale_if_changed`)
    /// vs. skipped because the new value equaled the old one.
    pub(crate) setter_calls: u32,
    pub(crate) setter_calls_skipped: u32,
    // resource creation
    pub(crate) cgpaths_created: u32,
    pub(crate) cgcolors_created: u32,
    pub(crate) text_layers_created: u32,
    pub(crate) attributed_strings_created: u32,
    pub(crate) ns_fonts_created: u32,
    // memory — populated by callers via `cache_bytes`/`phys_footprint_bytes`, not by `bump`
    pub(crate) image_cache_bytes: u64,
    pub(crate) vector_raster_cache_bytes: u64,
    pub(crate) process_footprint_bytes: u64,
}

#[cfg(any(test, debug_assertions, feature = "render-stats"))]
mod enabled {
    use super::RenderStats;
    use std::cell::Cell;

    thread_local! {
        static STATS: Cell<RenderStats> = Cell::new(RenderStats::default());
    }

    pub(crate) fn reset() {
        STATS.with(|s| s.set(RenderStats::default()));
    }

    pub(crate) fn snapshot() -> RenderStats {
        STATS.with(|s| s.get())
    }

    pub(crate) fn bump(f: impl FnOnce(&mut RenderStats)) {
        STATS.with(|s| {
            let mut value = s.get();
            f(&mut value);
            s.set(value);
        });
    }
}

#[cfg(not(any(test, debug_assertions, feature = "render-stats")))]
mod disabled {
    use super::RenderStats;

    pub(crate) fn reset() {}

    pub(crate) fn snapshot() -> RenderStats {
        RenderStats::default()
    }

    #[inline(always)]
    pub(crate) fn bump(_f: impl FnOnce(&mut RenderStats)) {}
}

#[cfg(any(test, debug_assertions, feature = "render-stats"))]
pub(crate) use enabled::{bump, reset, snapshot};
#[cfg(not(any(test, debug_assertions, feature = "render-stats")))]
pub(crate) use disabled::{bump, reset, snapshot};

/// Reads this process's `phys_footprint` (the same figure Activity Monitor's "Memory" column
/// reports) via `task_info(TASK_VM_INFO)`. Only the fields up to and including `phys_footprint`
/// are declared here — `task_info` is told the (truncated) size of this buffer via `count`, so it
/// only ever writes that many words into it; the real kernel struct has further trailing fields
/// (peak/lifetime counters) that this deliberately never asks for. See `<mach/task_info.h>`'s
/// `task_vm_info` for the authoritative layout this mirrors.
pub(crate) fn phys_footprint_bytes() -> u64 {
    #[repr(C)]
    struct TaskVmInfo {
        virtual_size: u64,
        region_count: i32,
        page_size: i32,
        resident_size: u64,
        resident_size_peak: u64,
        device: u64,
        device_peak: u64,
        internal: u64,
        internal_peak: u64,
        external: u64,
        external_peak: u64,
        reusable: u64,
        reusable_peak: u64,
        purgeable_volatile_pmap: u64,
        purgeable_volatile_resident: u64,
        purgeable_volatile_virtual: u64,
        compressed: u64,
        compressed_peak: u64,
        compressed_lifetime: u64,
        phys_footprint: u64,
    }

    const TASK_VM_INFO: i32 = 22;

    unsafe extern "C" {
        fn mach_task_self() -> u32;
        fn task_info(
            target_task: u32,
            flavor: i32,
            task_info_out: *mut u32,
            task_info_out_cnt: *mut u32,
        ) -> i32;
    }

    let mut info: TaskVmInfo = unsafe { std::mem::zeroed() };
    let mut count = (std::mem::size_of::<TaskVmInfo>() / std::mem::size_of::<u32>()) as u32;
    let result = unsafe {
        task_info(
            mach_task_self(),
            TASK_VM_INFO,
            (&raw mut info) as *mut u32,
            &mut count,
        )
    };
    if result == 0 { info.phys_footprint } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_then_bump_reads_back_the_bumped_value() {
        reset();
        bump(|s| s.cgpaths_created += 1);
        bump(|s| s.cgpaths_created += 1);
        assert_eq!(snapshot().cgpaths_created, 2);
        reset();
        assert_eq!(snapshot().cgpaths_created, 0);
    }

    #[test]
    fn phys_footprint_bytes_returns_a_plausible_nonzero_value() {
        // A real process always has *some* physical footprint — this is mostly a smoke test that
        // the `task_info` call succeeds and the struct layout above lines up with the kernel's.
        assert!(phys_footprint_bytes() > 0);
    }
}
