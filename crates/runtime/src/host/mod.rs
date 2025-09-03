use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::Arc;

use anyhow::{Context, bail};
use names::{Generator, Name};
use tokio::sync::RwLock;

use crate::engine::Engine;
use crate::plugin::Plugin;
use crate::wit::WitWorld;
use crate::workload::{Workload, WorkloadState};

mod sysinfo;
use sysinfo::SystemMonitor;
mod types;
pub use types::*;

/// The API for interacting with a host
pub trait HostApi {
    /// Request a [`HostHeartbeat`] from the host
    fn heartbeat(&self) -> impl Future<Output = anyhow::Result<HostHeartbeat>>;
    fn workload_start(
        &self,
        request: WorkloadStartRequest,
    ) -> impl Future<Output = anyhow::Result<WorkloadStartResponse>>;
    fn workload_status(
        &self,
        request: WorkloadStatusRequest,
    ) -> impl Future<Output = anyhow::Result<WorkloadStatusResponse>>;
    fn workload_stop(
        &self,
        request: WorkloadStopRequest,
    ) -> impl Future<Output = anyhow::Result<WorkloadStopResponse>>;
}

// Helper trait impl that helps with Arc-ing the Host
impl<T: HostApi> HostApi for Arc<T> {
    async fn heartbeat(&self) -> anyhow::Result<HostHeartbeat> {
        self.as_ref().heartbeat().await
    }
    async fn workload_start(
        &self,
        request: WorkloadStartRequest,
    ) -> anyhow::Result<WorkloadStartResponse> {
        self.as_ref().workload_start(request).await
    }
    async fn workload_stop(
        &self,
        request: WorkloadStopRequest,
    ) -> anyhow::Result<WorkloadStopResponse> {
        self.as_ref().workload_stop(request).await
    }
    async fn workload_status(
        &self,
        request: WorkloadStatusRequest,
    ) -> anyhow::Result<WorkloadStatusResponse> {
        self.as_ref().workload_status(request).await
    }
}

pub struct Host {
    engine: Engine,
    /// Workloads mapped from ID to the workload and its current state
    workloads: Arc<RwLock<HashMap<String, (Workload, WorkloadState)>>>,
    /// Plugins in a map from their ID to the plugin itself
    plugins: HashMap<String, Arc<dyn Plugin>>,
    /// Host metadata
    id: String,
    hostname: String,
    friendly_name: String,
    version: String,
    labels: HashMap<String, String>,
    started_at: chrono::DateTime<chrono::Utc>,
    /// System monitor for tracking CPU/memory usage
    system_monitor: Arc<RwLock<SystemMonitor>>,
    // endpoints: HashMap<String, EndpointConfiguration>
}

impl Host {
    /// Start the host, performing initialization steps like starting plugins
    pub async fn start(self) -> anyhow::Result<Arc<Self>> {
        // Start all plugins, any errors means the host fails to start.
        for (id, plugin) in &self.plugins {
            if let Err(e) = plugin.start().await {
                tracing::error!(id = id, err = ?e, "failed to start plugin");
                bail!(e)
            }
        }

        Ok(Arc::new(self))
    }

    pub async fn stop(self: Arc<Self>) -> anyhow::Result<()> {
        // Stop all plugins, log errors but continue stopping others
        for (id, plugin) in &self.plugins {
            let stop_fut = plugin.stop();
            match tokio::time::timeout(std::time::Duration::from_secs(3), stop_fut).await {
                Ok(Err(e)) => {
                    tracing::error!(id = id, err = ?e, "failed to stop plugin");
                }
                Err(_) => {
                    tracing::error!(id = id, "plugin stop timed out after 3 seconds");
                }
                _ => {}
            }
        }

        Ok(())
    }

    pub fn label(&self, label: impl AsRef<str>) -> Option<&String> {
        self.labels.get(label.as_ref())
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn friendly_name(&self) -> &str {
        &self.friendly_name
    }

    /// Helper function to generate a unique ID for a workload
    fn generate_workload_id(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }

    /// Returns the WIT (imports, exports) that this host can provide to any component.
    ///
    /// Put another way, this represents a simplified version of the host world. For
    /// example, this WIT world:
    /// ```wit
    /// package wasmcloud:host@0.1.0;
    ///
    /// interface foo {
    /// ...
    /// }
    /// interface bar {
    /// ...
    /// }
    ///
    /// world host {
    ///   import foo;
    ///   export bar;
    /// }
    /// ```
    ///
    /// Would be returned as:
    /// (
    ///  vec![WitInterface { namespace: "wasmcloud", package: "host", interfaces: ["foo"], version: Some("0.1.0") }],
    ///  vec![WitInterface { namespace: "wasmcloud", package: "host", interfaces: ["bar"], version: Some("0.1.0") }],
    /// )
    ///
    /// This can be viewed as an inversion of the worlds that this host can support. In the above example,
    /// this host can support any component that imports `bar` and exports `foo`. Other exports will be ignored,
    /// and other imports that are unsatisfied will be rejected.
    pub fn wit_world(&self) -> WitWorld {
        let mut imports = HashSet::new();
        // The host provides wasi@0.2 interfaces other than wasi:http
        // <https://docs.rs/wasmtime-wasi/36.0.2/wasmtime_wasi/p2/index.html#wasip2-interfaces>
        let mut exports = HashSet::from([
            "wasi:io/poll,error,streams@0.2.0".into(),
            "wasi:clocks/monotonic-clock,wall-time@0.2.0".into(),
            "wasi:random/random@0.2.0".into(),
            "wasi:cli/environment,exit,stderr,stdin,stdout,terminal-input,terminal-output,terminal-stderr,terminal-stdin,terminal-stdout@0.2.0".into(),
            "wasi:clocks/monotonic-clock,wall-clock@0.2.0".into(),
            "wasi:filesystem/preopens,types@0.2.0".into(),
            "wasi:random/insecure-seed,insecure,random@0.2.0".into(),
            "wasi:sockets/instance-network,ip-name-lookup,network,tcp-create-socket,tcp,udp-create-socket,udp@0.2.0".into(),
        ]);

        // Include imports and exports that plugins specify
        imports.extend(
            self.plugins
                .values()
                .flat_map(|p| p.world().imports.into_iter().collect::<Vec<_>>()),
        );
        exports.extend(
            self.plugins
                .values()
                .flat_map(|p| p.world().exports.into_iter().collect::<Vec<_>>()),
        );

        WitWorld { imports, exports }
    }

    /// Returns a three-tuple of (OS architecture, OS name, OS kernel)
    async fn get_system_info(&self) -> (String, String, String) {
        // Get OS information
        let os_name = std::env::consts::OS.to_string();
        let os_arch = std::env::consts::ARCH.to_string();
        let os_kernel = std::env::consts::FAMILY.to_string();
        (os_arch, os_name, os_kernel)
    }

    /// Returns a tuple of (total memory, free memory)
    async fn get_memory_info(&self) -> anyhow::Result<(u64, u64)> {
        let monitor = self.system_monitor.read().await;
        let mem = monitor.memory_usage();
        Ok((mem.total_memory, mem.free_memory))
    }

    /// Returns the current global CPU usage as a percentage
    async fn get_cpu_usage(&self) -> anyhow::Result<f32> {
        let monitor = self.system_monitor.read().await;
        Ok(monitor.cpu_usage().global_usage)
    }
}

impl HostApi for Host {
    async fn heartbeat(&self) -> anyhow::Result<HostHeartbeat> {
        // Refresh system info before reporting
        {
            let mut monitor = self.system_monitor.write().await;
            monitor.refresh();
            monitor.report_usage();
        }

        let (os_arch, os_name, os_kernel) = self.get_system_info().await;
        let (system_memory_total, system_memory_free) = self
            .get_memory_info()
            .await
            .context("failed to get memory info")?;
        let system_cpu_usage = self
            .get_cpu_usage()
            .await
            .context("failed to get CPU usage")?;

        // Count components and providers from workloads
        let component_count: u64 = self
            .workloads
            .read()
            .await
            .values()
            // TODO: Include services?
            .map(|(w, _)| w.components.len() as u64)
            .sum();

        // TODO: Properly track providers once we have provider support
        let provider_count = 0;

        // Collect all imports and exports from the host and plugins
        let mut imports = Vec::new();
        let exports = Vec::new();

        for (workload, state) in self.workloads.read().await.values() {
            if *state == WorkloadState::Running {
                // Add host interfaces as imports
                imports.extend(workload.host_interfaces.clone());
                // TODO: Add component exports when we track them
            }
        }

        Ok(HostHeartbeat {
            id: self.id.clone(),
            hostname: self.hostname.clone(),
            friendly_name: self.friendly_name.clone(),
            version: self.version.clone(),
            labels: self.labels.clone(),
            started_at: self.started_at,
            os_arch,
            os_name,
            os_kernel,
            system_cpu_usage,
            system_memory_total,
            system_memory_free,
            component_count,
            provider_count,
            imports,
            exports,
        })
    }

    async fn workload_start(
        &self,
        request: WorkloadStartRequest,
    ) -> anyhow::Result<WorkloadStartResponse> {
        let Workload {
            namespace: _,
            name: _,
            annotations: _,
            service: _,
            components: _,
            host_interfaces,
            volumes: _,
        } = &request.workload;

        let workload_id = self.generate_workload_id();

        // Store the workload with initial state
        self.workloads.write().await.insert(
            workload_id.clone(),
            (request.workload.clone(), WorkloadState::Starting),
        );

        // Start the workload using the engine
        let (_service, workload_handles) = self.engine.start_workload(request.workload.clone())?;

        // Bind plugins to all workload handles
        for (component_idx, workload_handle) in workload_handles.iter().enumerate() {
            tracing::debug!("Binding plugins for component {}", component_idx);

            for ww in host_interfaces {
                tracing::info!(interface = ?ww, component = component_idx, "Checking interface for plugin binding");
                for (id, p) in &self.plugins {
                    let plugin_interfaces = p.world();
                    tracing::debug!(plugin_id = id, plugin_interfaces = ?plugin_interfaces, "Checking plugin interfaces");

                    // TODO: Might need to be directional
                    // Check if plugin supports this interface (ignoring config which is binding-specific)
                    let interface_match = plugin_interfaces.imports.iter().any(|pi| {
                        pi.namespace == ww.namespace
                            && pi.package == ww.package
                            && pi.interfaces == ww.interfaces
                            && pi.version == ww.version
                    }) || plugin_interfaces.exports.iter().any(|pi| {
                        pi.namespace == ww.namespace
                            && pi.package == ww.package
                            && pi.interfaces == ww.interfaces
                            && pi.version == ww.version
                    });
                    if interface_match {
                        tracing::info!(
                            "binding plugin {} to workload component {}",
                            id,
                            component_idx
                        );
                        // Create a unique workload ID for each component
                        let component_workload_id = format!("{}_{}", workload_id, component_idx);
                        if let Err(e) = p
                            .bind_workload(
                                &component_workload_id,
                                workload_handle.clone(),
                                HashSet::from([ww.clone()]),
                            )
                            .await
                        {
                            tracing::error!(plugin_id = id, component = component_idx, err = ?e, "failed to bind workload to plugin");
                        } else {
                            tracing::info!(
                                plugin_id = id,
                                component = component_idx,
                                "Successfully bound workload to plugin"
                            );
                        }
                    }
                }
            }
        }

        // 3. Link components together where possible

        // 3a. If more unresolved interfaces, bail

        // 4. Starting execution

        // Store the workload with initial state
        self.workloads.write().await.insert(
            workload_id.clone(),
            (request.workload.clone(), WorkloadState::Running),
        );
        Ok(WorkloadStartResponse {
            workload_status: WorkloadStatus {
                workload_id,
                workload_state: WorkloadState::Running,
                message: "Workload is running".to_string(),
            },
        })
    }

    async fn workload_status(
        &self,
        request: WorkloadStatusRequest,
    ) -> anyhow::Result<WorkloadStatusResponse> {
        if let Some((_, state)) = self.workloads.read().await.get(&request.workload_id) {
            Ok(WorkloadStatusResponse {
                workload_status: WorkloadStatus {
                    workload_id: request.workload_id,
                    workload_state: *state,
                    message: format!("Workload is {:?}", state),
                },
            })
        } else {
            anyhow::bail!("Workload not found: {}", request.workload_id)
        }
    }

    async fn workload_stop(
        &self,
        request: WorkloadStopRequest,
    ) -> anyhow::Result<WorkloadStopResponse> {
        let (workload_state, message) = if self
            .workloads
            .read()
            .await
            .contains_key(&request.workload_id)
        {
            // Update state to stopping
            // if let Some((_, state)) = self.workloads.get_mut(&request.workload_id) {
            //     *state = WorkloadState::Stopping;
            // }

            // TODO: Actually stop the workload
            // This would involve:
            // 1. Sending stop signal to the service
            // 2. Cleaning up resources
            // 3. Removing from active workloads

            // For now, simulate stopping
            self.workloads.write().await.remove(&request.workload_id);

            (
                WorkloadState::Stopping,
                "Workload stopped successfully".to_string(),
            )
        } else {
            (WorkloadState::Unspecified, "Workload not found".to_string())
        };

        Ok(WorkloadStopResponse {
            workload_status: WorkloadStatus {
                workload_id: request.workload_id,
                workload_state,
                message,
            },
        })
    }
}

/// Builder for the [`Host`]
pub struct HostBuilder {
    engine: Option<Engine>,
    plugins: HashMap<String, Arc<dyn Plugin>>,
    hostname: Option<String>,
    friendly_name: Option<String>,
    labels: HashMap<String, String>,
}

impl Default for HostBuilder {
    fn default() -> Self {
        Self {
            engine: None,
            plugins: HashMap::new(),
            hostname: None,
            friendly_name: None,
            labels: HashMap::new(),
        }
    }
}

impl HostBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_engine(mut self, engine: Engine) -> Self {
        self.engine = Some(engine);
        self
    }

    pub fn with_plugin(mut self, id: impl AsRef<str>, plugin: Arc<dyn Plugin>) -> Self {
        self.plugins.insert(id.as_ref().to_string(), plugin);
        self
    }

    pub fn with_hostname(mut self, hostname: impl AsRef<str>) -> Self {
        self.hostname = Some(hostname.as_ref().to_string());
        self
    }

    pub fn with_friendly_name(mut self, name: impl AsRef<str>) -> Self {
        self.friendly_name = Some(name.as_ref().to_string());
        self
    }

    pub fn with_label(mut self, key: impl AsRef<str>, value: impl AsRef<str>) -> Self {
        self.labels
            .insert(key.as_ref().to_string(), value.as_ref().to_string());
        self
    }

    pub fn build(self) -> anyhow::Result<Host> {
        let engine = if let Some(engine) = self.engine {
            engine
        } else {
            Engine::builder().build()?
        };

        // Get hostname from system if not provided
        let hostname = self.hostname.unwrap_or_else(|| {
            hostname::get()
                .map(|h| h.to_string_lossy().to_string())
                .unwrap_or_else(|_| "unknown".to_string())
        });

        // Generate a friendly name if not provided
        let friendly_name = self.friendly_name.unwrap_or_else(|| {
            let mut generator = Generator::with_naming(Name::Numbered);
            generator
                .next()
                .unwrap_or_else(|| format!("host-{}", uuid::Uuid::new_v4()))
        });

        Ok(Host {
            engine,
            workloads: Arc::default(),
            plugins: self.plugins,
            id: uuid::Uuid::new_v4().to_string(),
            hostname,
            friendly_name,
            version: env!("CARGO_PKG_VERSION").to_string(),
            labels: self.labels,
            started_at: chrono::Utc::now(),
            system_monitor: Arc::new(RwLock::new(SystemMonitor::new())),
        })
    }
}

// Manual Debug implementation for Host since Plugin trait objects can't derive Debug
impl std::fmt::Debug for Host {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Host")
            .field("engine", &self.engine)
            .field("workloads", &self.workloads)
            .field("plugins", &format_args!("<{} plugins>", self.plugins.len()))
            .field("id", &self.id)
            .field("hostname", &self.hostname)
            .field("friendly_name", &self.friendly_name)
            .field("version", &self.version)
            .field("labels", &self.labels)
            .field("started_at", &self.started_at)
            .field("system_monitor", &"<SystemMonitor>")
            .finish()
    }
}
