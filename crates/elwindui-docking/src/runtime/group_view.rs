//! CustomTabView realization for authored and generated groups.

use elwindui_custom_controls::{CustomTabView, CustomTabViewItem};
use std::rc::Rc;

pub(crate) fn replace_group_items(view: &CustomTabView, items: Vec<Rc<CustomTabViewItem>>) {
    view.replace_children(items);
}
