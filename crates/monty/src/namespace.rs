/// Unique identifier for variable slots in namespaces (globals and function locals).
///
/// Used by the bytecode compiler to emit slot indices for variable access.
/// The VM uses these indices to read/write values in the globals vector
/// or the stack-inlined locals region.
///
/// Storage is `u16` because every bytecode opcode that takes a namespace
/// slot (`LoadLocal`, `LoadGlobal`, `StoreLocal`, …) encodes the slot in
/// 16 bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub(crate) struct NamespaceId(u16);

impl NamespaceId {
    /// Creates a `NamespaceId` from a `usize` slot index, returning `None` if
    /// the index doesn't fit in `u16`. Callers in the prepare phase wrap the
    /// `None` case in a `ParseError::Syntax` so user-input-driven overflows
    /// surface as a clean `SyntaxError` rather than a panic at emission time.
    pub fn new(index: usize) -> Option<Self> {
        u16::try_from(index).ok().map(Self)
    }

    /// Returns the slot index as the `u16` operand consumed by the bytecode.
    #[inline]
    pub fn as_u16(self) -> u16 {
        self.0
    }

    /// Returns the slot as `usize` for `Vec`/array indexing in the VM.
    #[inline]
    pub fn index(self) -> usize {
        self.0.into()
    }
}

/// Identifies one global namespace among the several a session can hold over
/// one heap.
///
/// Distinct from [`NamespaceId`], which is a slot *within* a namespace. Every
/// namespace over a session's heap shares the program's slot map, so
/// `NamespaceId(k)` names the same variable in all of them and `ScopeId` says
/// which of them is being read or written. A namespace that never bound slot
/// `k` holds `Undefined` there, which reads as the `NameError` it should.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, serde::Serialize, serde::Deserialize)]
pub struct ScopeId(u32);

impl ScopeId {
    /// The namespace a session starts with, and the one every API that does
    /// not name a namespace acts on.
    pub(crate) const ROOT: Self = Self(0);

    /// Builds a handle from a raw index. Only the owning collection should
    /// mint these; a handle for a namespace that does not exist is refused
    /// where it is used, not here.
    pub(crate) fn from_index(index: usize) -> Option<Self> {
        u32::try_from(index).ok().map(Self)
    }

    /// The index this handle addresses.
    #[inline]
    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }

    /// Rebuilds a handle a host is giving back, as it crossed the boundary.
    ///
    /// A handle naming no namespace is refused where it is used, not here, so
    /// a stale one from a released namespace or another session reads as "no
    /// such namespace" rather than addressing whatever now sits at its index.
    #[must_use]
    pub fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    /// The handle as it crosses to a host.
    #[must_use]
    pub fn raw(self) -> u32 {
        self.0
    }
}
