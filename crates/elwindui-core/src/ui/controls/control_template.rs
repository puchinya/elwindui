//! Typed control-template values selected from an effective Environment during mount.

use super::*;
use crate::environment::EnvironmentContext;

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
    // Shares its boxed-closure storage with `ViewTemplate` (see `crate::ui::view_template`), but
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

    /// Builds the template root once for generated mount code.
    #[doc(hidden)]
    pub fn __build(&self, context: ControlTemplateContext<C>) -> Rc<dyn UIElementExt> {
        self.factory
            .build(context)
            .expect("ControlTemplate factories always produce a root")
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

    struct ProbeKey;

    impl crate::environment::EnvironmentKey for ProbeKey {
        type Value = i32;

        fn default_value() -> Self::Value {
            0
        }
    }
}
