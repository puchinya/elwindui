//! Proc-macro regression coverage for Issue #65: a direct observable field of a stable `for`
//! item must compile as a typed TwoWay binding.
//!
//! The generated item callback owns only the item `Rc`; the item's model-to-widget observer is
//! owned by the corresponding `DynamicChild`. The function below is intentionally type-checked but
//! not executed: AppKit requires native view construction on the process main thread, while Rust's
//! test harness invokes test functions from worker threads. `elwindui-core` separately tests the
//! `DynamicChildSlot` subscription Drop path.

#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]

use std::rc::Rc;

#[elwindui::viewmodel]
mod row_view_model {
    struct RowViewModel {
        #[observable(default = String::new())]
        content: String,
    }
}

#[elwindui::viewmodel]
mod rows_view_model {
    use super::RowViewModel;

    struct RowsViewModel {
        #[observable(default = Vec::new())]
        rows: Vec<RowViewModel>,
    }
}

#[elwindui::component(inherits ContentControl)]
struct ForItemTwoWayHost {
    #[bindable]
    vm: Rc<RowsViewModel>,

    template: template_view! {
        VerticalLayout {
            for row in vm.rows {
                TextArea { text <=> row.content }
            }
        }
    },
}

#[elwindui::component]
impl ForItemTwoWayHost {}

fn type_checked_construction_and_drop(rows: Rc<RowsViewModel>) {
    let host = ForItemTwoWayHost::new(Rc::clone(&rows));
    drop(host);
}

#[test]
fn for_item_two_way_generated_host_type_checks() {
    let _ = type_checked_construction_and_drop as fn(Rc<RowsViewModel>);
}
