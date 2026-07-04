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
