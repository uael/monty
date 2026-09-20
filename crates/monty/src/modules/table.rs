//! The modules a session has imported, which `sys.modules` is.

use crate::{
    bytecode::VM,
    exception_private::{RunError, RunResult},
    heap::{ContainsHeap, HeapData, HeapId, HeapReadOutput},
    intern::StringId,
    types::Dict,
    value::Value,
};

/// One session's imported modules, by name.
///
/// `import x` builds a module the first time and finds that same one every
/// time after, so `import sys` twice in a session gives one object and an
/// attribute set on it in one snippet is there in the next. A host, or the
/// sandbox itself, puts a module of its own in here by writing to
/// `sys.modules`, and `import` then finds it: that is the only way a name no
/// standard module answers can be imported, since a sandbox reads no files.
///
/// **Owns one heap reference**, the dict, which lives as long as the session.
/// The dict owns a reference to every module in it.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct ModuleTable {
    /// The `sys.modules` dict, allocated the first time anything imports.
    dict: Option<HeapId>,
}

impl ModuleTable {
    /// The `sys.modules` dict, made if this session has none yet.
    ///
    /// The caller gets the raw id, which the table keeps owning.
    pub(crate) fn dict(&mut self, heap: &impl ContainsHeap) -> HeapId {
        *self
            .dict
            .get_or_insert_with(|| heap.heap().allocate(HeapData::Dict(Dict::new())))
    }
}

impl VM<'_> {
    /// The `sys.modules` dict of this session, with a reference of its own.
    pub(crate) fn modules_dict(&mut self) -> Value {
        let dict_id = self.modules.dict(self.heap);
        self.heap.inc_ref(dict_id);
        Value::Ref(dict_id)
    }

    /// What `name` is bound to in `sys.modules`, with a reference of its own.
    ///
    /// A name bound to something that is no module reads the same as a module:
    /// CPython binds whatever `sys.modules` holds, and so does this.
    pub(crate) fn imported(&mut self, name: StringId) -> Option<Value> {
        let dict_id = self.modules.dict(self.heap);
        let name = self.interns.get_str(name);
        let HeapData::Dict(dict) = self.heap.get(dict_id) else {
            unreachable!("the module table allocates a dict")
        };
        dict.get_by_str(name, self.heap, self.interns)
            .map(|held| held.clone_with_heap(self.heap))
    }

    /// Binds `module` to `name`, so the next import of it finds this one.
    ///
    /// Takes the reference it is given, and releases it if the bind fails.
    pub(crate) fn remember(&mut self, name: StringId, module: Value) -> RunResult<()> {
        let dict_id = self.modules.dict(self.heap);
        let HeapReadOutput::Dict(mut dict) = self.heap.read(dict_id) else {
            module.drop_with(self);
            return Err(RunError::internal("sys.modules is not a dict"));
        };
        if let Some(replaced) = dict.set(Value::InternString(name), module, self)? {
            replaced.drop_with(self);
        }
        Ok(())
    }
}
