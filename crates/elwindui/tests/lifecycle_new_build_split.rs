//! CI-2 of #80 (docs/design/runtime/component_lifecycle_design.md): `generate_view` now emits
//! view-construction statements into a separate `__build_view` method instead of inlining them
//! directly into `on_constructed`/`new()`. This is meant to be a purely mechanical, timing-
//! preserving split — `new()` must still trigger the full build (through `on_mount`) synchronously,
//! exactly once, by the time it returns. This test proves that observably, from outside the crate,
//! using a construction counter, so a later issue that actually defers this call to an explicit
//! `mount()` has a regression test to change deliberately rather than break silently.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use std::cell::Cell;
use std::rc::Rc;

thread_local! {
    static BUILD_COUNT: Cell<u32> = const { Cell::new(0) };
}

#[elwindui::component(inherits ContentControl)]
struct LifecycleBuildSplitProbe {
    body: view! {
        on_mount {
            BUILD_COUNT.with(|c| c.set(c.get() + 1));
        }
        TextBlock { text: "probe" }
    },
}

#[elwindui::component]
impl LifecycleBuildSplitProbe {}

#[test]
fn new_triggers_the_build_exactly_once_synchronously() {
    BUILD_COUNT.with(|c| c.set(0));

    let probe = LifecycleBuildSplitProbe::new();

    // `on_mount` already ran by the time `new()` returned — the build statements now live in
    // `__build_view`, but `on_constructed` still invokes it immediately, unchanged in timing.
    assert_eq!(BUILD_COUNT.with(|c| c.get()), 1);
    let _keep_alive: Rc<LifecycleBuildSplitProbe> = probe;
}

#[test]
fn each_new_instance_builds_exactly_once() {
    BUILD_COUNT.with(|c| c.set(0));

    let first = LifecycleBuildSplitProbe::new();
    assert_eq!(BUILD_COUNT.with(|c| c.get()), 1);

    let second = LifecycleBuildSplitProbe::new();
    assert_eq!(BUILD_COUNT.with(|c| c.get()), 2);

    drop(first);
    drop(second);
}

// CI-3 of #80 (docs/design/runtime/component_lifecycle_design.md §4a): `mount()` is a real,
// separately-callable, idempotent method now — `new()`'s own call into it is just its first (and,
// today, only) caller. `OnceCell::set` failing on a second call is the whole idempotency guard;
// this proves it deterministically panics rather than silently rebuilding/duplicating the view.
#[test]
#[should_panic(expected = "mount: component is already mounted")]
fn mounting_an_already_mounted_component_panics() {
    let probe = LifecycleBuildSplitProbe::new();
    let env = elwindui::core::environment::EnvironmentContext::root();
    probe.mount(env);
}
