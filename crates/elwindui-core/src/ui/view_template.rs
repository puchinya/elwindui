//! Generic deferred View factory, evaluated on demand rather than at declaration/mount time.
//!
//! [`DeferredViewFactory`] is the shared internal storage behind both
//! [`ControlTemplate`](crate::ui::ControlTemplate) (Control-appearance-specific, see
//! `docs/design/runtime/control_template_design.md`) and [`ViewTemplate`] (this module,
//! general-purpose). The two public types intentionally stay separate — see
//! `docs/design/runtime/view_template_design.md` for why they are not unified or aliased.

use super::*;
use crate::environment::EnvironmentContext;

/// Storage shared by every deferred-view factory type. Not exposed publicly: each public type
/// (`ControlTemplate<C>`, `ViewTemplate`) wraps this with its own typed `Context` and semantics,
/// and callers only ever interact with the wrapping type.
pub(crate) struct DeferredViewFactory<C> {
    factory: Rc<dyn Fn(C) -> Option<Rc<dyn UIElementExt>>>,
}

impl<C> DeferredViewFactory<C> {
    pub(crate) fn new(factory: impl Fn(C) -> Option<Rc<dyn UIElementExt>> + 'static) -> Self {
        Self {
            factory: Rc::new(factory),
        }
    }

    pub(crate) fn build(&self, context: C) -> Option<Rc<dyn UIElementExt>> {
        (self.factory)(context)
    }
}

impl<C> Clone for DeferredViewFactory<C> {
    fn clone(&self) -> Self {
        Self {
            factory: self.factory.clone(),
        }
    }
}

/// The context supplied to a [`ViewTemplate`] factory upon building its deferred view.
///
/// Unlike `ControlTemplateContext<C>`, this carries no target-type parameter and no
/// Control-specific semantics (no `templated_parent`, no `ContentPresenter` involvement) —
/// `ViewTemplate` is a general primitive for any deferred, independently-lifetimed View subtree:
/// today `context_popup`, and potentially future lazy tab content, dialogs, sheets, popovers.
#[derive(Clone)]
pub struct ViewBuildContext {
    /// The element that owns this deferred view. Retained only as `Weak` — a `ViewTemplate`
    /// factory must never keep its owner alive, since the owner strong-owns (directly or
    /// indirectly) the `ViewTemplate` value itself.
    pub owner: Weak<dyn UIElementExt>,
    /// The effective [`EnvironmentContext`] the deferred view should mount against.
    pub environment: EnvironmentContext,
}

/// A cloneable factory for a deferred, independently-lifetimed View subtree.
///
/// Evaluated on demand (e.g. when a popup opens), not at the owner's construction/mount time, and
/// may be evaluated again on each new demand — each evaluation produces an independent instance
/// with its own mount/unmount lifecycle. Building may fail (`build` returns `None`) once the
/// captured owner has already been dropped, i.e. `ViewBuildContext::owner` fails to upgrade.
#[derive(Clone)]
pub struct ViewTemplate {
    factory: DeferredViewFactory<ViewBuildContext>,
}

impl ViewTemplate {
    /// Creates a new view template from a factory closure.
    pub fn new(
        factory: impl Fn(ViewBuildContext) -> Option<Rc<dyn UIElementExt>> + 'static,
    ) -> Self {
        Self {
            factory: DeferredViewFactory::new(factory),
        }
    }

    /// Builds the view subtree using the provided context, or `None` if the owner is gone.
    pub fn build(&self, context: ViewBuildContext) -> Option<Rc<dyn UIElementExt>> {
        self.factory.build(context)
    }
}

impl<F> From<F> for ViewTemplate
where
    F: Fn(ViewBuildContext) -> Option<Rc<dyn UIElementExt>> + 'static,
{
    fn from(factory: F) -> Self {
        Self::new(factory)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn build_invokes_factory_with_context() {
        let calls = Rc::new(Cell::new(0));
        let calls_for_factory = calls.clone();
        let template = ViewTemplate::new(move |ctx: ViewBuildContext| {
            calls_for_factory.set(calls_for_factory.get() + 1);
            assert!(ctx.owner.upgrade().is_some());
            Some(crate::ui::TextBlock::new())
        });
        let owner: Rc<dyn UIElementExt> = crate::ui::TextBlock::new();
        let built = template.build(ViewBuildContext {
            owner: Rc::downgrade(&owner),
            environment: EnvironmentContext::root(),
        });
        assert!(built.is_some());
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn build_returns_none_when_owner_dropped() {
        let template = ViewTemplate::new(|ctx: ViewBuildContext| {
            ctx.owner.upgrade()?;
            Some(crate::ui::TextBlock::new())
        });
        let owner: Rc<dyn UIElementExt> = crate::ui::TextBlock::new();
        let weak_owner = Rc::downgrade(&owner);
        drop(owner);
        let built = template.build(ViewBuildContext {
            owner: weak_owner,
            environment: EnvironmentContext::root(),
        });
        assert!(built.is_none());
    }

    #[test]
    fn clone_keeps_a_capturing_factory() {
        let calls = Rc::new(Cell::new(0));
        let calls_for_factory = calls.clone();
        let template = ViewTemplate::new(move |_ctx| {
            calls_for_factory.set(calls_for_factory.get() + 1);
            Some(crate::ui::TextBlock::new())
        });
        let cloned = template.clone();
        let owner: Rc<dyn UIElementExt> = crate::ui::TextBlock::new();
        let _ = cloned.build(ViewBuildContext {
            owner: Rc::downgrade(&owner),
            environment: EnvironmentContext::root(),
        });
        assert_eq!(calls.get(), 1);
    }
}
