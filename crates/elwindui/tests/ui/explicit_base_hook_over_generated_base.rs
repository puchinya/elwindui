// Issue #166 (PR #311 A/P1-2): a generated base embedded in a derived Component runs its own
// `on_mount`/`on_unmount` automatically (mount from the base up, unmount from the most-derived
// level down). An explicit `base::on_mount()`/`base::on_unmount()` would run the base hook a
// second time, so it is rejected at compile time.
#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]
// trybuild compiles fixtures outside the workspace lint table, which is where the generated
// cfg(rust_analyzer) gate is registered for check-cfg; keep the snapshot to the diagnostic under test.
#![allow(unexpected_cfgs)]

#[elwindui::component(inherits VerticalLayout)]
struct HookBase {
    body: view! {
        on_mount {
            println!("base mount");
        }
        on_unmount {
            println!("base unmount");
        }
        TextBlock { text: "base" }
    },
}

#[elwindui::component]
impl HookBase {}

#[elwindui::component(inherits crate::HookBase)]
struct HookDerived {
    body: view! {
        on_mount {
            base::on_mount();
        }
        on_unmount {
            base::on_unmount();
        }
        TextBlock { text: "derived" }
    },
}

#[elwindui::component]
impl HookDerived {}

fn main() {}
