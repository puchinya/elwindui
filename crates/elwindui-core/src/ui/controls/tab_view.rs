//! `builtin::TabView` — multi-document tab container.

use super::*;

/// `TabView`'s class trait (docs/design/gui_framework_design.md §5.1). Its content is a live, ordered
/// collection of `TabViewItem`s. Dynamic child ranges update this collection directly; the
/// backend reconciles the corresponding native tabs.
#[elwindui_macros::class(trait_only, inherits = crate::ui::NativeControl, sealed)]
#[content(children)]
#[prop(children: Vec<std::rc::Rc<dyn crate::ui::TabViewItemExt>>)]
#[prop(two_way, selected_index: usize)]
#[prop(on_select: fn(usize))]
#[prop(on_new_tab: fn())]
pub trait TabView {
    fn children(&self) -> &dyn ListExt<dyn TabViewItemExt>;
}
