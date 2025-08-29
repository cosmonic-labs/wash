use crate::{WorkloadHandle, wit::WitWorld};

/// The `wasi:http/incoming-handler@0.2.0` server plugin
#[cfg(feature = "http")]
pub mod http_server;
/// The `wasi:config/runtime@0.2.0-draft` runtime configuration plugin
#[cfg(feature = "runtime-config")]
pub mod runtime_config;
/// The `wasi:logging/logging@0.1.0-draft` plugin
#[cfg(feature = "wasi-logging")]
pub mod wasi_logging;

#[async_trait::async_trait]
pub trait Plugin: Send + Sync + 'static {
    /// Returns the WIT interfaces that this plugin exposes. This plugin's [`Plugin::bind_workload`] function
    /// will only be invoked if the workload is using one of these interfaces.
    fn world(&self) -> WitWorld {
        WitWorld::default()
    }

    /// Invoked when the plugin is started, perform any necessary pre-initialization steps
    async fn start(&self) -> anyhow::Result<()> {
        Ok(())
    }

    /// Invoked when a workload binds to this plugin.
    /// ## Arguments
    /// - `id`: The ID of the workload
    /// - `workload_handle`: Handle to the workload that provides access to instance pre and store creation
    /// - `interfaces`: The WIT interfaces that the workload is expecting this plugin to implement.
    ///
    /// ## Returns
    /// The configuration map for the workload to implement this plugin. Can be empty.
    async fn bind_workload(
        &self,
        _id: &String,
        mut _workload_handle: WorkloadHandle,
        _interfaces: std::collections::HashSet<crate::wit::WitInterface>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    /// Invoked when a workload is stopped or unbound from this plugin
    async fn unbind_workload(
        &self,
        _id: &String,
        mut _workload_handle: WorkloadHandle,
        _interfaces: std::collections::HashSet<crate::wit::WitInterface>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    /// Perform any necessary cleanup before stopping the plugin.
    async fn stop(&self) -> anyhow::Result<()> {
        Ok(())
    }
}
