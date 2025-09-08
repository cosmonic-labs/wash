use std::collections::{HashMap, HashSet};

use anyhow::bail;

const WASI_LOGGING_ID: &str = "wasi-logging";

use crate::{
    Plugin, UnresolvedWorkloadHandle, WitInterface, engine::Ctx,
    plugin::wasi_logging::bindings::wasi::logging::logging::Level, wit::WitWorld,
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
    fn id(&self) -> &'static str {
        WASI_LOGGING_ID
    }

    fn world(&self) -> WitWorld {
        WitWorld {
            imports: HashSet::from([WitInterface {
                namespace: "wasi".to_string(),
                package: "logging".to_string(),
                interfaces: vec!["logging".to_string()],
                version: Some(semver::Version::parse("0.1.0-draft").unwrap()),
                config: HashMap::default(),
            }]),
            exports: HashSet::default(),
        }
    }

    async fn bind_workload(
        &self,
        _id: &str,
        workload_handle: &mut UnresolvedWorkloadHandle,
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
