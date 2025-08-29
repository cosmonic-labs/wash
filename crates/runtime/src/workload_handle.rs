use std::path::PathBuf;
use tracing;
use wasmtime::Store;
use wasmtime::component::{InstancePre, Linker};
use wasmtime_wasi::WasiCtxBuilder;

use crate::engine::{Ctx, CtxBuilder, Engine};
use crate::workload::VolumeMount;

/// A handle to a workload that plugins can use to create stores and get instance pres.
/// This abstracts the runtime's management of components from the plugins.
#[derive(Clone)]
pub struct WorkloadHandle {
    /// The [`Engine`] used to compile this component
    engine: Engine,
    /// Pre-instantiated component
    instance_pre: InstancePre<Ctx>,
    /// Linker with all the necessary imports
    linker: Linker<Ctx>,
    /// Validated volume mounts (host_path, volume_mount_config)
    volume_mounts: Vec<(PathBuf, VolumeMount)>,
}

impl WorkloadHandle {
    /// Creates a new WorkloadHandle
    pub fn new(
        engine: Engine,
        instance_pre: InstancePre<Ctx>,
        linker: Linker<Ctx>,
        volume_mounts: Vec<(PathBuf, VolumeMount)>,
    ) -> Self {
        Self {
            engine,
            instance_pre,
            linker,
            volume_mounts,
        }
    }

    /// Creates a new store with a fresh context for this workload
    pub fn new_store(&self) -> Store<Ctx> {
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

        let ctx = CtxBuilder::new(uuid::Uuid::new_v4().to_string())
            .with_wasi_ctx(wasi_ctx)
            .build();

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
    pub fn engine(&self) -> &Engine {
        &self.engine
    }
}
