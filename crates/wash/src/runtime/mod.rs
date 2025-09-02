use anyhow::{self, Context as _};
use runtime::Engine;
use std::collections::HashMap;
use std::fmt::Debug;
use std::path::Path;
use std::sync::Arc;
use std::thread::JoinHandle;
use tokio::{process::Child, sync::RwLock};
use tracing::{debug, error, info, trace, warn};
use wasmtime::component::ResourceTable;
use wasmtime_wasi::{IoView, WasiCtx, WasiCtxBuilder, WasiView};
use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpView};

use crate::{
    dev::DevPluginManager, plugin::PluginComponent, runtime::wasm::link_imports_plugin_exports,
};

pub mod bindings;
pub mod plugin;
mod wasm;

/// Use the runtime crate's Ctx
pub use runtime::engine::Ctx;

// WASI logging is implemented in the runtime crate via the Plugin system

// WASI config runtime is implemented in the runtime crate via the Plugin system

/// Create a new [`runtime::Engine`], returning the engine.
pub fn new_engine() -> anyhow::Result<Engine> {
    Engine::builder().build()
}

/// Alias for new_engine for backward compatibility
pub fn new_runtime() -> anyhow::Result<Engine> {
    new_engine()
}

/// Prepare a WebAssembly component for use as a wash plugin using the local runtime
pub async fn prepare_component_plugin(
    engine: &Engine,
    wasm: &[u8],
    data_dir: Option<&Path>,
) -> anyhow::Result<PluginComponent> {
    use wasmtime::component::Component;
    use wasmtime::component::Linker;

    // Create a wasmtime component from the bytes
    let wasmtime_component =
        Component::new(engine.inner(), wasm).context("failed to compile WebAssembly component")?;

    // Create a linker and add plugin interfaces
    let mut linker: Linker<Ctx> = Linker::new(engine.inner());

    // Add WASI interfaces
    wasmtime_wasi::add_to_linker_async(&mut linker).context("failed to add WASI to linker")?;

    // Add HTTP interfaces
    wasmtime_wasi_http::add_only_http_to_linker_async(&mut linker)
        .context("failed to add WASI HTTP to linker")?;

    // Add wash plugin host interfaces
    crate::runtime::bindings::plugin::wasmcloud::wash::types::add_to_linker(&mut linker, |ctx| ctx)
        .context("failed to add wash plugin bindings to linker")?;

    // Pre-instantiate the component
    let instance_pre = linker
        .instantiate_pre(&wasmtime_component)
        .context("failed to pre-instantiate component")?;

    // Create a WorkloadHandle for the plugin
    let workload_handle = runtime::WorkloadHandle::new(
        engine.clone(),
        instance_pre,
        linker,
        vec![], // No volume mounts for plugins
    );

    // Create the PluginComponent
    PluginComponent::new(workload_handle, data_dir).await
}

/// Prepare a WebAssembly component for use in development mode using the local runtime
pub async fn prepare_component_dev(
    engine: &Engine,
    wasm: &[u8],
    plugin_manager: Arc<DevPluginManager>,
) -> anyhow::Result<runtime::WorkloadHandle> {
    use wasmtime::component::Component;
    use wasmtime::component::Linker;

    // Create a wasmtime component from the bytes
    let wasmtime_component =
        Component::new(engine.inner(), wasm).context("failed to compile WebAssembly component")?;

    // Create a linker and add dev interfaces
    let mut linker: Linker<Ctx> = Linker::new(engine.inner());

    // Add WASI interfaces
    wasmtime_wasi::add_to_linker_async(&mut linker).context("failed to add WASI to linker")?;

    // Add HTTP interfaces
    wasmtime_wasi_http::add_only_http_to_linker_async(&mut linker)
        .context("failed to add WASI HTTP to linker")?;

    // Add wash dev bindings (this might not be needed for dev components)
    // TODO: Check if dev world needs specific host bindings

    // Link plugin exports to component imports
    crate::runtime::wasm::link_imports_plugin_exports(
        &mut linker,
        &wasmtime_component,
        plugin_manager,
    )
    .context("failed to link plugin exports to component imports")?;

    // Pre-instantiate the component
    let instance_pre = linker
        .instantiate_pre(&wasmtime_component)
        .context("failed to pre-instantiate component")?;

    // Create a WorkloadHandle for the dev component
    Ok(runtime::WorkloadHandle::new(
        engine.clone(),
        instance_pre,
        linker,
        vec![], // No volume mounts for dev components by default
    ))
}

// TODO: Re-implement tests with local runtime
// #[cfg(test)]
// mod test {
//     // All tests commented out until we implement local runtime equivalents
// }
