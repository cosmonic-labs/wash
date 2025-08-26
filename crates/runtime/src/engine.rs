use std::{
    any::{Any, TypeId},
    collections::HashMap,
    sync::Arc,
};

use anyhow::Context;
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime_wasi::{IoView, WasiCtx, WasiCtxBuilder, WasiView};
use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpView};

use crate::{EmptyDirVolume, HostPathVolume, Plugin, VolumeType, WorkloadHandle};
use std::path::PathBuf;

/// The context for a component store and linker, providing access to implementations of:
/// - wasi@0.2 interfaces
/// - wasi:http@0.2 interfaces
pub struct Ctx {
    /// Unique identifier for this component. This is a [uuid::Uuid::new_v4] string.
    pub id: String,
    /// The resource table used to manage resources in the Wasmtime store.
    pub table: wasmtime::component::ResourceTable,
    /// The WASI context used to provide WASI functionality to the component.
    pub ctx: WasiCtx,
    /// The HTTP context used to provide HTTP functionality to the component.
    pub http: WasiHttpCtx,
    /// The configuration for the plugins associated with this context. Plugins will
    /// be able to use [`Ctx::get_config`] to fetch their own configuration.
    // plugin_config: HashMap<TypeId, HashMap<String, String>>,
    plugins: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
}

impl Ctx {
    pub fn get_plugin<T: Plugin + 'static>(&self) -> Option<Arc<T>> {
        self.plugins
            .get(&TypeId::of::<T>())
            .and_then(|arc| Arc::downcast::<T>(arc.clone()).ok())
    }
}

/// Helper struct to build a [`Ctx`] with a builder pattern
pub struct CtxBuilder {
    id: String,
    ctx: WasiCtx,
}

impl CtxBuilder {
    pub fn new(id: String, args: Option<&[&str]>) -> Self {
        let args_vec: Vec<&str> = args
            .map(|a| a.iter().map(|s| s.as_ref()).collect())
            .unwrap_or_else(|| vec!["main.wasm"]);
        Self {
            id,
            ctx: WasiCtxBuilder::new()
                .args(&args_vec)
                .inherit_stderr()
                .build(),
        }
    }

    pub fn with_wasi_ctx(mut self, ctx: WasiCtx) -> Self {
        self.ctx = ctx;
        self
    }

    pub fn build(self) -> Ctx {
        Ctx {
            ctx: self.ctx,
            http: WasiHttpCtx::new(),
            ..Default::default()
        }
    }
}

impl Ctx {
    pub fn builder(id: String) -> CtxBuilder {
        CtxBuilder::new(id, None)
    }
}

impl Default for Ctx {
    fn default() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            table: ResourceTable::new(),
            ctx: WasiCtxBuilder::new()
                .args(&["main.wasm"])
                .inherit_stderr()
                .build(),
            http: WasiHttpCtx::new(),
            plugins: HashMap::new(),
        }
    }
}

impl std::fmt::Debug for Ctx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ctx")
            .field("id", &self.id)
            .field("table", &self.table)
            .finish()
    }
}

impl IoView for Ctx {
    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
}
impl WasiView for Ctx {
    fn ctx(&mut self) -> &mut WasiCtx {
        &mut self.ctx
    }
}

// Implement WasiHttpView for wasi:http@0.2
impl WasiHttpView for Ctx {
    fn ctx(&mut self) -> &mut WasiHttpCtx {
        &mut self.http
    }
}

#[derive(Debug, Clone)]
pub struct Engine {
    // wasmtime engine
    pub(crate) inner: wasmtime::Engine,
}

impl Engine {
    /// Creates a new [`EngineBuilder`]
    pub fn builder() -> EngineBuilder {
        EngineBuilder::default()
    }

    /// Gets a reference to the inner wasmtime engine
    pub fn inner(&self) -> &wasmtime::Engine {
        &self.inner
    }

    pub fn start_workload(&self, workload: crate::workload::Workload) -> anyhow::Result<()> {
        // Handle optional service
        let service = if let Some(svc) = &workload.service {
            Some(Component::new(&self.inner, svc.bytes.clone())?)
        } else {
            None
        };
        let _linker: Linker<Ctx> = Linker::new(&self.inner);
        if let Some(ref s) = service {
            let _ty = s.component_type();
            // TODO: Link to components
        }

        // Handle optional wit_world
        let _components = if let Some(wit_world) = &workload.wit_world {
            wit_world
                .components
                .iter()
                .map(|c| Component::new(&self.inner, c.bytes.clone()))
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        // TODO: link components to other components

        let mut service_wasi_ctx = &mut WasiCtxBuilder::new();

        for v in workload.volumes {
            // TODO: use for filesystem preopens
            let host_path = match v.volume_type {
                VolumeType::HostPath(HostPathVolume { local_path }) => {
                    let path = PathBuf::from(&local_path);
                    if !path.is_dir() {
                        anyhow::bail!(
                            "HostPath volume '{local_path}' does not exist or is not a directory",
                        );
                    }
                    path
                }
                VolumeType::EmptyDir(EmptyDirVolume {}) => {
                    // Create a temporary directory for the empty dir volume
                    let temp_dir = tempfile::tempdir()
                        .context("failed to create temp dir for empty dir volume")?;
                    tracing::debug!(path = ?temp_dir.path(), "created temp dir for empty dir volume");
                    temp_dir.keep()
                }
            };
            // Only process volume mounts if service exists
            if let Some(ref service) = workload.service {
                for vm in &service.local_resources.volume_mounts {
                    if vm.name == v.name {
                        let (dir_perms, file_perms) = if vm.read_only {
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
                        service_wasi_ctx = service_wasi_ctx.preopened_dir(
                            &host_path,
                            &vm.mount_path,
                            dir_perms,
                            file_perms,
                        )?;
                    }
                }
            }
        }

        // if let Some(wit_world) = workload.wit_world {
        //     for c in wit_world.components {
        //         let component = Component::new(&self.inner, c.bytes)?;
        //         // let mut linker: Linker<Ctx> = Linker::new(&self.inner);
        //         // Do this x max_pool
        //         // let instance_pre = linker
        //         // .instantiate_pre(&component)
        //         // .context("failed to pre-instantiate component")?;
        //     }
        // }

        Ok(())
    }

    pub fn stop_workload() {}

    /// Initialize a workload component and return a WorkloadHandle
    pub fn initialize_workload(&self, component: Component) -> anyhow::Result<WorkloadHandle> {
        // Create a linker for this component
        let mut linker: Linker<Ctx> = Linker::new(&self.inner);

        // Add WASI@0.2 interfaces to the linker
        wasmtime_wasi::add_to_linker_async(&mut linker).context("failed to add WASI to linker")?;

        // Add HTTP interfaces to the linker
        #[cfg(feature = "http")]
        wasmtime_wasi_http::add_only_http_to_linker_async(&mut linker)
            .context("failed to add wasi:http/types to linker")?;

        // Pre-instantiate the component
        let instance_pre = linker
            .instantiate_pre(&component)
            .context("failed to pre-instantiate component")?;

        // Create the WorkloadHandle
        Ok(WorkloadHandle::new(
            Arc::new(self.clone()),
            instance_pre,
            linker,
        ))
    }

    /// Initialize a service component and return a service handle
    pub fn initialize_service(&self, _component: Component) -> anyhow::Result<()> {
        // TODO: Implement service handle creation
        todo!("Service handle implementation pending")
    }
}

/// Builder for the [`Engine`]
#[derive(Default)]
pub struct EngineBuilder {
    config: wasmtime::Config,
}

impl EngineBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_config(mut self, config: wasmtime::Config) -> Self {
        self.config = config;
        self
    }
}

impl EngineBuilder {
    pub fn build(mut self) -> anyhow::Result<Engine> {
        // Async support must be enabled
        self.config.async_support(true);

        let inner = wasmtime::Engine::new(&self.config)?;
        Ok(Engine { inner })
    }
}
