//! Minimal end-to-end proof that `#[elwindui::viewmodel]` (a real Rust attribute macro over a
//! `mod { struct ... impl ... }`, see `elwindui_codegen::attr_frontend`) generates a working
//! viewmodel, without going through the `.elwind` DSL/`parser.rs` at all. Deliberately kept small
//! and separate from `examples/notepad`: this crate only exercises the new frontend, not the view
//! layer or any backend.

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

fn main() {
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
