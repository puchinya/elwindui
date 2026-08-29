//! Generic deferred View factory, evaluated on demand rather than at declaration/mount time.
//!
//! [`DeferredViewFactory`] is the shared internal storage behind both
//! [`ControlTemplate`](crate::ui::ControlTemplate) (Control-appearance-specific, see
//! `docs/design/runtime/control_template_design.md`) and [`ViewFactory`] (this module,
//! general-purpose). The two public types intentionally stay separate — see
//! `docs/design/runtime/view_factory_design.md` for why they are not unified or aliased.

use super::*;
use crate::environment::EnvironmentContext;

/// Storage shared by every deferred-view factory type. Not exposed publicly: each public type
/// (`ControlTemplate<C>`, `ViewFactory`) wraps this with its own typed `Context` and semantics,
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

/// The context supplied to a [`ViewFactory`] factory upon building its deferred view.
///
/// Unlike `ControlTemplateContext<C>`, this carries no target-type parameter and no
/// Control-specific semantics (no `templated_parent`, no `ContentPresenter` involvement) —
/// `ViewFactory` is a general primitive for any deferred, independently-lifetimed View subtree:
/// today `context_popup`, and potentially future lazy tab content, dialogs, sheets, popovers.
#[derive(Clone)]
pub struct ViewBuildContext {
    /// The element that owns this deferred view. Retained only as `Weak` — a `ViewFactory`
    /// factory must never keep its owner alive, since the owner strong-owns (directly or
    /// indirectly) the `ViewFactory` value itself.
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
pub struct ViewFactory {
    factory: DeferredViewFactory<ViewBuildContext>,
}

impl ViewFactory {
    /// Creates a new view template from a factory closure.
    pub fn new(
        factory: impl Fn(ViewBuildContext) -> Option<Rc<dyn UIElementExt>> + 'static,
    ) -> Self {
        Self {
            factory: DeferredViewFactory::new(factory),
        }
    }

    /// Builds the view subtree using the provided context, or `None` if the owner is gone.
    ///
    /// Owner liveness is enforced here, mechanically, before the factory closure ever runs — not
    /// left to the closure's own discretion. A factory that never itself checks `ctx.owner` (e.g.
    /// `ViewFactory::new(|_ctx| Some(TextBlock::new()))`) still cannot build once the owner has been
    /// dropped, so "a deferred View cannot be built after its owner is gone" is a real invariant of
    /// this type, not just documented intent. The factory may call `context.owner.upgrade()` again
    /// itself if it needs the concrete `Rc` — this only re-checks liveness, it doesn't consume or
    /// strengthen `context.owner`.
    pub fn build(&self, context: ViewBuildContext) -> Option<Rc<dyn UIElementExt>> {
        context.owner.upgrade()?;
        self.factory.build(context)
    }
}

impl<F> From<F> for ViewFactory
where
    F: Fn(ViewBuildContext) -> Option<Rc<dyn UIElementExt>> + 'static,
{
    fn from(factory: F) -> Self {
        Self::new(factory)
    }
}

/// PR #165 review remediation, A4 (round 2: `from_view_factory` replaces the assertion-only
/// original): a sealed marker implemented only for `ViewFactory` and `Option<ViewFactory>` —
/// the only two types a `context_popup: view! { .. }` (or any other `ViewExpr::DeferredView`) may
/// ever be assigned to (`docs/specs/dsl_spec.md` rule 37) — that also *converts* a freshly-built
/// `ViewFactory` factory into whichever of the two shapes the target property actually declares.
///
/// `elwindui-codegen`'s own `validate::check_deferred_view_assignment` already rejects a
/// mismatched target *when* the target component has a local `TypeInfo` (a same-crate
/// `#[elwindui::component]`) — but a real builtin (`TextBlock`, `Window`, every hand-written
/// `#[class]`-declared type in `elwindui-core`/a backend crate) never has one, so that check
/// silently no-ops for the actual production path (`emit_external_attribute_sets`). This trait is
/// the other half: `elwindui-codegen` emits a bound against it, generic over
/// `__elwindui_props_{Type}!(@field_type {field})` (the same cross-crate field-type transport
/// `synthesize_external_base_fields`/`resolve_effective_fields` already use, Refs #90) — so a
/// mismatched real builtin target fails *at the consumer crate's own compile time*, with this
/// trait's `#[diagnostic::on_unimplemented]` message naming the field and the required type,
/// exactly like the local-`TypeInfo` diagnostic already does. See
/// `docs/design/tools/codegen_design.md` §3.35 and `docs/specs/dsl_spec.md` rule 37.
///
/// The round-1 version of this trait was assertion-only (`__assert_deferred_view_assignment_target`)
/// — it type-checked the target but `emit_external_attribute_sets` still unconditionally wrapped
/// the built factory in `Some(..)` regardless of which of the two accepted shapes the property
/// actually declared, so a real builtin property declared bare `ViewFactory` (not
/// `Option<ViewFactory>`) would pass this assertion and then fail immediately afterward with a
/// type mismatch on the generated setter call — only accidentally correct for `context_popup`
/// (the only production `Option<ViewFactory>` consumer today). `from_view_factory` fixes this by
/// performing the actual shape conversion generically, so the external emission path works for
/// either accepted shape identically to the local-`TypeInfo` path, which already branches on
/// `is_option`.
#[doc(hidden)]
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a valid `context_popup`/deferred-view target type",
    label = "a deferred view (`view! {{ .. }}`) can only be assigned to a `ViewFactory` or `Option<ViewFactory>` property, not `{Self}`",
    note = "rewrite the target property's declared type to `ViewFactory` or `Option<ViewFactory>`, or assign an ordinary value instead of `view! {{ .. }}`"
)]
pub trait DeferredViewAssignmentTarget: private::Sealed + Sized {
    #[doc(hidden)]
    fn from_view_factory(value: ViewFactory) -> Self;
}

mod private {
    pub trait Sealed {}
    impl Sealed for super::ViewFactory {}
    impl Sealed for Option<super::ViewFactory> {}
}

impl DeferredViewAssignmentTarget for ViewFactory {
    fn from_view_factory(value: ViewFactory) -> Self {
        value
    }
}

impl DeferredViewAssignmentTarget for Option<ViewFactory> {
    fn from_view_factory(value: ViewFactory) -> Self {
        Some(value)
    }
}

/// Generated-code-only entry point (`elwindui-codegen`'s `emit_external_attribute_sets`): converts
/// a freshly-built `ViewFactory` factory into whichever of `ViewFactory`/`Option<ViewFactory>`
/// the real builtin's own `@field_type` transport reports as `T`, failing to compile (via
/// `DeferredViewAssignmentTarget`'s own `#[diagnostic::on_unimplemented]`) for any other `T`.
#[doc(hidden)]
pub fn __coerce_deferred_view_assignment_target<T: DeferredViewAssignmentTarget>(
    value: ViewFactory,
) -> T {
    T::from_view_factory(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn build_invokes_factory_with_context() {
        let calls = Rc::new(Cell::new(0));
        let calls_for_factory = calls.clone();
        let template = ViewFactory::new(move |ctx: ViewBuildContext| {
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
    fn build_returns_none_when_owner_dropped_and_never_invokes_the_factory() {
        // Stronger than "the factory itself checks ctx.owner and declines": `ViewFactory::build`
        // enforces owner liveness mechanically, before the factory closure runs at all, so even a
        // factory that never checks `ctx.owner` (like this one) cannot build once the owner is gone.
        let calls = Rc::new(Cell::new(0));
        let calls_for_factory = calls.clone();
        let template = ViewFactory::new(move |_ctx: ViewBuildContext| {
            calls_for_factory.set(calls_for_factory.get() + 1);
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
        assert_eq!(
            calls.get(),
            0,
            "factory must not run once the owner is already gone"
        );
    }

    #[test]
    fn clone_keeps_a_capturing_factory() {
        let calls = Rc::new(Cell::new(0));
        let calls_for_factory = calls.clone();
        let template = ViewFactory::new(move |_ctx| {
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

    /// A4-T1: `__coerce_deferred_view_assignment_target::<ViewFactory>` returns the same
    /// logical template value unwrapped — the bare, non-`Option` accepted shape.
    #[test]
    fn coerce_deferred_view_assignment_target_bare_view_factory() {
        let calls = Rc::new(Cell::new(0));
        let calls_for_factory = calls.clone();
        let template = ViewFactory::new(move |_ctx| {
            calls_for_factory.set(calls_for_factory.get() + 1);
            Some(crate::ui::TextBlock::new())
        });
        let value: ViewFactory = __coerce_deferred_view_assignment_target(template);
        let owner: Rc<dyn UIElementExt> = crate::ui::TextBlock::new();
        let built = value.build(ViewBuildContext {
            owner: Rc::downgrade(&owner),
            environment: EnvironmentContext::root(),
        });
        assert!(built.is_some());
        assert_eq!(calls.get(), 1);
    }

    /// A4-T2: `__coerce_deferred_view_assignment_target::<Option<ViewFactory>>` wraps the
    /// template in `Some` — the optional accepted shape (`context_popup`'s own real declared
    /// type today).
    #[test]
    fn coerce_deferred_view_assignment_target_optional_view_factory() {
        let template = ViewFactory::new(|_ctx| Some(crate::ui::TextBlock::new()));
        let value: Option<ViewFactory> = __coerce_deferred_view_assignment_target(template);
        assert!(value.is_some());
    }
}
