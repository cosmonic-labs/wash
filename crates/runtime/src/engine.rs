use std::{
    any::{Any, TypeId},
    collections::HashMap,
    sync::Arc,
};

use anyhow::Context;
use tracing::warn;
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
    ctx: Option<WasiCtx>,
}

impl CtxBuilder {
    pub fn new(id: String) -> Self {
        Self { id, ctx: None }
    }

    pub fn with_wasi_ctx(mut self, ctx: WasiCtx) -> Self {
        self.ctx = Some(ctx);
        self
    }

    pub fn build(self) -> Ctx {
        Ctx {
            id: self.id,
            ctx: self.ctx.unwrap_or_else(|| {
                WasiCtxBuilder::new()
                    .args(&["main.wasm"])
                    .inherit_stderr()
                    .build()
            }),
            http: WasiHttpCtx::new(),
            ..Default::default()
        }
    }
}

impl Ctx {
    pub fn builder(id: String) -> CtxBuilder {
        CtxBuilder::new(id)
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

    pub fn start_workload(
        &self,
        workload: crate::workload::Workload,
    ) -> anyhow::Result<(Option<crate::workload::Service>, Vec<WorkloadHandle>)> {
        // Handle optional service - just validate for now, don't create handle yet
        let service = if let Some(svc) = &workload.service {
            warn!(
                "services not supported yet, validating that it's a proper component but not starting it"
            );
            let _component = Component::new(&self.inner, svc.bytes.clone())
                .context("failed to validate service component")?;
            Some(svc.clone())
        } else {
            None
        };

        // Process and validate volumes - create a lookup map from volume name to validated host path
        let mut validated_volumes = std::collections::HashMap::new();

        for v in workload.volumes {
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

            // Store the validated volume for later lookup
            validated_volumes.insert(v.name.clone(), host_path);
        }

        // Initialize all components in wit_world
        let mut workload_handles = Vec::new();
        if let Some(wit_world) = &workload.wit_world {
            for (idx, component) in wit_world.components.iter().enumerate() {
                match self.initialize_workload(component.clone(), &validated_volumes) {
                    Ok(handle) => {
                        tracing::debug!("Successfully initialized component {}", idx);
                        workload_handles.push(handle);
                    }
                    Err(e) => {
                        tracing::error!("Failed to initialize component {}: {}", idx, e);
                        // Decide if we want to fail fast or continue with other components
                        // For now, we'll fail fast
                        return Err(e).context(format!("failed to initialize component {}", idx));
                    }
                }
            }
        }

        Ok((service, workload_handles))
    }

    pub fn stop_workload() {}

    /// Initialize a workload component and return a WorkloadHandle
    fn initialize_workload(
        &self,
        component: crate::workload::Component,
        validated_volumes: &std::collections::HashMap<String, PathBuf>,
    ) -> anyhow::Result<WorkloadHandle> {
        // Create a wasmtime component from the bytes
        let wasmtime_component = Component::new(&self.inner, component.bytes)
            .context("failed to create component from bytes")?;

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
            .instantiate_pre(&wasmtime_component)
            .context("failed to pre-instantiate component")?;

        // Build volume mounts for this component by looking up validated volumes
        let mut component_volume_mounts = Vec::new();
        for vm in &component.local_resources.volume_mounts {
            if let Some(host_path) = validated_volumes.get(&vm.name) {
                component_volume_mounts.push((host_path.clone(), vm.clone()));
            } else {
                tracing::warn!("Component references volume '{}' that was not found in workload volumes", vm.name);
            }
        }

        // Create the WorkloadHandle with volume mounts
        // TODO: Pass component configuration (pool_size, max_invocations) to WorkloadHandle
        Ok(WorkloadHandle::new(
            self.clone(),
            instance_pre,
            linker,
            component_volume_mounts,
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
