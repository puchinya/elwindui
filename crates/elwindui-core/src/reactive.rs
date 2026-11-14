/// Owns a registration with a generated `PropertyChanged` event.
///
/// Generated views keep these handles for as long as they display a value.  Dropping the handle
/// removes the callback, which keeps a long-lived viewmodel from accumulating dead observers when
/// a dynamic view region is replaced.
pub struct Subscription {
    cancel: Option<Box<dyn FnOnce()>>,
}

impl Subscription {
    pub fn new(cancel: impl FnOnce() + 'static) -> Self {
        Self {
            cancel: Some(Box::new(cancel)),
        }
    }

    /// Explicitly removes the callback before this handle is dropped.
    pub fn cancel(mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel();
        }
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel();
        }
    }
}

/// Common interface for a type with a generated `PropertyChanged` event (currently only
/// `viewmodel`, docs/design/runtime/state_management_design.md) — identifies the changed property by name (`&'static str`) rather
/// than a per-viewmodel-generated enum. This exists specifically for `#[bindable]` fields
/// (docs/design/runtime/state_management_design.md): a `component` written via `#[elwindui::component]` + `body: view! { .. }` (or a
/// DSL `view` referencing a `viewmodel` declared elsewhere) is parsed by a *separate* macro
/// invocation / file from whatever declares the concrete viewmodel type, so it never has a name
/// for that type's own generated `XProperty` enum to write a match arm against. A property *name*
/// needs no such type knowledge — `#[component]`'s own codegen already knows every `vm.<field>`
/// path referenced in its view body (that's what it parsed), so it can match on the name directly.
///
/// This is the Rust-without-KeyPaths answer to "which property changed": a literal KeyPath
/// (`fn(&T) -> V` or similar) would still require naming the concrete type `T`, which is exactly
/// the cross-macro-invocation knowledge that isn't available here — so identity-by-name is used
/// instead of identity-by-typed-accessor.
///
/// Every call site is against a statically-known concrete type (never `dyn ObservableExt`), so
/// this does not reintroduce the dynamic dispatch that `docs/design/runtime/state_management_design.md`
/// deliberately avoids — it's a marker/protocol trait, not a type-erasure mechanism.
pub trait ObservableExt {
    fn subscribe_property_changed(&self, f: impl Fn(&'static str) + 'static) -> Subscription;
}

/// Autoref dispatch for a `for` item's generated observer: `for` accepts any `Rc<T>`, so whether
/// `T` publishes changes is only known once the item type is, not when codegen runs.
/// `(&ItemObservation(item)).observe_item(..)` selects `ObservableItem` for an `ObservableExt` item
/// and otherwise autorefs once more into `PlainItem`, which never subscribes.
#[doc(hidden)]
pub struct ItemObservation<'a, T: ?Sized>(pub &'a T);

#[doc(hidden)]
pub trait ObservableItem {
    fn observe_item(&self, f: impl Fn(&'static str) + 'static) -> Option<Subscription>;
}

impl<T: ObservableExt + ?Sized> ObservableItem for ItemObservation<'_, T> {
    fn observe_item(&self, f: impl Fn(&'static str) + 'static) -> Option<Subscription> {
        Some(self.0.subscribe_property_changed(f))
    }
}

#[doc(hidden)]
pub trait PlainItem {
    fn observe_item(&self, _f: impl Fn(&'static str) + 'static) -> Option<Subscription> {
        None
    }
}

impl<T: ?Sized> PlainItem for &ItemObservation<'_, T> {}

/// Issue #166: a generated Component embedded as another Component's `base` value has no `Rc` of
/// its own type, so it cannot install its own property-change subscription. The outermost
/// Component (which does observe its `base` by name) relays every notification down the chain
/// through this hook, and each embedded level re-syncs its own view bindings and dynamic regions.
#[doc(hidden)]
pub trait EmbeddedViewSync {
    fn __elwindui_embedded_property_changed(&self, property: &'static str);

    /// Mounts this level as the embedding level's `base`. An embedded generated Component does not
    /// mount itself from its own `on_constructed` (it has no `Rc` of its own type yet); the
    /// embedding level calls this from its own `mount`, first recording `handle` — the weak handle
    /// through which this level reaches itself from the outermost Component's `Rc` — so every
    /// callback this level builds while mounting (dynamic-region item wiring, lazily materialized
    /// branch wiring) can address it without a strong reference to the owner. A level that is
    /// already mounted is left as is.
    fn __elwindui_mount_as_base(
        &self,
        handle: Option<WeakEmbeddedHandle<Self>>,
        environment: crate::environment::EnvironmentContext,
    ) where
        Self: Sized;

    /// Installs this embedded level's own view wiring (event handlers, two-way bindings) and its
    /// unmount hooks through `this`, a handle to it projected out of the outermost Component's
    /// `Rc`, then continues with its own embedded base. Called after the embedding level's own
    /// wiring and unmount hooks, so unmount runs from the most-derived level down to the base.
    fn __elwindui_wire_embedded(&self, this: EmbeddedHandle<Self>)
    where
        Self: Sized;

    /// Installs this embedded level's own subscriptions (bindable owners, Environment, semantic
    /// brushes, `on_update`) and runs its `on_mount`, after its own embedded base did the same, so
    /// `on_mount` runs from the base up to the most-derived level.
    fn __elwindui_mount_embedded(&self, this: EmbeddedHandle<Self>)
    where
        Self: Sized;

    /// How many children this level and every level below it currently place at the front of the
    /// shared root content collection. A further-derived level's root-direct dynamic regions
    /// start after this prefix.
    fn __elwindui_root_content_span(&self) -> usize;
}

/// A callback owner captured by generated view wiring. Event handlers, two-way bindings and
/// subscriptions are stored on widgets the owner itself keeps alive, so they capture only the
/// weak form and upgrade it when they run (issue #166: no `owner -> child -> handler -> owner`
/// cycle). Implemented for `Rc<T>` (a standalone Component) and [`EmbeddedHandle`] (a Component
/// embedded as another one's `base`).
#[doc(hidden)]
pub trait WiringOwner {
    type Weak: WiringWeak<Strong = Self> + 'static;
    fn downgrade_wiring(&self) -> Self::Weak;
}

#[doc(hidden)]
pub trait WiringWeak {
    type Strong;
    fn upgrade_wiring(&self) -> Option<Self::Strong>;
}

impl<T: ?Sized + 'static> WiringOwner for std::rc::Rc<T> {
    type Weak = std::rc::Weak<T>;
    fn downgrade_wiring(&self) -> Self::Weak {
        std::rc::Rc::downgrade(self)
    }
}

impl<T: ?Sized + 'static> WiringWeak for std::rc::Weak<T> {
    type Strong = std::rc::Rc<T>;
    fn upgrade_wiring(&self) -> Option<Self::Strong> {
        self.upgrade()
    }
}

impl<T: ?Sized + 'static> WiringOwner for EmbeddedHandle<T> {
    type Weak = WeakEmbeddedHandle<T>;
    fn downgrade_wiring(&self) -> Self::Weak {
        WeakEmbeddedHandle {
            owner: std::rc::Rc::downgrade(&self.owner),
            project: std::rc::Rc::clone(&self.project),
        }
    }
}

/// The weak form of [`EmbeddedHandle`]: it does not keep the outermost owner alive.
#[doc(hidden)]
pub struct WeakEmbeddedHandle<T: ?Sized + 'static> {
    owner: std::rc::Weak<dyn std::any::Any>,
    project: std::rc::Rc<dyn Fn(&dyn std::any::Any) -> &T>,
}

impl<T: ?Sized + 'static> Clone for WeakEmbeddedHandle<T> {
    fn clone(&self) -> Self {
        Self {
            owner: self.owner.clone(),
            project: std::rc::Rc::clone(&self.project),
        }
    }
}

impl<T: ?Sized + 'static> WiringWeak for WeakEmbeddedHandle<T> {
    type Strong = EmbeddedHandle<T>;
    fn upgrade_wiring(&self) -> Option<Self::Strong> {
        Some(EmbeddedHandle {
            owner: self.owner.upgrade()?,
            project: std::rc::Rc::clone(&self.project),
        })
    }
}

/// The declared type of one required constructor input (`#[param]` / `#[bindable]` without a
/// default) of a generated Component, keyed by a stable hash of the field name
/// (`elwindui_codegen`'s `required_param_key`). A derived Component in another crate cannot name
/// the base's field types (they may use imports only visible in the defining module), so its
/// auto-forwarded constructor parameters are typed `<Base as RequiredParam<KEY>>::Ty` instead,
/// resolved where the base is defined. Generated code only; not part of the authoring API.
#[doc(hidden)]
pub trait RequiredParam<const KEY: u64> {
    /// The field's declared type.
    type Ty;
}

/// A strong, cloneable handle to a value embedded inside an `Rc`-owned outer value: the owner
/// `Rc` plus a projection to the embedded part. It dereferences to the embedded value, so
/// generated wiring written against `this: Rc<Self>` (method calls, `&*this`, [`WiringOwner`])
/// works unchanged for an embedded `base`. Generated code only holds it for the duration of a
/// call; stored callbacks hold its [`WeakEmbeddedHandle`] form.
#[doc(hidden)]
pub struct EmbeddedHandle<T: ?Sized + 'static> {
    owner: std::rc::Rc<dyn std::any::Any>,
    project: std::rc::Rc<dyn Fn(&dyn std::any::Any) -> &T>,
}

impl<T: 'static> EmbeddedHandle<T> {
    pub fn new(owner: std::rc::Rc<T>) -> Self {
        Self {
            owner,
            project: std::rc::Rc::new(|owner: &dyn std::any::Any| {
                owner
                    .downcast_ref::<T>()
                    .expect("EmbeddedHandle: owner type is fixed at construction")
            }),
        }
    }
}

impl<T: ?Sized + 'static> EmbeddedHandle<T> {
    pub fn map<U: ?Sized + 'static>(
        &self,
        f: impl for<'a> Fn(&'a T) -> &'a U + 'static,
    ) -> EmbeddedHandle<U> {
        let project = std::rc::Rc::clone(&self.project);
        EmbeddedHandle {
            owner: std::rc::Rc::clone(&self.owner),
            project: std::rc::Rc::new(move |owner: &dyn std::any::Any| f(project(owner))),
        }
    }
}

impl<T: ?Sized + 'static> Clone for EmbeddedHandle<T> {
    fn clone(&self) -> Self {
        Self {
            owner: std::rc::Rc::clone(&self.owner),
            project: std::rc::Rc::clone(&self.project),
        }
    }
}

/// Mirrors `Rc<T>: AsRef<T>`, which generated code also uses on `this`.
impl<T: ?Sized + 'static> AsRef<T> for EmbeddedHandle<T> {
    fn as_ref(&self) -> &T {
        self
    }
}

impl<T: ?Sized + 'static> std::ops::Deref for EmbeddedHandle<T> {
    type Target = T;
    fn deref(&self) -> &T {
        (self.project)(&*self.owner)
    }
}

/// Autoref dispatch for [`EmbeddedViewSync`], mirroring [`ItemObservation`]:
/// `(&EmbeddedNotify(base)).notify_embedded(..)` relays to a generated base and is a no-op for a
/// hand-written/builtin base.
#[doc(hidden)]
pub struct EmbeddedNotify<'a, T: ?Sized>(pub &'a T);

#[doc(hidden)]
pub trait EmbeddedNotifyForward<T: ?Sized + 'static> {
    fn notify_embedded(&self, property: &'static str);
    fn mount_as_base(
        &self,
        handle: Option<WeakEmbeddedHandle<T>>,
        environment: crate::environment::EnvironmentContext,
    );
    fn wire_embedded(&self, this: EmbeddedHandle<T>);
    fn mount_embedded(&self, this: EmbeddedHandle<T>);
    fn embedded_span(&self) -> usize;
}

impl<T: EmbeddedViewSync + 'static> EmbeddedNotifyForward<T> for EmbeddedNotify<'_, T> {
    fn notify_embedded(&self, property: &'static str) {
        self.0.__elwindui_embedded_property_changed(property);
    }
    fn mount_as_base(
        &self,
        handle: Option<WeakEmbeddedHandle<T>>,
        environment: crate::environment::EnvironmentContext,
    ) {
        self.0.__elwindui_mount_as_base(handle, environment);
    }
    fn wire_embedded(&self, this: EmbeddedHandle<T>) {
        self.0.__elwindui_wire_embedded(this);
    }
    fn mount_embedded(&self, this: EmbeddedHandle<T>) {
        self.0.__elwindui_mount_embedded(this);
    }
    fn embedded_span(&self) -> usize {
        self.0.__elwindui_root_content_span()
    }
}

#[doc(hidden)]
pub trait EmbeddedNotifyIgnore<T: ?Sized + 'static> {
    fn notify_embedded(&self, _property: &'static str) {}
    fn mount_as_base(
        &self,
        _handle: Option<WeakEmbeddedHandle<T>>,
        _environment: crate::environment::EnvironmentContext,
    ) {
    }
    fn wire_embedded(&self, _this: EmbeddedHandle<T>) {}
    fn mount_embedded(&self, _this: EmbeddedHandle<T>) {}
    fn embedded_span(&self) -> usize {
        0
    }
}

impl<T: ?Sized + 'static> EmbeddedNotifyIgnore<T> for &EmbeddedNotify<'_, T> {}

/// The value of a `#[async_computed]` field — what its generated getter returns instead of the
/// bare declared type `T`. See docs/design/runtime/state_management_design.md "Async work",
/// docs/specs/dsl_spec.md §3 "`viewmodel`と`store`:宣言構文".
///
/// `Failed(String)` rather than a generic `AsyncComputed<T, E>`: matches this codebase's existing
/// stringly-typed-error idiom at API boundaries (e.g. `elwindui::InitError`, `validate::validate`'s
/// `Vec<String>`), and avoids forcing a second type parameter into every `#[async_computed]`
/// field's inference. Generated codegen builds this by `.to_string()`-ing whatever `E: Display`
/// the field's `expr` produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AsyncComputed<T> {
    /// No recompute has completed yet — the field's initial state before its first spawn resolves,
    /// or (transiently) while a supersede is in flight if the caller re-reads before it resolves.
    Loading,
    /// The most recent non-superseded recompute completed successfully.
    Ready(T),
    /// The most recent non-superseded recompute completed with an error, stringified via
    /// `E: Display`.
    Failed(String),
}

impl<T> Default for AsyncComputed<T> {
    fn default() -> Self {
        AsyncComputed::Loading
    }
}

#[cfg(test)]
mod tests {
    use super::Subscription;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    #[test]
    fn subscription_cancels_once_on_drop() {
        let calls = Rc::new(Cell::new(0));
        {
            let calls_for_cancel = calls.clone();
            let _subscription =
                Subscription::new(move || calls_for_cancel.set(calls_for_cancel.get() + 1));
            assert_eq!(calls.get(), 0);
        }
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn explicit_cancel_does_not_run_twice() {
        let calls = Rc::new(Cell::new(0));
        let calls_for_cancel = calls.clone();
        Subscription::new(move || calls_for_cancel.set(calls_for_cancel.get() + 1)).cancel();
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn repeated_generated_style_subscriptions_remove_registry_entries() {
        type HandlerEntry = (Rc<Cell<bool>>, Rc<dyn Fn()>);

        let handlers = Rc::new(RefCell::new(Vec::<HandlerEntry>::new()));
        for _ in 0..64 {
            let active = Rc::new(Cell::new(true));
            handlers.borrow_mut().push((active.clone(), Rc::new(|| {})));
            let weak_handlers = Rc::downgrade(&handlers);
            let subscription = Subscription::new(move || {
                active.set(false);
                if let Some(handlers) = weak_handlers.upgrade() {
                    handlers
                        .borrow_mut()
                        .retain(|(registered, _)| !Rc::ptr_eq(registered, &active));
                }
            });

            assert_eq!(handlers.borrow().len(), 1);
            drop(subscription);
            assert!(handlers.borrow().is_empty());
        }
        assert_eq!(Rc::strong_count(&handlers), 1);
    }
}
