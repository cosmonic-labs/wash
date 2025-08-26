use std::sync::Arc;

use wasmtime::Store;
use wasmtime::component::{InstancePre, Linker};

use crate::engine::{Ctx, Engine};

/// A handle to a workload that plugins can use to create stores and get instance pres.
/// This abstracts the runtime's management of components from the plugins.
#[derive(Clone)]
pub struct WorkloadHandle {
    /// Reference to the runtime engine
    engine: Arc<Engine>,
    /// Pre-instantiated component
    instance_pre: InstancePre<Ctx>,
    /// Linker with all the necessary imports
    linker: Linker<Ctx>,
}
// TODO: De-Arc the above, the instance pre / engine handles are light afaik

impl WorkloadHandle {
    /// Creates a new WorkloadHandle
    pub fn new(engine: Arc<Engine>, instance_pre: InstancePre<Ctx>, linker: Linker<Ctx>) -> Self {
        Self {
            engine,
            instance_pre,
            linker,
        }
    }

    /// Creates a new store with a fresh context for this workload
    pub fn new_store(&self) -> Store<Ctx> {
        // TODO: We need this Ctx to contain the proper files, sockets, plugins, etc.
        // Create a new context for this store
        let ctx = Ctx::default();
        Store::new(self.engine.inner(), ctx)
    }

    /// Creates a new store with the provided context
    pub fn new_store_with_ctx(&self, ctx: Ctx) -> Store<Ctx> {
        Store::new(self.engine.inner(), ctx)
    }

    /// Gets the pre-instantiated component for this workload
    pub fn instance_pre(&self) -> InstancePre<Ctx> {
        self.instance_pre.clone()
    }

    /// Gets the linker for this workload
    pub fn linker(&mut self) -> &mut Linker<Ctx> {
        &mut self.linker
    }

    /// Gets the engine reference
    pub fn engine(&self) -> Arc<Engine> {
        self.engine.clone()
    }
}
