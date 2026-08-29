// A ContentControl-derived component keeps caller content in the inherited logical content slot
// and presents it through an explicit default ControlTemplate.  The template is independent of
// the content supplied at each call site.
#[elwindui::component(inherits ContentControl)]
struct ContentWrapper {
    template: template_view!(|templated_parent: Self| { ContentPresenter {} }),
}

#[elwindui::component]
impl ContentWrapper {}
