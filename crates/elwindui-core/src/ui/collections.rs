//! Child-storage collections: the Visual tree's own [`UIElementVisualCollection`], the
//! `Panel.Children`-style [`UIElementCollection`], the generic [`ListExt`] shape non-`UIElement`
//! lists (`Menu::items`, `MenuBar::items`) implement, and the [`ChildList`]/[`DynamicChildSlot`]/
//! [`DynamicChild`] machinery the generated `if`/`for` view code drives.

use super::*;

/// The Visual tree's actual child storage (the low-level
/// counterpart to `Panel.Children`'s `UIElementCollection` below) — a plain, runtime-mutable
/// `add`/`insert`/`remove`/`remove_at`/`clear` collection. `UIElement::visual_children` holds
/// one of these directly; `UIElement::visual_children()` (the default trait method) just reads it.
/// Every mutation owns Visual-parent wiring and invalidates its owner. `owner` is set once, at
/// construction (from `__self_weak` — see `UIElement::construct`), and never changes afterward: no
/// two-stage bind-after-`Rc::new` step is needed anymore.
#[derive(Clone)]
pub struct UIElementVisualCollection {
    storage: Rc<RefCell<Vec<Rc<dyn UIElementExt>>>>,
    owner: Weak<dyn UIElementExt>,
}

impl UIElementVisualCollection {
    pub fn new(owner: Weak<dyn UIElementExt>) -> Self {
        Self {
            storage: Rc::new(RefCell::new(Vec::new())),
            owner,
        }
    }
    pub fn owner_rc(&self) -> Option<Rc<dyn UIElementExt>> {
        self.owner.upgrade()
    }
    pub fn add(&self, child: Rc<dyn UIElementExt>) {
        if let Some(owner) = self.owner_rc() {
            *child.as_ui_element().visual_parent.borrow_mut() = Some(Rc::downgrade(&owner));
            owner.invalidate_measure();
        }
        self.storage.borrow_mut().push(child);
    }
    pub fn insert(&self, index: usize, child: Rc<dyn UIElementExt>) {
        if let Some(owner) = self.owner_rc() {
            *child.as_ui_element().visual_parent.borrow_mut() = Some(Rc::downgrade(&owner));
            owner.invalidate_measure();
        }
        self.storage.borrow_mut().insert(index, child);
    }
    /// Removes the first entry pointer-equal to `child`, if any — returns whether one was found.
    pub fn remove(&self, child: &Rc<dyn UIElementExt>) -> bool {
        let mut storage = self.storage.borrow_mut();
        match storage.iter().position(|c| Rc::ptr_eq(c, child)) {
            Some(index) => {
                let removed = storage.remove(index);
                *removed.as_ui_element().visual_parent.borrow_mut() = None;
                if let Some(owner) = self.owner_rc() {
                    owner.invalidate_measure();
                }
                true
            }
            None => false,
        }
    }
    pub fn remove_at(&self, index: usize) -> Rc<dyn UIElementExt> {
        let child = self.storage.borrow_mut().remove(index);
        *child.as_ui_element().visual_parent.borrow_mut() = None;
        if let Some(owner) = self.owner_rc() {
            owner.invalidate_measure();
        }
        child
    }
    pub fn clear(&self) {
        let children = std::mem::take(&mut *self.storage.borrow_mut());
        for child in children {
            *child.as_ui_element().visual_parent.borrow_mut() = None;
        }
        if let Some(owner) = self.owner_rc() {
            owner.invalidate_measure();
        }
    }
    pub fn len(&self) -> usize {
        self.storage.borrow().len()
    }
    pub fn is_empty(&self) -> bool {
        self.storage.borrow().is_empty()
    }
    pub fn to_vec(&self) -> Vec<Rc<dyn UIElementExt>> {
        self.storage.borrow().clone()
    }
}

/// The Logical-tree-shaped child list a container (`Layout`/`Control` family) declares in
/// the DSL — WinUI3's own `UIElementCollection` (docs/design/runtime/ui_tree_design.md), e.g.
/// `Panel.Children`. There is no separate, generically-traversable Logical tree: this is simply the
/// convenience API a *particular* component exposes for its own children, which automatically stays
/// in sync with the real Visual tree — `add`/`insert`/`remove`/`remove_at`/`clear` all mutate the
/// its own storage and additionally keeps each affected child's Logical `parent` pointer correct.
/// Deliberately has no way to replace its storage wholesale (no `set_children`) — every mutation
/// goes through one of these add/remove operations, so the Visual tree can never silently drift out
/// of sync with whatever a container thinks its own children are.
#[derive(Clone)]
pub struct UIElementCollection {
    storage: Rc<RefCell<Vec<Rc<dyn UIElementExt>>>>,
    owner: Weak<dyn UIElementExt>,
}

impl UIElementCollection {
    pub fn new(owner: Weak<dyn UIElementExt>) -> Self {
        Self {
            storage: Rc::new(RefCell::new(Vec::new())),
            owner,
        }
    }
    fn owner_rc(&self) -> Option<Rc<dyn UIElementExt>> {
        self.owner.upgrade()
    }
    pub fn add(&self, child: Rc<dyn UIElementExt>) {
        if let Some(owner) = self.owner_rc() {
            *child.as_ui_element().parent.borrow_mut() = Some(Rc::downgrade(&owner));
        }
        if let Some(owner) = self.owner_rc() {
            owner.as_ui_element().visual_collection.add(child.clone());
        }
        self.storage.borrow_mut().push(child);
    }
    pub fn insert(&self, index: usize, child: Rc<dyn UIElementExt>) {
        if let Some(owner) = self.owner_rc() {
            *child.as_ui_element().parent.borrow_mut() = Some(Rc::downgrade(&owner));
        }
        if let Some(owner) = self.owner_rc() {
            owner
                .as_ui_element()
                .visual_collection
                .insert(index, child.clone());
        }
        self.storage.borrow_mut().insert(index, child);
    }
    pub fn remove(&self, child: &Rc<dyn UIElementExt>) -> bool {
        let mut storage = self.storage.borrow_mut();
        let removed = storage
            .iter()
            .position(|candidate| Rc::ptr_eq(candidate, child))
            .map(|index| storage.remove(index));
        drop(storage);
        if let Some(removed) = removed {
            *child.as_ui_element().parent.borrow_mut() = None;
            if let Some(owner) = self.owner_rc() {
                owner.as_ui_element().visual_collection.remove(&removed);
            }
            true
        } else {
            false
        }
    }
    pub fn remove_at(&self, index: usize) -> Rc<dyn UIElementExt> {
        let child = self.storage.borrow_mut().remove(index);
        *child.as_ui_element().parent.borrow_mut() = None;
        if let Some(owner) = self.owner_rc() {
            owner.as_ui_element().visual_collection.remove(&child);
        }
        child
    }
    pub fn clear(&self) {
        for child in self.to_vec() {
            *child.as_ui_element().parent.borrow_mut() = None;
            if let Some(owner) = self.owner_rc() {
                owner.as_ui_element().visual_collection.remove(&child);
            }
        }
        self.storage.borrow_mut().clear();
    }
    pub fn len(&self) -> usize {
        self.storage.borrow().len()
    }
    pub fn is_empty(&self) -> bool {
        self.storage.borrow().is_empty()
    }
    pub fn to_vec(&self) -> Vec<Rc<dyn UIElementExt>> {
        self.storage.borrow().clone()
    }
}

/// Lets a `Layout`-family container's own `UIElementCollection` (`VerticalLayout`/`HorizontalLayout`/
/// `Grid`'s `children`) serve as a dynamic-child-range host the exact same way `TabView`/`Menu`/
/// `MenuBar`'s own dedicated `ListExt` implementors already do (`elwindui-codegen`'s
/// `DynamicChildSlot::replace_children`/`replace_rc_items`, driving `if`/`for`/`match` inside a
/// DSL view) — every method here already exists verbatim as one of `UIElementCollection`'s own
/// inherent methods just above; this only adds the trait so a `&UIElementCollection` can also be used
/// as `&dyn ListExt<dyn UIElementExt>` where the generated code needs one. The inherent methods
/// remain what ordinary `.add(..)`-style call sites resolve to (inherent methods take priority over
/// trait methods for a concrete receiver type) — this impl only matters for `dyn`-erased callers.
impl ListExt<dyn UIElementExt> for UIElementCollection {
    fn add(&self, item: Rc<dyn UIElementExt>) {
        UIElementCollection::add(self, item);
    }
    fn insert(&self, index: usize, item: Rc<dyn UIElementExt>) {
        UIElementCollection::insert(self, index, item);
    }
    fn remove(&self, item: &Rc<dyn UIElementExt>) -> bool {
        UIElementCollection::remove(self, item)
    }
    fn remove_at(&self, index: usize) -> Rc<dyn UIElementExt> {
        UIElementCollection::remove_at(self, index)
    }
    fn clear(&self) {
        UIElementCollection::clear(self);
    }
    fn len(&self) -> usize {
        UIElementCollection::len(self)
    }
    fn is_empty(&self) -> bool {
        UIElementCollection::is_empty(self)
    }
    fn to_vec(&self) -> Vec<Rc<dyn UIElementExt>> {
        UIElementCollection::to_vec(self)
    }
}

/// A generic, `Vec`-like collection abstraction — `add`/`insert`/`remove`/`remove_at`/`clear`/
/// `len`/`is_empty`/`to_vec` mirror `UIElementCollection`'s own method set (see that struct's own
/// doc comment), minus the `UIElement`-tree-specific `parent`-pointer wiring `add`/`insert`/
/// `remove`/`remove_at` do there — `ListExt<T>` items aren't necessarily `UIElement`s at all (e.g.
/// `Menu::items`/`MenuBar::items` hold `Rc<dyn MenuItemExt>`/`Rc<dyn MenuBarItemExt>`, neither of
/// which is part of the `UIElement` visual tree). A plain hand-written trait, not `#[class]`-managed
/// (the macro's `trait_only`/`struct_only` shapes are for the concrete elwindui class hierarchy;
/// `ListExt<T>` is a generic utility type, one level below that, the same way `UIElementCollection`
/// itself is a plain hand-written struct rather than a `#[class]`-managed one). Each backend
/// provides its own concrete implementor per `Menu`/`MenuBar` (see `Menu::items`/`MenuBar::items`'s
/// own doc comment) — `elwindui-core` only declares the shape.
pub trait ListExt<T: ?Sized> {
    fn add(&self, item: Rc<T>);
    fn insert(&self, index: usize, item: Rc<T>);
    fn remove(&self, item: &Rc<T>) -> bool;
    fn remove_at(&self, index: usize) -> Rc<T>;
    fn clear(&self);
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool;
    fn to_vec(&self) -> Vec<Rc<T>>;
}

/// Backing storage for a [`ListExt`] implementation: an ordered `Rc<T>` list with identity
/// (`Rc::ptr_eq`) removal, and nothing else.
///
/// Every backend's `Menu::items` / `MenuBar::items` / `TabView::children` needs exactly this same
/// bookkeeping plus one backend-specific "now re-sync the native widget" call, so the bookkeeping
/// half lives here and each backend's `ListExt` impl becomes delegation + its own `rebuild()`.
/// Deliberately hook-free — a caller that needs a pre-step (`TabView::add`'s header-listener
/// attach, say) does it around the delegation rather than handing this type a callback, keeping it
/// a plain container. This is `UIElementCollection`'s sibling for lists that are *not* part of the
/// visual tree and therefore have no parent/visual-collection wiring to maintain.
pub struct ChildList<T: ?Sized> {
    items: RefCell<Vec<Rc<T>>>,
}

impl<T: ?Sized> Default for ChildList<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: ?Sized> ChildList<T> {
    pub fn new() -> Self {
        Self {
            items: RefCell::new(Vec::new()),
        }
    }
    pub fn add(&self, item: Rc<T>) {
        self.items.borrow_mut().push(item);
    }
    /// `index` is clamped to the current length, so an out-of-range insert appends rather than
    /// panicking the way `Vec::insert` would.
    pub fn insert(&self, index: usize, item: Rc<T>) {
        let mut items = self.items.borrow_mut();
        let index = index.min(items.len());
        items.insert(index, item);
    }
    /// Identity-based (`Rc::ptr_eq`), not value-based — `T` is a trait object with no `PartialEq`.
    pub fn remove(&self, item: &Rc<T>) -> bool {
        let mut items = self.items.borrow_mut();
        let Some(index) = items
            .iter()
            .position(|candidate| Rc::ptr_eq(candidate, item))
        else {
            return false;
        };
        items.remove(index);
        true
    }
    pub fn remove_at(&self, index: usize) -> Rc<T> {
        self.items.borrow_mut().remove(index)
    }
    pub fn clear(&self) {
        self.items.borrow_mut().clear();
    }
    /// Swaps the whole contents in one shot. The reconciling `set_children` implementations use
    /// this with a vector they built from [`Self::to_vec`], so the native add/remove calls their
    /// diff triggers happen with *no* borrow of this list outstanding — holding a `RefCell` borrow
    /// across a call back into backend/user code is the shape that produced the `makeFirstResponder`
    /// double-borrow panic (see `focus`'s own notes), so this type offers no `retain`-style
    /// borrow-holding callback API.
    pub fn replace_all(&self, items: Vec<Rc<T>>) {
        *self.items.borrow_mut() = items;
    }
    pub fn len(&self) -> usize {
        self.items.borrow().len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.borrow().is_empty()
    }
    pub fn to_vec(&self) -> Vec<Rc<T>> {
        self.items.borrow().clone()
    }
}

/// State owned by a generated dynamic child range. It is deliberately not a `UIElement`: callers
/// pass the resolved parent collection on every update, so `for`/`if`/`match` insert their actual
/// children directly into that collection. For `Vec<Rc<U>>` sources, unchanged source identities
/// retain their already-constructed child instances.
pub struct DynamicChildSlot<T: ?Sized> {
    keys: RefCell<Vec<usize>>,
    items: RefCell<Vec<Rc<DynamicChild<T>>>>,
}

/// A dynamic child together with subscriptions that must live exactly as long as that child.
/// This is an ownership record, not a UI node: the contained child is inserted directly in the
/// parent's declared content collection.
pub struct DynamicChild<T: ?Sized> {
    pub child: Rc<T>,
    /// Additional siblings produced by the same logical `for` item. `child` remains separate to
    /// preserve the single-child API used by existing generated code.
    pub siblings: Vec<Rc<T>>,
    pub subscriptions: Vec<crate::reactive::Subscription>,
}

impl<T: ?Sized> DynamicChild<T> {
    pub fn new(child: Rc<T>) -> Self {
        Self {
            child,
            siblings: Vec::new(),
            subscriptions: Vec::new(),
        }
    }

    pub fn with_subscriptions(
        child: Rc<T>,
        subscriptions: Vec<crate::reactive::Subscription>,
    ) -> Self {
        Self {
            child,
            siblings: Vec::new(),
            subscriptions,
        }
    }

    pub fn with_children(
        children: Vec<Rc<T>>,
        subscriptions: Vec<crate::reactive::Subscription>,
    ) -> Self {
        let mut children = children.into_iter();
        let child = children
            .next()
            .expect("a dynamic child item must contain at least one child");
        Self {
            child,
            siblings: children.collect(),
            subscriptions,
        }
    }

    fn child_count(&self) -> usize {
        1 + self.siblings.len()
    }

    fn children(&self) -> impl Iterator<Item = &Rc<T>> {
        std::iter::once(&self.child).chain(self.siblings.iter())
    }
}

impl<T: ?Sized> Default for DynamicChildSlot<T> {
    fn default() -> Self {
        Self {
            keys: RefCell::new(Vec::new()),
            items: RefCell::new(Vec::new()),
        }
    }
}

impl<T: ?Sized> DynamicChildSlot<T> {
    /// Number of children this slot currently occupies in its parent collection.
    pub fn len(&self) -> usize {
        self.items
            .borrow()
            .iter()
            .map(|item| item.child_count())
            .sum()
    }

    pub fn replace_rc_items<U: 'static>(
        &self,
        host: &dyn ListExt<T>,
        start: usize,
        items: &[Rc<U>],
        render: impl Fn(&Rc<U>) -> DynamicChild<T>,
    ) {
        let previous_keys = self.keys.borrow();
        let previous_items = self.items.borrow();
        let mut next_keys = Vec::with_capacity(items.len());
        let mut next_items = Vec::with_capacity(items.len());
        for item in items {
            let key = Rc::as_ptr(item) as usize;
            let rendered = previous_keys
                .iter()
                .position(|previous| *previous == key)
                .map(|index| Rc::clone(&previous_items[index]))
                .unwrap_or_else(|| Rc::new(render(item)));
            next_keys.push(key);
            next_items.push(rendered);
        }
        drop(previous_items);
        drop(previous_keys);
        self.replace_at(host, start, next_keys, next_items);
    }

    /// Rebuilds only this slot for collections that do not provide stable `Rc` identity. Unlike
    /// `replace_rc_items`, no item instance is retained across calls; static siblings and other
    /// dynamic slots remain untouched.
    pub fn replace_items<U>(
        &self,
        host: &dyn ListExt<T>,
        start: usize,
        items: impl IntoIterator<Item = U>,
        render: impl Fn(&U) -> DynamicChild<T>,
    ) {
        let items = items
            .into_iter()
            .map(|item| Rc::new(render(&item)))
            .collect();
        self.replace_at(host, start, Vec::new(), items);
    }

    pub fn replace_children(&self, host: &dyn ListExt<T>, start: usize, children: Vec<Rc<T>>) {
        self.replace_at(
            host,
            start,
            Vec::new(),
            children
                .into_iter()
                .map(|child| Rc::new(DynamicChild::new(child)))
                .collect(),
        );
    }

    fn replace_at(
        &self,
        host: &dyn ListExt<T>,
        start: usize,
        keys: Vec<usize>,
        items: Vec<Rc<DynamicChild<T>>>,
    ) {
        let previous = self.items.borrow();
        let previous_children: Vec<_> = previous
            .iter()
            .flat_map(|item| item.children().cloned())
            .collect();
        let next_children: Vec<_> = items
            .iter()
            .flat_map(|item| item.children().cloned())
            .collect();
        let shared = previous_children.len().min(next_children.len());
        for index in 0..shared {
            if !Rc::ptr_eq(&previous_children[index], &next_children[index]) {
                host.remove_at(start + index);
                host.insert(start + index, Rc::clone(&next_children[index]));
            }
        }
        for _ in next_children.len()..previous_children.len() {
            host.remove_at(start + next_children.len());
        }
        for (index, child) in next_children
            .iter()
            .enumerate()
            .skip(previous_children.len())
        {
            host.insert(start + index, Rc::clone(child));
        }
        drop(previous);
        *self.keys.borrow_mut() = keys;
        *self.items.borrow_mut() = items;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::testsupport::*;

    #[test]
    fn logical_and_visual_parents_are_set_by_collections() {
        let leaf = native("a", size(10.0, 20.0));
        let root = stack(Orientation::Vertical, 0.0, vec![Rc::clone(&leaf)]);
        assert!(Rc::ptr_eq(
            &leaf.parent().expect("leaf should have a logical parent"),
            &root
        ));
        assert!(Rc::ptr_eq(
            &leaf
                .visual_parent()
                .expect("leaf should have a visual parent"),
            &root
        ));
        assert!(root.parent().is_none());
    }

    #[test]
    fn runtime_add_and_remove_after_construction_wire_parent_and_visual_children() {
        // `UIElementCollection::add`/`remove` must work *after* the owner is already `Rc`-wrapped
        // after the owner is already constructed.
        let root = VerticalLayout::new();
        let root_erased: Rc<dyn UIElementExt> = root.clone();
        let children = root.children().clone();
        assert!(root.visual_children().is_empty());

        let child = native("a", size(10.0, 20.0));
        children.add(Rc::clone(&child));

        assert_eq!(root.visual_children().len(), 1);
        assert!(Rc::ptr_eq(
            &child
                .parent()
                .expect("add should wire the child's logical parent"),
            &root_erased
        ));
        assert!(Rc::ptr_eq(
            &child
                .visual_parent()
                .expect("add should wire the child's visual parent"),
            &root_erased
        ));

        assert!(children.remove(&child));
        assert!(root.visual_children().is_empty());
        assert!(
            child.parent().is_none(),
            "remove should clear the child's parent"
        );
        assert!(
            child.visual_parent().is_none(),
            "remove should clear the child's visual parent"
        );
    }

    #[test]
    fn logical_and_visual_collections_keep_their_parent_relationships_separate() {
        let root = VerticalLayout::new();
        let root_erased: Rc<dyn UIElementExt> = root.clone();

        let visual_only = TextBlock::new();
        root.as_ui_element()
            .visual_collection
            .add(visual_only.clone());
        assert!(visual_only.parent().is_none());
        assert!(Rc::ptr_eq(
            &visual_only.visual_parent().expect("visual parent"),
            &root_erased
        ));

        let logical_child = TextBlock::new();
        root.children().add(logical_child.clone());
        assert!(Rc::ptr_eq(
            &logical_child.parent().expect("logical parent"),
            &root_erased
        ));
        assert!(Rc::ptr_eq(
            &logical_child.visual_parent().expect("visual parent"),
            &root_erased
        ));
    }

    #[test]
    fn dynamic_child_slot_reuses_rc_item_children_and_applies_source_order() {
        struct TestList(RefCell<Vec<Rc<String>>>);

        impl ListExt<String> for TestList {
            fn add(&self, item: Rc<String>) {
                self.0.borrow_mut().push(item);
            }
            fn insert(&self, index: usize, item: Rc<String>) {
                self.0.borrow_mut().insert(index, item);
            }
            fn remove(&self, item: &Rc<String>) -> bool {
                let mut items = self.0.borrow_mut();
                let Some(index) = items.iter().position(|current| Rc::ptr_eq(current, item)) else {
                    return false;
                };
                items.remove(index);
                true
            }
            fn remove_at(&self, index: usize) -> Rc<String> {
                self.0.borrow_mut().remove(index)
            }
            fn clear(&self) {
                self.0.borrow_mut().clear();
            }
            fn len(&self) -> usize {
                self.0.borrow().len()
            }
            fn is_empty(&self) -> bool {
                self.0.borrow().is_empty()
            }
            fn to_vec(&self) -> Vec<Rc<String>> {
                self.0.borrow().clone()
            }
        }

        let slot = DynamicChildSlot::<String>::default();
        let host = TestList(RefCell::new(Vec::new()));
        let leading = Rc::new("leading".to_owned());
        let trailing = Rc::new("trailing".to_owned());
        let first = Rc::new("first".to_owned());
        let second = Rc::new("second".to_owned());
        let renders = Cell::new(0);
        let first_subscription_dropped = Rc::new(Cell::new(false));
        let second_subscription_dropped = Rc::new(Cell::new(false));
        host.add(Rc::clone(&leading));
        host.add(Rc::clone(&trailing));

        slot.replace_rc_items(&host, 1, &[Rc::clone(&first), Rc::clone(&second)], |item| {
            renders.set(renders.get() + 1);
            let dropped = if Rc::ptr_eq(item, &first) {
                Rc::clone(&first_subscription_dropped)
            } else {
                Rc::clone(&second_subscription_dropped)
            };
            DynamicChild::with_subscriptions(
                Rc::new(format!("child:{item}")),
                vec![crate::reactive::Subscription::new(move || {
                    dropped.set(true)
                })],
            )
        });
        let original = host.to_vec();
        assert_eq!(renders.get(), 2);
        assert!(Rc::ptr_eq(&original[0], &leading));
        assert!(Rc::ptr_eq(&original[3], &trailing));

        slot.replace_rc_items(&host, 1, &[Rc::clone(&second), Rc::clone(&first)], |_| {
            panic!("an unchanged Rc item must reuse its child")
        });
        let reordered = host.to_vec();
        assert_eq!(renders.get(), 2);
        assert!(Rc::ptr_eq(&reordered[0], &leading));
        assert!(Rc::ptr_eq(&reordered[1], &original[2]));
        assert!(Rc::ptr_eq(&reordered[2], &original[1]));
        assert!(Rc::ptr_eq(&reordered[3], &trailing));

        slot.replace_rc_items(&host, 1, &[Rc::clone(&second)], |_| {
            panic!("a retained Rc item must not be rendered again")
        });
        assert!(first_subscription_dropped.get());
        assert!(!second_subscription_dropped.get());

        slot.replace_children(
            &host,
            1,
            vec![
                Rc::new("first-child".to_owned()),
                Rc::new("second-child".to_owned()),
            ],
        );
        assert_eq!(slot.len(), 2);
        assert_eq!(
            host.to_vec().len(),
            4,
            "the range occupies both grouped children"
        );
    }
}
