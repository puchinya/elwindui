//! Typed control-template values selected from an effective Environment during mount.

use super::*;
use crate::environment::{EnvironmentContext, EnvironmentKey};
use crate::reactive::Subscription;
use std::marker::PhantomData;

/// TypeId-scoped Environment slot for the default/override template of `C`.
///
/// The key is intentionally generic: templates for two distinct control types occupy distinct
/// Environment slots without requiring one public `EnvironmentKey` declaration per control.
#[doc(hidden)]
pub struct ControlTemplateEnvironment<C: ControlExt + 'static>(PhantomData<fn() -> C>);

impl<C: ControlExt + 'static> EnvironmentKey for ControlTemplateEnvironment<C> {
    type Value = Option<ControlTemplate<C>>;

    fn default_value() -> Self::Value {
        None
    }
}

/// Compile-time property bridge used by the `template_view!` expression frontend.
///
/// The bridge is deliberately keyed by a compile-time property token rather than a runtime
/// string.  Component code generation implements it by delegating to the component's existing
/// typed getter and `PropertyChanged` subscription surface; standalone template code can then be
/// generic over the expected `ControlTemplate<C>` target without introducing reflection or an
/// erased property map.
#[doc(hidden)]
pub trait TemplateProperty<const KEY: u64> {
    type Value: Clone + 'static;

    fn __template_get(&self) -> Self::Value;
    fn __template_subscribe(&self, listener: impl Fn() + 'static) -> Subscription;
}

/// Compile-time writable capability for a [`TemplateProperty`] bridge entry.
///
/// Code generation implements this trait only for a property that has a real typed setter.  A
/// template two-way binding or an explicit `set_<property>` call therefore fails during Rust trait
/// resolution for computed, read-only, derived, and otherwise non-settable properties instead of
/// reaching a runtime panic or a silent no-op.
///
/// ```compile_fail
/// use elwindui_core::reactive::Subscription;
/// use elwindui_core::ui::{TemplateProperty, WritableTemplateProperty};
///
/// struct ReadOnly;
///
/// impl TemplateProperty<7> for ReadOnly {
///     type Value = String;
///
///     fn __template_get(&self) -> Self::Value {
///         String::new()
///     }
///
///     fn __template_subscribe(&self, _listener: impl Fn() + 'static) -> Subscription {
///         Subscription::new(|| {})
///     }
/// }
///
/// fn write<T>(target: &T)
/// where
///     T: WritableTemplateProperty<7> + TemplateProperty<7, Value = String>,
/// {
///     <T as WritableTemplateProperty<7>>::__template_set(target, String::from("x"));
/// }
///
/// fn main() {
///     write(&ReadOnly);
/// }
/// ```
#[doc(hidden)]
#[diagnostic::on_unimplemented(
    message = "`{Self}` has no writable template-property capability for this key",
    note = "template two-way bindings and templated_parent.set_* require a #[prop] or #[state] property with a setter"
)]
pub trait WritableTemplateProperty<const KEY: u64>: TemplateProperty<KEY> {
    /// Writes the value through the generated component property's typed setter.
    #[doc(hidden)]
    fn __template_set(&self, value: Self::Value);
}

/// The context supplied to a [`ControlTemplate`] factory.
///
/// The control is strongly owned only for the duration chosen by the factory. Template
/// implementations that are subsequently owned by the control should retain a `Weak<C>` instead
/// of cloning this handle into the template root.
pub struct ControlTemplateContext<C: ControlExt + 'static> {
    /// The typed control whose Visual subtree is being built.
    pub control: Rc<C>,
    /// The effective mount-time Environment inherited by the template subtree.
    pub environment: EnvironmentContext,
}

/// A cloneable, typed factory for the Visual subtree of an ElwindUI-rendered [`Control`].
///
/// Selection is performed by generated component mount code. The value itself does not subscribe
/// to Environment changes and therefore does not implement runtime re-template.
///
/// Non-`Control` targets are rejected by the public type bound:
///
/// ```compile_fail
/// use elwindui_core::ui::ControlTemplate;
///
/// fn invalid(_: ControlTemplate<String>) {}
/// ```
pub struct ControlTemplate<C: ControlExt + 'static> {
    // Shares its boxed-closure storage with `ViewFactory` (see `crate::ui::view_factory`), but
    // `ControlTemplate`'s contract is that a factory always produces a root — building is
    // infallible from callers' perspective, so `__build` unwraps the shared `Option`-returning
    // storage rather than exposing it.
    factory: DeferredViewFactory<ControlTemplateContext<C>>,
}

impl<C: ControlExt + 'static> Clone for ControlTemplate<C> {
    fn clone(&self) -> Self {
        Self {
            factory: self.factory.clone(),
        }
    }
}

impl<C: ControlExt + 'static> ControlTemplate<C> {
    /// Creates a template from a capturing typed factory.
    pub fn new(
        factory: impl Fn(ControlTemplateContext<C>) -> Rc<dyn UIElementExt> + 'static,
    ) -> Self {
        Self {
            factory: DeferredViewFactory::new(move |context| Some(factory(context))),
        }
    }

    /// Creates a template whose visual tree does not read the typed templated parent.  Keeping
    /// the factory parameter independent of `C` lets Rust infer the target from an expected
    /// `ControlTemplate<C>` expression even when the template contains no parent property path.
    #[doc(hidden)]
    pub fn from_environment(
        factory: impl Fn(EnvironmentContext) -> Rc<dyn UIElementExt> + 'static,
    ) -> Self {
        Self::new(move |context| factory(context.environment))
    }

    /// Builds the template root once for generated mount code.
    #[doc(hidden)]
    pub fn __build(&self, context: ControlTemplateContext<C>) -> Rc<dyn UIElementExt> {
        self.factory
            .build(context)
            .expect("ControlTemplate factories always produce a root")
    }
}

impl EnvironmentContext {
    /// Installs a mount-time template override for the exact control type `C` at this context.
    ///
    /// `Some(template)` overrides the component default for subsequently mounted instances.
    /// `None` deliberately shadows an inherited value and selects the component default; it does
    /// not remove this context's Environment cell.
    pub fn set_control_template<C: ControlExt + 'static>(
        &self,
        template: Option<ControlTemplate<C>>,
    ) {
        self.set::<ControlTemplateEnvironment<C>>(template);
    }

    /// Reads the exact-type template override visible from this context.
    #[doc(hidden)]
    pub fn __control_template<C: ControlExt + 'static>(&self) -> Option<ControlTemplate<C>> {
        self.get::<ControlTemplateEnvironment<C>>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn clone_keeps_a_capturing_typed_factory() {
        let calls = Rc::new(Cell::new(0));
        let calls_for_factory = calls.clone();
        let template = ControlTemplate::<Control>::new(move |context| {
            calls_for_factory.set(calls_for_factory.get() + 1);
            assert_eq!(context.environment.get::<ProbeKey>(), 42);
            TextBlock::new()
        });
        let cloned = template.clone();
        let environment = EnvironmentContext::root();
        environment.set::<ProbeKey>(42);
        let control = Control::new();

        let _ = cloned.__build(ControlTemplateContext {
            control,
            environment,
        });

        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn environment_template_slots_are_typed_and_shadowable() {
        let root = EnvironmentContext::root();
        let child = root.derive();
        let sibling = root.derive();

        let control_template = ControlTemplate::<Control>::new(|_context| TextBlock::new());
        root.set_control_template::<Control>(Some(control_template));

        assert!(child.__control_template::<Control>().is_some());
        assert!(sibling.__control_template::<Control>().is_some());

        child.set_control_template::<Control>(None);
        assert!(child.__control_template::<Control>().is_none());
        assert!(root.__control_template::<Control>().is_some());
        assert!(sibling.__control_template::<Control>().is_some());

        let content_template = ControlTemplate::<ContentControl>::new(|_context| TextBlock::new());
        root.set_control_template::<ContentControl>(Some(content_template));
        assert!(root.__control_template::<ContentControl>().is_some());
        assert!(root.__control_template::<Control>().is_some());
    }

    struct ProbeKey;

    impl crate::environment::EnvironmentKey for ProbeKey {
        type Value = i32;

        fn default_value() -> Self::Value {
            0
        }
    }
}
