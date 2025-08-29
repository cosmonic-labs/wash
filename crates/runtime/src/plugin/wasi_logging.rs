use anyhow::bail;

use crate::{
    Plugin, WorkloadHandle, engine::Ctx,
    plugin::wasi_logging::bindings::wasi::logging::logging::Level,
};

mod bindings {
    wasmtime::component::bindgen!({
        world: "logging",
        trappable_imports: true,
        async: true,
    });
}

pub struct WasiLogging;

impl bindings::wasi::logging::logging::Host for Ctx {
    async fn log(&mut self, level: Level, context: String, message: String) -> anyhow::Result<()> {
        match level {
            Level::Critical => tracing::error!(id = &self.id, context, "{message}"),
            Level::Error => tracing::error!(id = &self.id, context, "{message}"),
            Level::Warn => tracing::warn!(id = &self.id, context, "{message}"),
            Level::Info => tracing::info!(id = &self.id, context, "{message}"),
            Level::Debug => tracing::debug!(id = &self.id, context, "{message}"),
            Level::Trace => tracing::trace!(id = &self.id, context, "{message}"),
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl Plugin for WasiLogging {
    async fn bind_workload(
        &self,
        _id: &String,
        mut workload_handle: WorkloadHandle,
        interfaces: std::collections::HashSet<crate::wit::WitInterface>,
    ) -> anyhow::Result<()> {
        // Ensure exactly one interface: "wasi:logging/logging"
        let mut iter = interfaces.iter();
        let Some(interface) = iter.next() else {
            bail!("No interfaces provided; expected wasi:logging/logging");
        };
        if iter.next().is_some()
            || interface.namespace != "wasi"
            || interface.package != "logging"
            || !interface.interfaces.contains(&"logging".to_string())
        {
            bail!(
                "Expected exactly one interface: wasi:logging/logging, got: {:?}",
                interfaces
            );
        }

        // Add `wasi:logging/logging` to the workload's linker
        bindings::wasi::logging::logging::add_to_linker(workload_handle.linker(), |ctx| ctx)?;

        Ok(())
    }
}
