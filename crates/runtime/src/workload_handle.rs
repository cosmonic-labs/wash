use std::{any::Any, collections::HashMap, path::PathBuf, sync::Arc};
use wasmtime::Store;
use wasmtime::component::{Component, InstancePre, Linker};
use wasmtime_wasi::WasiCtxBuilder;

use crate::engine::{Ctx, CtxBuilder, Engine};
use crate::workload::VolumeMount;

/// An unresolved workload handle that plugins can bind to but cannot yet instantiate with full context.
/// This is created during the initial workload setup and passed to plugins during binding.
#[derive(Clone)]
pub struct UnresolvedWorkloadHandle {
    /// The unique identifier for the workload component
    id: String,
    /// The [`Engine`] used to compile this component
    engine: Engine,
    /// The compiled [`Component`]
    component: Component,
    /// Linker with all the necessary imports (modified during plugin binding)
    linker: Linker<Ctx>,
    /// Validated volume mounts (host_path, volume_mount_config)
    volume_mounts: Vec<(PathBuf, VolumeMount)>,
}

impl UnresolvedWorkloadHandle {
    /// Creates a new UnresolvedWorkloadHandle
    pub fn new(
        id: String,
        engine: Engine,
        component: Component,
        linker: Linker<Ctx>,
        volume_mounts: Vec<(PathBuf, VolumeMount)>,
    ) -> Self {
        Self {
            id,
            engine,
            component,
            linker,
            volume_mounts,
        }
    }

    /// Builds a base context for this workload without plugin contexts
    pub fn build_base_ctx(&self) -> Ctx {
        // If we have volume mounts, we need to build a custom WASI context
        let wasi_ctx = if !self.volume_mounts.is_empty() {
            let mut builder = WasiCtxBuilder::new();
            builder.args(&["main.wasm"]).inherit_stderr();

            for (host_path, volume_mount) in &self.volume_mounts {
                let (dir_perms, file_perms) = if volume_mount.read_only {
                    (
                        wasmtime_wasi::DirPerms::READ,
                        wasmtime_wasi::FilePerms::READ,
                    )
                } else {
                    (
                        wasmtime_wasi::DirPerms::MUTATE,
                        wasmtime_wasi::FilePerms::WRITE,
                    )
                };

                // Try to preopen the directory
                if let Err(e) = builder.preopened_dir(
                    host_path,
                    &volume_mount.mount_path,
                    dir_perms,
                    file_perms,
                ) {
                    tracing::error!(
                        "Failed to mount volume {} at {}: {}",
                        volume_mount.name,
                        volume_mount.mount_path,
                        e
                    );
                } else {
                    tracing::debug!(
                        "Successfully mounted volume {} at {}",
                        volume_mount.name,
                        volume_mount.mount_path
                    );
                }
            }

            builder.build()
        } else {
            // No volume mounts, use default context
            WasiCtxBuilder::new().inherit_stderr().build()
        };

        CtxBuilder::new(self.id.clone())
            .with_wasi_ctx(wasi_ctx)
            .build()
    }

    pub fn new_store_with_ctx(&self, ctx: Ctx) -> Store<Ctx> {
        Store::new(self.engine.inner(), ctx)
    }

    /// Gets the pre-instantiated component for this workload
    pub fn instantiate_pre(&self) -> anyhow::Result<InstancePre<Ctx>> {
        self.linker.instantiate_pre(&self.component)
    }

    // Instantiates a brand new instance of a component for this workload
    // pub async fn instantiate_async(&self) -> anyhow::Result<Instance> {
    //     let pre = self.linker.instantiate_pre(&self.component)?;
    //     pre.instantiate_async(self.new_store()).await
    // }

    // /// Instantiates a brand new instance of a component for this workload with the given [`Ctx`]
    // pub async fn instantiate_async_with_ctx(&self, ctx: Ctx) -> anyhow::Result<Instance> {
    //     let pre = self.linker.instantiate_pre(&self.component)?;
    //     pre.instantiate_async(self.new_store_with_ctx(ctx)).await
    // }

    /// Gets the linker for this workload
    pub fn linker(&mut self) -> &mut Linker<Ctx> {
        &mut self.linker
    }

    /// Gets the engine reference
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Resolve this handle into a fully configured WorkloadHandle
    pub fn resolve(
        self,
        plugins: HashMap<&'static str, Arc<dyn Any + Send + Sync>>,
    ) -> WorkloadHandle {
        WorkloadHandle::new(self, plugins)
    }
}

/// A fully resolved workload handle that contains all plugin contexts and bindings.
/// This is created after all plugins have bound to the workload's linker.
#[derive(Clone)]
pub struct WorkloadHandle {
    /// The underlying unresolved workload handle with fully-bound linker
    unresolved_handle: UnresolvedWorkloadHandle,
    /// Plugins
    plugins: HashMap<&'static str, Arc<dyn Any + Send + Sync>>,
}

impl WorkloadHandle {
    /// Creates a new WorkloadHandle from an unresolved handle
    pub fn new(
        unresolved_handle: UnresolvedWorkloadHandle,
        plugins: HashMap<&'static str, Arc<dyn Any + Send + Sync>>,
    ) -> Self {
        Self {
            unresolved_handle,
            plugins,
        }
    }

    /// Creates a new store with plugin contexts injected
    pub fn new_store(&self) -> Store<Ctx> {
        let ctx = self
            .unresolved_handle
            .build_base_ctx()
            .with_plugins(self.plugins.clone());
        self.unresolved_handle.new_store_with_ctx(ctx)
    }

    pub fn instantiate_pre(&self) -> anyhow::Result<InstancePre<Ctx>> {
        self.unresolved_handle.instantiate_pre()
    }

    pub async fn instantiate_async(&self) -> anyhow::Result<wasmtime::component::Instance> {
        let pre = self.unresolved_handle.instantiate_pre()?;
        pre.instantiate_async(&mut self.new_store()).await
    }

    /// Gets the engine reference
    pub fn engine(&self) -> &Engine {
        self.unresolved_handle.engine()
    }
}
