//! Handles: the values of the sandbox with no data form, and its classes, that
//! a session holds for its host.
//!
//! A value crosses to the host as data, but a function, a generator or a
//! template carries no data the host could hand back as itself: it crosses as
//! a handle, and the session holds the value under that handle until the host
//! releases it, so a value the sandbox made and let go of is still there when
//! the host hands it back, or calls it. A class crosses as its type node under
//! its own boundary uuid, and a session that holds handles holds it the same
//! way. Only a session that asked for handles holds them; every other session
//! keeps such a value's `repr()`.

use std::collections::BTreeMap;

use monty_types::MontyUuid;

use super::{Heap, HeapId};
use crate::{boundary_uuid::create_uuid, intern::FunctionId, modules::ModuleFunctions, value::Value};

/// What a session holds for its host: the handle of each held value.
pub(crate) type Handles = BTreeMap<MontyUuid, Held>;

/// One value a session holds under a handle. A heap value is held by one
/// reference; the others are immediate values and need none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum Held {
    Ref(HeapId),
    Def(FunctionId),
    Module(ModuleFunctions),
}

impl Held {
    /// The held form of a value that can be held, by its identity.
    pub(crate) fn of(value: &Value) -> Option<Self> {
        match value {
            Value::Ref(id) => Some(Self::Ref(*id)),
            Value::DefFunction(function) => Some(Self::Def(*function)),
            Value::ModuleFunction(function) => Some(Self::Module(*function)),
            _ => None,
        }
    }
}

impl Heap {
    /// From now on, every value with no data form and every class that crosses
    /// to the host is held under a handle until the host releases it.
    pub(crate) fn hold_handles(&mut self) {
        self.handles.get_or_insert_default();
    }

    /// Whether the session holds handles.
    pub(crate) fn holds_handles(&self) -> bool {
        self.handles.is_some()
    }

    /// The handle `held` crosses under, held from now on, or `None` when the
    /// session holds no handles. A value held already keeps its handle; a new
    /// one takes `uuid` when it has an identity of its own, and a fresh one
    /// otherwise.
    pub(crate) fn hold(&mut self, held: Held, uuid: Option<MontyUuid>) -> Option<MontyUuid> {
        let handles = self.handles.as_mut()?;
        if let Some((existing, _)) = handles.iter().find(|(_, other)| **other == held) {
            return Some(*existing);
        }
        let uuid = uuid.unwrap_or_else(create_uuid);
        handles.insert(uuid, held);
        if let Held::Ref(id) = held {
            self.inc_ref(id);
        }
        Some(uuid)
    }

    /// The value held under `uuid`, as a new reference, if it is held.
    pub(crate) fn held(&self, uuid: &MontyUuid) -> Option<Value> {
        let held = *self.handles.as_ref()?.get(uuid)?;
        Some(match held {
            Held::Ref(id) => {
                self.inc_ref(id);
                Value::Ref(id)
            }
            Held::Def(function) => Value::DefFunction(function),
            Held::Module(function) => Value::ModuleFunction(function),
        })
    }

    /// The host holds `uuid` no more: the session lets go of the value, which
    /// lives on only if something else holds it. Whether it was held.
    pub(crate) fn release(&mut self, uuid: &MontyUuid) -> bool {
        let Some(held) = self.handles.as_mut().and_then(|handles| handles.remove(uuid)) else {
            return false;
        };
        if let Held::Ref(id) = held {
            self.dec_ref(id);
        }
        true
    }

    /// Lets go of every held value, as a session does when it ends.
    pub(crate) fn release_all(&mut self) {
        let held: Vec<MontyUuid> = self
            .handles
            .iter()
            .flat_map(|handles| handles.keys().copied())
            .collect();
        for uuid in held {
            self.release(&uuid);
        }
    }
}
