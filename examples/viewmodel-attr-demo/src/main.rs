//! Minimal end-to-end proof that `#[elwindui::viewmodel]` (a real Rust attribute macro over a
//! `mod { struct ... impl ... }`, see `elwindui_codegen::attr_frontend`) generates a working
//! viewmodel, without going through the text frontend (`parser.rs`) at all. Deliberately kept small
//! and separate from `examples/notepad`: this crate only exercises the new frontend, not the view
//! layer or any backend.
//!
//! Also covers `#[elwindui::component] impl Name { .. }` — §3's `#[overridable]`/`#[overrides]`
//! method inheritance — for the same reason: it is the only place the generated `__base_<name>`
//! shadow and the `base::<name>(..)` rewrite are actually compiled and run, rather than just
//! asserted on as token text in `elwindui-codegen`'s own unit tests.

// Required in the crate root of anything using `#[elwindui_macros::class]` (which every
// `inherits`-carrying component becomes) — see docs/specs/macro_class_spec.md §10.
#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

#[elwindui::viewmodel]
mod counter_vm {
    pub struct Counter {
        #[observable(default = 0i32)]
        count: i32,

        #[computed(expr = count * 2)]
        doubled: i32,

        #[computed(expr = count < 10)]
        increment_can_execute: bool,
    }

    impl Counter {
        fn increment(&self) {
            count = count + 1;
        }
    }
}

// `inherits ContentControl` rather than a bare component: a component with no `inherits` isn't
// declared as an `#[elwindui_macros::class]` at all, so it can't carry `#[overridable]` methods.
#[elwindui::component(inherits ContentControl)]
struct LabeledPanel {
    body: view! {
        TextBlock { text: "panel" }
    },
}

#[elwindui::component]
impl LabeledPanel {
    #[overridable]
    fn label(&self) -> String {
        "panel".to_string()
    }
}


fn main() {
    elwindui::init().expect("initialize elwindui");

    let panel = LabeledPanel::new();
    assert_eq!(panel.label(), "panel");
    println!("ok: label={}", panel.label());

    let c = Counter::new();
    assert_eq!(c.count(), 0);
    assert_eq!(c.doubled(), 0);
    assert!(c.increment_can_execute());

    c.increment();
    assert_eq!(c.count(), 1);
    assert_eq!(c.doubled(), 2);

    println!(
        "ok: count={} doubled={} can_execute={}",
        c.count(),
        c.doubled(),
        c.increment_can_execute()
    );
}
