/// Slotmap-style handle (index + generation) into a [`ReactiveGraph`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignalId {
    index: u32,
    generation: u32,
}

/// Fallback only (付録O.5): ordinary `#[computed]`/`#[async_computed]` fields have their
/// dependency graph extracted statically by `elwindui-codegen`, which generates direct call
/// chains instead of going through this graph. This exists only for dependency paths that can't
/// be resolved at compile time. Modeled loosely on WinUI3's `DependencyProperty` invalidation
/// (a value change invalidates dependents; they're lazily recomputed on next read) but kept
/// deliberately minimal given how rarely it should be reached.
pub struct ReactiveGraph {
    _private: (),
}

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
/// `viewmodel`, 付録O.2/O.5) — identifies the changed property by name (`&'static str`) rather
/// than a per-viewmodel-generated enum. This exists specifically for `#[bindable]` fields
/// (付録O.8): a `component` written via `#[elwindui::component]` + `body: view! { .. }` (or a
/// `.elwind` `view` referencing a `viewmodel` declared elsewhere) is parsed by a *separate* macro
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
/// this does not reintroduce the dynamic dispatch 付録O.5 deliberately avoids — it's a marker/
/// protocol trait, not a type-erasure mechanism.
pub trait ObservableExt {
    fn subscribe_property_changed(&self, f: impl Fn(&'static str) + 'static) -> Subscription;
}

impl ReactiveGraph {
    pub fn create<T: 'static>(&mut self, _initial: T) -> SignalId {
        todo!("elwindui-core: fallback reactive graph not yet implemented")
    }

    pub fn get<T: 'static>(&self, _id: SignalId) -> &T {
        todo!("elwindui-core: fallback reactive graph not yet implemented")
    }

    pub fn set<T: 'static>(&mut self, _id: SignalId, _value: T) {
        todo!("elwindui-core: fallback reactive graph not yet implemented")
    }

    pub fn depend_on(&mut self, _dependent: SignalId, _dependency: SignalId) {
        todo!("elwindui-core: fallback reactive graph not yet implemented")
    }
}

#[cfg(test)]
mod tests {
    use super::Subscription;
    use std::cell::Cell;
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
}
