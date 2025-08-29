//! # Usage
// TODO: doctest, pull from test

pub mod engine;
pub mod host;
pub mod plugin;
mod wit;
mod workload;
mod workload_handle;

// Public exports for external use
pub use engine::{Engine, EngineBuilder};
pub use host::{Host, HostApi, HostBuilder};
pub use plugin::Plugin;
pub use wit::WitInterface;
pub use workload::*;
pub use workload_handle::WorkloadHandle;

// service wasi:cli/run
// workload-a and workload-b both export foo:bar/interface
// in a workload, the world must be resolveable.
// The service is a part of the world resolution
// component exports mycompany-handler. Service can import that.
// components can't call services.

// Services can open their own ports, components in that workload can call those ports by using localhost.
// Component wasi:sockets connect through this localhost.

// Services can go through another component (or a resource) to communicate with components

// get_host_by_name
// fetch the IP address for a host, stash it
// connecting to that IP address is allowed.

// volumes on the workload
// mounts on component
// share same dirs.

// #[cfg(feature = "wrpc")]
// mod wrpc {
//     use std::collections::HashMap;

//     use wasmtime_wasi::IoView;

//     use crate::{
//         Plugin, WitInterface,
//         engine::{Ctx, CtxView},
//     };

//     pub struct WrpcPlugin;

//     pub struct WrpcCtx {
//         inner: Ctx,
//         /// Map from target to the WIT interfaces that it supports. This is used to
//         /// determine which interfaces need to be polyfilled.
//         interfaces_to_polyfill: HashMap<String, Vec<WitInterface>>,
//     }
//     impl IoView for WrpcCtx {
//         fn table(&mut self) -> &mut wasmtime::component::ResourceTable {
//             &mut self.inner.table
//         }
//     }
//     impl CtxView for WrpcCtx {
//         fn ctx(&mut self) -> &mut Ctx {
//             &mut self.inner
//         }
//     }

//     impl Plugin for WrpcPlugin {
//         type Ctx = WrpcCtx;

//         fn new_context(&self, _id: &String, ctx: Ctx) -> anyhow::Result<Box<dyn CtxView>> {
//             Ok(Box::new(WrpcCtx {
//                 inner: ctx,
//                 interfaces_to_polyfill: HashMap::new(),
//             }))
//         }

//         fn add_to_linker(
//             &self,
//             _id: &String,
//             linker: &mut wasmtime::component::Linker<Self::Ctx>,
//         ) -> anyhow::Result<std::pin::Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>>>
//         {
//             Ok(Box::pin(async move { Ok(()) }))
//         }
//     }
// }

// Tests should only be run with all features enabled, there isn't proper feature gating _yet_
#[cfg(test)]
mod test {
    use crate::plugin::http_server::HttpServer;
    use crate::plugin::runtime_config::RuntimeConfig;
    use crate::{
        host::{HostApi, WorkloadStartRequest},
        workload::Workload,
    };
    use std::collections::HashMap;

    use super::{engine::Engine, host::HostBuilder};

    #[tokio::test]
    #[cfg(feature = "http")]
    async fn can_run_engine() -> anyhow::Result<()> {
        use std::sync::Arc;

        let engine = Engine::builder().build()?;
        let http_plugin = HttpServer::new("127.0.0.1:8080".parse()?);
        let runtime_config_plugin = RuntimeConfig::default();

        let mut host = HostBuilder::new()
            .with_engine(engine)
            .with_plugin("http".to_string(), Arc::new(http_plugin))
            .with_plugin("config".to_string(), Arc::new(runtime_config_plugin))
            .build()?;

        // Start plugins and host
        if let Err(e) = host.start().await {
            tracing::error!(err = ?e, "failed to start host");
        }

        let req = WorkloadStartRequest {
            workload: Workload {
                namespace: "test".to_string(),
                name: "test-workload".to_string(),
                annotations: HashMap::new(),
                service: Some(crate::workload::Service {
                    // TODO: Pull from file, integration test.
                    bytes: bytes::Bytes::from_static(b"whee component"),
                    local_resources: crate::workload::LocalResources {
                        memory_limit_mb: 256,
                        cpu_limit: 1,
                        config: HashMap::new(),
                        volume_mounts: vec![],
                        allowed_hosts: vec![],
                    },
                    max_restarts: 3,
                }),
                wit_world: Some(crate::workload::WitWorld {
                    components: vec![],
                    host_interfaces: vec![],
                }),
                volumes: vec![],
            },
        };
        let res = host.workload_start(req).await?;

        Ok(())
    }
}
