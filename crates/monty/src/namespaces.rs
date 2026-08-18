//! Every global namespace a session holds over one heap.

use crate::{
    heap::{ContainsHeap, DropWithContext, Heap},
    namespace::ScopeId,
    value::Value,
};

/// The global namespaces of one session, each a vector of values indexed by
/// the slot map their shared [`Program`](crate::program::Program) owns.
///
/// A namespace is a name map, not a heap: two of them holding the same
/// `Value::Ref` name the same live object, so a mutation through one is seen
/// through the other while a rebinding through one is not. That is the whole
/// difference between dressing a child ([`dress`](Self::dress), which copies
/// the names and shares the objects) and forking a session (which copies the
/// heap and shares nothing).
///
/// Storage is a dense vector indexed by [`ScopeId`]. An entry is `None` when
/// its namespace has been released, and also while it is the one installed in
/// a running VM — the VM holds that vector directly so the hot load path stays
/// a single index.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct Namespaces {
    scopes: Vec<Option<Vec<Value>>>,
}

impl Namespaces {
    /// The values of a namespace that is not currently installed.
    pub(crate) fn get(&self, id: ScopeId) -> Option<&Vec<Value>> {
        self.scopes.get(id.index())?.as_ref()
    }

    /// Adds an empty namespace and returns its handle.
    ///
    /// Reuses a released slot when there is one, so a driver that opens and
    /// closes namespaces in a loop does not grow the vector without bound.
    ///
    /// `installed` is the namespace a VM currently holds: its slot is empty
    /// for exactly as long as that lasts, and handing it out again would make
    /// the new namespace an alias of the running one.
    pub(crate) fn create(&mut self, installed: ScopeId) -> Option<ScopeId> {
        let free = self
            .scopes
            .iter()
            .enumerate()
            .position(|(index, slot)| slot.is_none() && index != installed.index());
        let index = match free {
            Some(index) => index,
            None => {
                self.scopes.push(None);
                self.scopes.len() - 1
            }
        };
        let id = ScopeId::from_index(index)?;
        self.scopes[index] = Some(Vec::new());
        Some(id)
    }

    /// Fills a namespace `create` just made, replacing its empty vector.
    pub(crate) fn put_over(&mut self, id: ScopeId, values: Vec<Value>) {
        if let Some(slot) = self.scopes.get_mut(id.index()) {
            *slot = Some(values);
        }
    }

    /// Removes a namespace, releasing every reference it held.
    ///
    /// Returns `false` when the id names nothing, which is what a double
    /// release or a handle from a loaded dump looks like.
    pub(crate) fn release(&mut self, id: ScopeId, heap: &mut Heap) -> bool {
        let Some(slot) = self.scopes.get_mut(id.index()) else {
            return false;
        };
        let Some(values) = slot.take() else { return false };
        values.drop_with(heap);
        true
    }

    /// Lifts a namespace out so a VM can install it, leaving a hole.
    ///
    /// The hole is what [`get`](Self::get) reports as absent while the
    /// namespace is running; the caller must put it back.
    pub(crate) fn take(&mut self, id: ScopeId) -> Option<Vec<Value>> {
        self.scopes.get_mut(id.index())?.take()
    }

    /// Returns a namespace the VM had installed.
    ///
    /// Grows to reach `id` when it has to, so putting back what a
    /// default-constructed collection reports as installed cannot silently
    /// drop the values and leak every reference in them.
    pub(crate) fn put(&mut self, id: ScopeId, values: Vec<Value>) {
        if self.scopes.len() <= id.index() {
            self.scopes.resize_with(id.index() + 1, || None);
        }
        debug_assert!(
            self.scopes[id.index()].is_none(),
            "put over a namespace that was never taken"
        );
        self.scopes[id.index()] = Some(values);
    }

    /// Grows every namespace to `slots` entries.
    ///
    /// New names are appended to the program's one slot map, so a namespace
    /// that has not run since then is short; the slots it gains are
    /// `Undefined`, which is exactly "this namespace does not bind that name".
    pub(crate) fn ensure_slots(&mut self, slots: usize) {
        for values in self.scopes.iter_mut().flatten() {
            if values.len() < slots {
                values.resize_with(slots, || Value::Undefined);
            }
        }
    }

    /// Every heap id held by a namespace that is not installed.
    ///
    /// The leak check needs these as roots: a value a parked namespace names
    /// is reachable, however long it has been since that namespace ran.
    #[cfg(feature = "ref-count-return")]
    pub(crate) fn root_ids(&self) -> Vec<crate::heap::HeapId> {
        self.scopes
            .iter()
            .flatten()
            .flatten()
            .filter_map(|value| match value {
                Value::Ref(id) => Some(*id),
                _ => None,
            })
            .collect()
    }
}

impl<C: ContainsHeap> DropWithContext<C> for Namespaces {
    fn drop_with(self, ctx: &mut C) {
        for values in self.scopes.into_iter().flatten() {
            values.drop_with(ctx);
        }
    }
}

/// What a VM runs over: the namespace it has installed, the ones parked beside
/// it, and which of them is installed.
///
/// The installed one is held flat rather than left in the collection so the
/// load and store opcodes stay a single index into a vector; `parked` has a
/// hole at `current` for as long as that lasts. The three travel together
/// because they are only ever correct together.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct Scopes {
    /// Values of the installed namespace, indexed by slot.
    pub(crate) globals: Vec<Value>,
    /// Every other namespace, with a hole where the installed one was.
    pub(crate) parked: Namespaces,
    /// Which namespace `globals` holds.
    pub(crate) current: ScopeId,
}

impl Scopes {
    /// One empty namespace, [`ScopeId::ROOT`], installed.
    pub(crate) fn new() -> Self {
        Self {
            globals: Vec::new(),
            parked: Namespaces { scopes: vec![None] },
            current: ScopeId::ROOT,
        }
    }

    /// Moves the installed namespace to `target`, parking the old one.
    ///
    /// Returns `None` when `target` names nothing, which is what a handle for
    /// a released namespace looks like.
    pub(crate) fn reinstall(self, target: ScopeId) -> Option<Self> {
        if self.current == target {
            return Some(self);
        }
        Self::install(self.uninstall(), target)
    }

    /// Adds a namespace holding the same values as `from`, which may be the
    /// installed one.
    pub(crate) fn dress(&mut self, from: ScopeId, heap: &Heap) -> Option<ScopeId> {
        let source: Vec<Value> = self
            .get(from)?
            .iter()
            .map(|value| value.clone_with_heap(heap))
            .collect();
        let id = self.parked.create(self.current)?;
        self.parked.put_over(id, source);
        Some(id)
    }

    /// Adds an empty namespace.
    pub(crate) fn create(&mut self) -> Option<ScopeId> {
        self.parked.create(self.current)
    }

    /// Removes a namespace, releasing what it held. The installed one cannot
    /// be released: a session always has somewhere to run.
    pub(crate) fn release(&mut self, id: ScopeId, heap: &mut Heap) -> bool {
        id != self.current && self.parked.release(id, heap)
    }

    /// A session's worth of namespaces with `id` installed.
    ///
    /// Returns `None` when `id` names nothing, or names one already installed
    /// somewhere else.
    pub(crate) fn install(mut parked: Namespaces, id: ScopeId) -> Option<Self> {
        let globals = parked.take(id)?;
        Some(Self {
            globals,
            parked,
            current: id,
        })
    }

    /// Puts the installed namespace back, giving up the whole collection.
    pub(crate) fn uninstall(self) -> Namespaces {
        let Self {
            globals,
            mut parked,
            current,
        } = self;
        parked.put(current, globals);
        parked
    }

    /// The values of `id`, whether it is the installed namespace or a parked one.
    pub(crate) fn get(&self, id: ScopeId) -> Option<&Vec<Value>> {
        if id == self.current {
            Some(&self.globals)
        } else {
            self.parked.get(id)
        }
    }

    /// Grows every namespace, installed and parked, to `slots` entries.
    pub(crate) fn ensure_slots(&mut self, slots: usize) {
        if self.globals.len() < slots {
            self.globals.resize_with(slots, || Value::Undefined);
        }
        self.parked.ensure_slots(slots);
    }
}

impl<C: ContainsHeap> DropWithContext<C> for Scopes {
    fn drop_with(self, ctx: &mut C) {
        self.globals.drop_with(ctx);
        self.parked.drop_with(ctx);
    }
}
