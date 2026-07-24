//! Regression test for a `RefCell` double-borrow that `#[elwindui::viewmodel]` action bodies used
//! to generate whenever a field's own new value referenced its old value through `self.field.
//! borrow()` — e.g. the "append to a log" idiom `log = format!("{}line\n", self.log.borrow());`
//! (this is exactly the pattern `examples/controls-demo`'s TextBox `on_key_down` handler used,
//! which crashed the whole app the instant Enter was pressed).
//!
//! `rewrite_action_body` (`elwindui-codegen/src/codegen.rs`) used to rewrite `field = expr;`
//! straight into `self.set_field(expr);` with `expr` spliced in unchanged. When `expr` itself reads
//! `self.field.borrow()`, the `Ref` temporary that produces isn't dropped until the *end of the
//! whole statement* (Rust drops temporaries at statement end, not when the sub-expression that
//! created them finishes evaluating) — so it was still alive when `set_field`'s own
//! `self.field.borrow_mut()` ran, panicking with `BorrowMutError` every time. That panic then
//! unwound across an `extern "C"` Objective-C callback boundary on the AppKit backend, which
//! aborts the process — matching the observed "app just crashes" behavior.

#[elwindui::viewmodel]
mod log_view_model {
    struct LogViewModel {
        #[observable(default = String::new())]
        log: String,
    }

    impl LogViewModel {
        fn append(&self) {
            log = format!("{}line\n", self.log.borrow());
        }
    }
}

#[test]
fn self_referential_log_update_does_not_double_borrow() {
    let vm = LogViewModel::new();
    vm.append();
    vm.append();
    assert_eq!(vm.log(), "line\nline\n");
}
