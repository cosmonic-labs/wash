use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use tokio::sync::RwLock;

const RUNTIME_CONFIG_ID: &str = "runtime-config";

use crate::{Plugin, UnresolvedWorkloadHandle, WitInterface, engine::Ctx, wit::WitWorld};

mod bindings {
    wasmtime::component::bindgen!({
        world: "config",
        trappable_imports: true,
        async: true,
    });
}

use bindings::wasi::config::runtime::{ConfigError, Host};

#[derive(Clone, Default)]
pub struct RuntimeConfig {
    /// A map of configuration from workload id to key-value pairs
    config: Arc<RwLock<HashMap<String, HashMap<String, String>>>>,
}

impl Host for Ctx {
    async fn get(&mut self, key: String) -> anyhow::Result<Result<Option<String>, ConfigError>> {
        let Some(plugin) = self.get_plugin::<RuntimeConfig>(RUNTIME_CONFIG_ID) else {
            return Ok(Ok(None));
        };
        let config_guard = plugin.config.read().await;
        config_guard
            .get(&self.id)
            .and_then(|map| map.get(&key).cloned())
            .map_or(Ok(Ok(None)), |v| Ok(Ok(Some(v))))
    }

    async fn get_all(&mut self) -> anyhow::Result<Result<Vec<(String, String)>, ConfigError>> {
        let Some(plugin) = self.get_plugin::<RuntimeConfig>(RUNTIME_CONFIG_ID) else {
            return Ok(Ok(vec![]));
        };
        let config_guard = plugin.config.read().await;
        let entries = config_guard
            .get(&self.id)
            .map(|map| map.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();
        Ok(Ok(entries))
    }
}

#[async_trait::async_trait]
impl Plugin for RuntimeConfig {
    fn id(&self) -> &'static str {
        RUNTIME_CONFIG_ID
    }

    fn world(&self) -> WitWorld {
        WitWorld {
            imports: HashSet::from([WitInterface {
                namespace: "wasi".to_string(),
                package: "config".to_string(),
                interfaces: vec!["runtime".to_string()],
                version: Some(
                    semver::Version::parse("0.2.0-draft").expect("to parse runtime config version"),
                ),
                config: HashMap::new(),
            }]),
            exports: HashSet::new(),
        }
    }
    async fn bind_workload(
        &self,
        id: &str,
        workload_handle: &mut UnresolvedWorkloadHandle,
        interfaces: std::collections::HashSet<crate::wit::WitInterface>,
    ) -> anyhow::Result<()> {
        // Find the "wasi:config/runtime" interface, if present
        let Some(interface) = interfaces.iter().find(|i| {
            i.namespace == "wasi"
                && i.package == "config"
                && i.interfaces.contains(&"runtime".to_string())
        }) else {
            // Log a warning if the requested interfaces are not wasi:config/runtime
            tracing::warn!(
                "RuntimeConfig plugin requested for non-wasi:config/runtime interface(s): {:?}",
                interfaces
            );
            return Ok(());
        };

        // Add `wasi:config/runtime` to the workload's linker
        bindings::wasi::config::runtime::add_to_linker(workload_handle.linker(), |ctx| ctx)?;

        // Store the configuration for lookups later
        self.config
            .write()
            .await
            .insert(id.to_string(), interface.config.clone());

        Ok(())
    }
}
