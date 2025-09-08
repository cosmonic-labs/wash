use crate::{UnresolvedWorkloadHandle, WorkloadHandle, wit::WitWorld};

/// The `wasi:http/incoming-handler@0.2.0` server plugin
#[cfg(feature = "http")]
pub mod http_server;
/// The `wasi:config/runtime@0.2.0-draft` runtime configuration plugin
#[cfg(feature = "runtime-config")]
pub mod runtime_config;
/// The `wasi:blobstore@0.2.0-draft` in-memory blobstore plugin
#[cfg(feature = "wasi-blobstore")]
pub mod wasi_blobstore;
/// The `wasi:keyvalue@0.2.0-draft` in-memory keyvalue plugin
#[cfg(feature = "wasi-keyvalue")]
pub mod wasi_keyvalue;
/// The `wasi:logging/logging@0.1.0-draft` plugin
#[cfg(feature = "wasi-logging")]
pub mod wasi_logging;
// TODO: wasmcloud_messaging/wasi_messaging
// TODO: wasmcloud_bus?

// TODO: no async trait?
#[async_trait::async_trait]
pub trait Plugin: std::any::Any + Send + Sync + 'static {
    /// Unique identifier for this plugin type. Must be unique across all plugins.
    fn id(&self) -> &'static str;
    /// Returns the WIT interfaces that this plugin exposes. This plugin's [`Plugin::bind_workload`] function
    /// will only be invoked if the workload is using one of these interfaces.
    fn world(&self) -> WitWorld;

    /// Invoked when the plugin is started, perform any necessary pre-initialization steps
    /// Includes the id of the plugin which can be used to retrieve plugin data from [`crate::engine::Ctx::get_plugin`]
    async fn start(&self) -> anyhow::Result<()> {
        Ok(())
    }

    /// Invoked when a workload binds to this plugin.
    /// ## Arguments
    /// - `id`: The ID of the workload
    /// - `workload_handle`: Handle to the workload that provides access to linker modification
    /// - `interfaces`: The WIT interfaces that the workload is expecting this plugin to implement.
    ///
    /// ## Returns
    /// The configuration map for the workload to implement this plugin. Can be empty.
    async fn bind_workload(
        &self,
        _id: &str,
        _workload_handle: &mut UnresolvedWorkloadHandle,
        _interfaces: std::collections::HashSet<crate::wit::WitInterface>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    /// Called after all plugins have bound to the workload linker.
    /// Only plugins that need the fully resolved handle should implement this.
    /// The same ID from bind_workload is passed here.
    async fn on_workload_resolved(
        &self,
        _id: &str,
        _resolved_handle: &WorkloadHandle,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    /// Invoked when a workload is stopped or unbound from this plugin
    async fn unbind_workload(
        &self,
        _id: &str,
        _workload_handle: WorkloadHandle,
        _interfaces: std::collections::HashSet<crate::wit::WitInterface>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    /// Perform any necessary cleanup before stopping the plugin.
    async fn stop(&self) -> anyhow::Result<()> {
        Ok(())
    }
}
