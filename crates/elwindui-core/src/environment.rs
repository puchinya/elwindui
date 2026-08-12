//! Typed, reactive UI context values inherited down the component tree.
//!
//! Environment resolution happens during component construction, never through Visual Tree
//! attachment: `view!` bodies build their descendant tree synchronously inside a generated
//! `Component::new()`, before any `UIElement` exists to attach to. See
//! `docs/design/runtime/theme_environment_design.md` (`## Environment`) for the full ownership and
//! propagation model, and `docs/specs/theme_environment_spec.md` §2 for the normative contract.
//!
//! Theme and Environment are separate systems and share no runtime type here — see
//! [`crate::theme`].

use crate::reactive::Subscription;
use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// Identifies one Environment value slot. Implemented by types generated from
/// `#[elwindui::environment_key]` (`docs/specs/theme_environment_spec.md` §2).
///
/// Lookup is by `Self`'s `TypeId`, never by name — two keys may share the same `Value` type
/// without colliding.
pub trait EnvironmentKey: 'static {
    /// The value type carried by this key.
    type Value: Clone + 'static;

    /// The value inherited when no context in the chain has ever overridden this key.
    fn default_value() -> Self::Value;
}

/// A reactive slot for one resolved `EnvironmentKey::Value`. Shared by `Rc` between every
/// `EnvironmentContext` that has not overridden the key (`docs/design/runtime/theme_environment_design.md`).
struct EnvironmentCell<T> {
    value: RefCell<T>,
    listeners: RefCell<Vec<Rc<dyn Fn()>>>,
}

impl<T: Clone> EnvironmentCell<T> {
    fn new(value: T) -> Self {
        Self {
            value: RefCell::new(value),
            listeners: RefCell::new(Vec::new()),
        }
    }

    fn get(&self) -> T {
        self.value.borrow().clone()
    }

    fn set(&self, value: T) {
        *self.value.borrow_mut() = value;
        self.notify();
    }

    fn notify(&self) {
        // Snapshot first: a listener may subscribe or cancel its own subscription in response to
        // the change it is being notified of.
        let listeners = self.listeners.borrow().clone();
        for listener in listeners {
            listener();
        }
    }
}

/// Type-erased storage entry, downcast back to `Rc<EnvironmentCell<K::Value>>` at the single
/// call site that put it there, keyed by `K`'s `TypeId`.
type ErasedCell = Rc<dyn Any>;

struct EnvironmentState {
    /// `None` only for the root context; every other context derives from one parent.
    parent: Option<Rc<EnvironmentState>>,
    /// Keys this exact context has overridden (via `set`) or, for the root, lazily materialized
    /// on first resolution. A context with no entry for `K` here defers to `parent`.
    own: RefCell<HashMap<TypeId, ErasedCell>>,
}

impl EnvironmentState {
    fn resolve_cell<K: EnvironmentKey>(self: &Rc<Self>) -> Rc<EnvironmentCell<K::Value>> {
        let type_id = TypeId::of::<K>();
        if let Some(existing) = self.own.borrow().get(&type_id) {
            return downcast_cell::<K>(existing);
        }
        if let Some(parent) = &self.parent {
            return parent.resolve_cell::<K>();
        }
        // Root, first access: materialize and cache the default so every context that shares
        // this key (directly or through `derive`) observes the same cell identity.
        let cell = Rc::new(EnvironmentCell::new(K::default_value()));
        self.own.borrow_mut().insert(type_id, cell.clone());
        cell
    }
}

fn downcast_cell<K: EnvironmentKey>(erased: &ErasedCell) -> Rc<EnvironmentCell<K::Value>> {
    erased
        .clone()
        .downcast::<EnvironmentCell<K::Value>>()
        .unwrap_or_else(|_| {
            panic!(
                "EnvironmentKey `{}` was registered with an incompatible Value type",
                std::any::type_name::<K>()
            )
        })
}

/// Shared, cheaply-`Clone`-able handle to the Environment values inherited at one point in the
/// component tree (`docs/specs/theme_environment_spec.md` §2).
#[derive(Clone)]
pub struct EnvironmentContext {
    state: Rc<EnvironmentState>,
}

impl std::fmt::Debug for EnvironmentContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EnvironmentContext")
            .field("own_entries", &self.state.own.borrow().len())
            .field("has_parent", &self.state.parent.is_some())
            .finish()
    }
}

impl Default for EnvironmentContext {
    fn default() -> Self {
        Self::root()
    }
}

impl EnvironmentContext {
    /// Creates the root context. Every key resolves to `K::default_value()` until overridden.
    pub fn root() -> Self {
        Self {
            state: Rc::new(EnvironmentState {
                parent: None,
                own: RefCell::new(HashMap::new()),
            }),
        }
    }

    /// Resolves the current value of `K` by walking from this context toward the root, returning
    /// the nearest override, or `K::default_value()` if none exists.
    pub fn get<K: EnvironmentKey>(&self) -> K::Value {
        self.state.resolve_cell::<K>().get()
    }

    /// Overrides `K` on this exact context.
    ///
    /// The first call on a given context allocates a new cell decoupled from whatever the parent
    /// chain currently resolves to, so a sibling context (or the parent) is never affected. A
    /// later call on the *same* context that already owns `K` mutates that cell in place instead,
    /// so subscribers already resolved against it observe the change
    /// (`docs/design/runtime/theme_environment_design.md`, "Change propagation").
    pub fn set<K: EnvironmentKey>(&self, value: K::Value) {
        let type_id = TypeId::of::<K>();
        if let Some(existing) = self.state.own.borrow().get(&type_id) {
            downcast_cell::<K>(existing).set(value);
            return;
        }
        self.state
            .own
            .borrow_mut()
            .insert(type_id, Rc::new(EnvironmentCell::new(value)));
    }

    /// Creates a child context. A key not later overridden on the child continues to resolve to
    /// this context's cell by `Rc` identity; `set` on the child allocates a cell for that key only
    /// (`docs/specs/theme_environment_spec.md` §2, `EnvironmentScope`).
    pub fn derive(&self) -> EnvironmentContext {
        Self {
            state: Rc::new(EnvironmentState {
                parent: Some(self.state.clone()),
                own: RefCell::new(HashMap::new()),
            }),
        }
    }

    /// Subscribes to changes of whichever cell `K` currently resolves to on this context.
    ///
    /// `elwindui-codegen` uses this to re-run a `#[environment(name)]` field's dependents through
    /// the same generated notification path used for `#[prop]`; DSL author code never calls this
    /// (`docs/specs/dsl_spec.md` §4). Callers must resolve/subscribe only after every ancestor
    /// `EnvironmentScope` override in scope has already called `set` — `#[component]` codegen
    /// guarantees this ordering (`docs/design/runtime/theme_environment_design.md`, "Resolution and
    /// component integration").
    #[doc(hidden)]
    pub fn subscribe<K: EnvironmentKey>(&self, listener: impl Fn() + 'static) -> Subscription {
        let cell = self.state.resolve_cell::<K>();
        let listener: Rc<dyn Fn()> = Rc::new(listener);
        cell.listeners.borrow_mut().push(listener.clone());
        let weak_cell = Rc::downgrade(&cell);
        Subscription::new(move || {
            if let Some(cell) = weak_cell.upgrade() {
                cell.listeners
                    .borrow_mut()
                    .retain(|registered| !Rc::ptr_eq(registered, &listener));
            }
        })
    }

    /// Returns the ambient context in effect right now.
    ///
    /// `elwindui-codegen` calls this from a `#[environment(name)]` field's generated initializer.
    /// `view!` bodies evaluate a component's descendant tree synchronously and in construction
    /// order (`docs/design/runtime/theme_environment_design.md`), so a nested-call thread-local
    /// stack observes exactly the same ambient context at each construction point that explicitly
    /// threading a hidden constructor parameter through every call would — without changing every
    /// generated constructor's signature to carry it. DSL author code never calls this directly
    /// (`docs/specs/dsl_spec.md` §4).
    #[doc(hidden)]
    pub fn current() -> EnvironmentContext {
        CURRENT.with(|stack| {
            stack
                .borrow()
                .last()
                .cloned()
                .unwrap_or_else(EnvironmentContext::root)
        })
    }

    /// Makes `self` the ambient context (see [`Self::current`]) for as long as the returned guard
    /// is alive, restoring whatever was ambient before on drop — including on unwind, so a panic
    /// during nested construction cannot leave a stale context ambient for unrelated later work on
    /// this thread.
    ///
    /// `elwindui-codegen` uses this to implement `EnvironmentScope { .. }`
    /// (`docs/specs/dsl_spec.md` §5): it derives an overridden context, enters it, builds the
    /// scope's children while it is ambient, then lets the guard drop before returning. DSL author
    /// code never calls this directly.
    #[doc(hidden)]
    #[must_use = "the ambient context reverts as soon as the guard is dropped"]
    pub fn enter(&self) -> EnvironmentContextGuard {
        CURRENT.with(|stack| stack.borrow_mut().push(self.clone()));
        EnvironmentContextGuard { _private: () }
    }
}

thread_local! {
    static CURRENT: RefCell<Vec<EnvironmentContext>> = RefCell::new(Vec::new());
}

/// RAII guard restoring the previously ambient [`EnvironmentContext`] on drop. See
/// [`EnvironmentContext::enter`].
#[doc(hidden)]
pub struct EnvironmentContextGuard {
    _private: (),
}

impl Drop for EnvironmentContextGuard {
    fn drop(&mut self) {
        CURRENT.with(|stack| {
            stack.borrow_mut().pop();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct LocaleKey;
    impl EnvironmentKey for LocaleKey {
        type Value = &'static str;
        fn default_value() -> Self::Value {
            "en-US"
        }
    }

    struct ScaleKey;
    impl EnvironmentKey for ScaleKey {
        type Value = f32;
        fn default_value() -> Self::Value {
            1.0
        }
    }

    #[test]
    fn unresolved_key_falls_back_to_default_value() {
        let root = EnvironmentContext::root();
        assert_eq!(root.get::<LocaleKey>(), "en-US");
    }

    #[test]
    fn unmodified_key_shares_cell_identity_across_derive() {
        let root = EnvironmentContext::root();
        // Force the root to materialize the default cell first.
        assert_eq!(root.get::<LocaleKey>(), "en-US");
        let child = root.derive();

        // A change on the root, observed through the still-shared cell, is visible on the child
        // that never overrode the key.
        root.set::<LocaleKey>("ja-JP");
        assert_eq!(child.get::<LocaleKey>(), "ja-JP");
    }

    #[test]
    fn overriding_a_key_on_a_child_does_not_affect_the_parent_or_a_sibling() {
        let root = EnvironmentContext::root();
        let overridden_child = root.derive();
        let plain_sibling = root.derive();

        overridden_child.set::<LocaleKey>("fr-FR");

        assert_eq!(overridden_child.get::<LocaleKey>(), "fr-FR");
        assert_eq!(root.get::<LocaleKey>(), "en-US");
        assert_eq!(plain_sibling.get::<LocaleKey>(), "en-US");
    }

    #[test]
    fn overriding_one_key_does_not_allocate_a_new_cell_for_a_sibling_key() {
        let root = EnvironmentContext::root();
        assert_eq!(root.get::<ScaleKey>(), 1.0);
        let child = root.derive();

        child.set::<LocaleKey>("de-DE");

        // `ScaleKey` was never overridden on `child`; a change on the root must still reach it.
        root.set::<ScaleKey>(2.0);
        assert_eq!(child.get::<ScaleKey>(), 2.0);
    }

    #[test]
    fn subscriber_is_notified_only_for_the_key_it_subscribed_to() {
        let root = EnvironmentContext::root();
        let locale_notifications = Rc::new(Cell::new(0));
        let scale_notifications = Rc::new(Cell::new(0));

        let locale_notifications_for_listener = locale_notifications.clone();
        let _locale_subscription = root.subscribe::<LocaleKey>(move || {
            locale_notifications_for_listener.set(locale_notifications_for_listener.get() + 1);
        });
        let scale_notifications_for_listener = scale_notifications.clone();
        let _scale_subscription = root.subscribe::<ScaleKey>(move || {
            scale_notifications_for_listener.set(scale_notifications_for_listener.get() + 1);
        });

        root.set::<LocaleKey>("ja-JP");
        assert_eq!(locale_notifications.get(), 1);
        assert_eq!(scale_notifications.get(), 0);
    }

    #[test]
    fn overriding_a_key_a_second_time_on_the_same_context_notifies_existing_subscribers() {
        let root = EnvironmentContext::root();
        let child = root.derive();
        child.set::<LocaleKey>("fr-FR");

        let notifications = Rc::new(Cell::new(0));
        let notifications_for_listener = notifications.clone();
        let _subscription = child.subscribe::<LocaleKey>(move || {
            notifications_for_listener.set(notifications_for_listener.get() + 1);
        });

        child.set::<LocaleKey>("it-IT");
        assert_eq!(notifications.get(), 1);
        assert_eq!(child.get::<LocaleKey>(), "it-IT");
        // The parent must remain unaffected by a re-override on the child.
        assert_eq!(root.get::<LocaleKey>(), "en-US");
    }

    #[test]
    fn dropping_the_subscription_stops_further_notifications() {
        let root = EnvironmentContext::root();
        let notifications = Rc::new(Cell::new(0));
        let notifications_for_listener = notifications.clone();
        let subscription = root.subscribe::<LocaleKey>(move || {
            notifications_for_listener.set(notifications_for_listener.get() + 1);
        });

        root.set::<LocaleKey>("ja-JP");
        assert_eq!(notifications.get(), 1);

        drop(subscription);
        root.set::<LocaleKey>("de-DE");
        assert_eq!(notifications.get(), 1);
    }

    #[test]
    fn context_is_cheaply_clonable_and_clones_observe_the_same_state() {
        let root = EnvironmentContext::root();
        let cloned = root.clone();
        root.set::<LocaleKey>("ja-JP");
        assert_eq!(cloned.get::<LocaleKey>(), "ja-JP");
    }

    #[test]
    fn current_falls_back_to_a_default_root_before_anything_ever_entered() {
        assert_eq!(EnvironmentContext::current().get::<LocaleKey>(), "en-US");
    }

    #[test]
    fn entering_a_context_makes_it_ambient_until_the_guard_drops() {
        let scoped = EnvironmentContext::root();
        scoped.set::<LocaleKey>("ja-JP");

        assert_eq!(EnvironmentContext::current().get::<LocaleKey>(), "en-US");
        {
            let _guard = scoped.enter();
            assert_eq!(EnvironmentContext::current().get::<LocaleKey>(), "ja-JP");

            // Nested entry stacks and unwinds correctly.
            let inner = EnvironmentContext::current().derive();
            inner.set::<LocaleKey>("fr-FR");
            {
                let _inner_guard = inner.enter();
                assert_eq!(EnvironmentContext::current().get::<LocaleKey>(), "fr-FR");
            }
            assert_eq!(EnvironmentContext::current().get::<LocaleKey>(), "ja-JP");
        }
        assert_eq!(EnvironmentContext::current().get::<LocaleKey>(), "en-US");
    }
}
