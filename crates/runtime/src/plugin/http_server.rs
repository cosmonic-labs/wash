use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use crate::wit::WitWorld;
use crate::{Plugin, WorkloadHandle, engine::Ctx};
use anyhow::{Context, bail};
use hyper::server::conn::http1;
use tokio::net::TcpListener;
use tracing::{debug, error, info, warn};
use wasmtime::component::InstancePre;
use wasmtime::{AsContextMut, StoreContextMut};
use wasmtime_wasi_http::{
    WasiHttpView,
    bindings::{ProxyPre, http::types::Scheme},
    body::HyperOutgoingBody,
    io::TokioIo,
};

use tokio::sync::{RwLock, mpsc};

pub struct HttpServer {
    addr: SocketAddr,
    // Map from host header to workload handles
    workload_handles: Arc<RwLock<HashMap<String, WorkloadHandle>>>,
    shutdown_tx: Arc<RwLock<Option<mpsc::Sender<()>>>>,
}

impl HttpServer {
    pub fn new(addr: SocketAddr) -> Self {
        Self {
            addr,
            workload_handles: Arc::default(),
            shutdown_tx: Arc::new(RwLock::new(None)),
        }
    }
}

#[async_trait::async_trait]
impl Plugin for HttpServer {
    fn world(&self) -> WitWorld {
        let mut interfaces = std::collections::HashSet::new();
        interfaces.insert(crate::wit::WitInterface {
            namespace: "wasi".to_string(),
            package: "http".to_string(),
            interfaces: vec!["incoming-handler".to_string()],
            // TODO: multiple versions supported? 0.2.X
            version: Some(semver::Version::parse("0.2.0").expect("to parse HTTP server version")),
            config: HashMap::new(),
        });
        WitWorld {
            imports: interfaces,
            ..Default::default()
        }
    }

    async fn start(&self) -> anyhow::Result<()> {
        let addr = self.addr.clone();
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        let shutdown_tx_clone = self.shutdown_tx.clone();
        let workload_handles = self.workload_handles.clone();

        // Store the shutdown sender
        *shutdown_tx_clone.write().await = Some(shutdown_tx);

        // Start the HTTP server, any incoming requests call Host::handle and then it's routed
        // to the workload based on host header.
        tokio::spawn(async move {
            if let Err(e) = run_http_server(addr, workload_handles, &mut shutdown_rx).await {
                error!(err = ?e, addr = ?addr, "HTTP server error");
            }
        });

        info!(addr = ?addr, "HTTP server starting");
        Ok(())
    }

    async fn bind_workload(
        &self,
        id: &String,
        workload_handle: WorkloadHandle,
        interfaces: std::collections::HashSet<crate::wit::WitInterface>,
    ) -> anyhow::Result<()> {
        let Some(http_iface) = interfaces.iter().find(|iface| {
            iface.namespace == "wasi"
                && iface.package == "http"
                && iface.interfaces.contains(&"incoming-handler".to_string())
        }) else {
            bail!(
                "No wasi:http/incoming-handler interface found, plugin should not be bound to this workload"
            );
        };

        // Warnings for extra specified interfaces
        if interfaces.len() > 1 {
            warn!(
                interfaces = ?interfaces,
                "ignoring non-wasi:http/incoming-handler interfaces",
            );
        } else if http_iface.interfaces.len() > 1 {
            warn!(
                interfaces = ?http_iface.interfaces,
                "ignoring non-incoming-handler interfaces",
            );
        }

        let Some(host_header) = http_iface.config.get("host") else {
            bail!("no host header found, unable to bind to workload");
        };

        info!(host = %host_header, workload_id = id, "binding host header to workload");

        // NOTE: There is no `add_to_linker` call here because it's already added when initializing
        // the Ctx, as long as the `http` feature is enabled. This is totally possible to do here, but it would
        // mean re-implementing wasmtime_wasi_http.

        // Store the workload handle for this host header
        self.workload_handles
            .write()
            .await
            .insert(host_header.clone(), workload_handle);

        Ok(())
    }

    async fn stop(&self) -> anyhow::Result<()> {
        info!(addr = ?self.addr, "HTTP server stopping");
        // Stop the HTTP server
        let mut shutdown_guard = self.shutdown_tx.write().await;
        if let Some(tx) = shutdown_guard.take() {
            let _ = tx.send(()).await;
        }
        Ok(())
    }
}

/// HTTP server implementation that routes to workload components
async fn run_http_server(
    addr: SocketAddr,
    workload_handles: Arc<RwLock<HashMap<String, WorkloadHandle>>>,
    shutdown_rx: &mut mpsc::Receiver<()>,
) -> anyhow::Result<()> {
    let listener = TcpListener::bind(addr).await?;
    info!(addr = ?addr, "HTTP server listening");

    loop {
        tokio::select! {
            // Handle shutdown signal
            _ = shutdown_rx.recv() => {
                info!("HTTP server received shutdown signal");
                break;
            }
            // Accept new connections
            result = listener.accept() => {
                match result {
                    Ok((client, client_addr)) => {
                        debug!(addr = ?client_addr, "new HTTP client connection");

                        let handles_clone = workload_handles.clone();
                        tokio::spawn(async move {
                            if let Err(e) = http1::Builder::new()
                                .keep_alive(true)
                                .serve_connection(
                                    TokioIo::new(client),
                                    hyper::service::service_fn(move |req| {
                                        let handles = handles_clone.clone();
                                        async move {
                                            handle_http_request(req, handles).await
                                        }
                                    }),
                                )
                                .await
                            {
                                error!(addr = ?client_addr, err = ?e, "error serving HTTP client");
                            }
                        });
                    }
                    Err(e) => {
                        error!(err = ?e, "failed to accept HTTP connection");
                    }
                }
            }
        }
    }

    Ok(())
}

/// Handle individual HTTP requests by looking up workload and invoking component
async fn handle_http_request(
    req: hyper::Request<hyper::body::Incoming>,
    workload_handles: Arc<RwLock<HashMap<String, WorkloadHandle>>>,
) -> Result<hyper::Response<HyperOutgoingBody>, hyper::Error> {
    let method = req.method().clone();
    let uri = req.uri().clone();

    // Extract the Host header
    let host_header = req
        .headers()
        .get("host")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("<no host header>")
        .to_string(); // Convert to String to avoid borrow issues

    info!(
        method = %method,
        uri = %uri,
        host = %host_header,
        "HTTP request received"
    );

    // Look up workload handle for this host
    let workload_handle = {
        let handles = workload_handles.read().await;
        debug!(host = %host_header, "looking up workload handle for host header");
        handles.get(&host_header).cloned()
    };

    let response = match workload_handle {
        Some(handle) => match invoke_component_handler(handle, req).await {
            Ok(resp) => resp,
            Err(e) => {
                error!(err = ?e, host = %host_header, "Failed to invoke component");
                // TODO: add in error
                hyper::Response::builder()
                    .status(500)
                    .body(HyperOutgoingBody::default())
                    .unwrap()
            }
        },
        None => {
            warn!(host = %host_header, "No workload bound to host header");
            hyper::Response::builder()
                .status(404)
                .body(HyperOutgoingBody::default())
                .unwrap()
        }
    };

    Ok(response)
}

/// Invoke the component handler for the given workload
async fn invoke_component_handler(
    workload_handle: WorkloadHandle,
    req: hyper::Request<hyper::body::Incoming>,
) -> anyhow::Result<hyper::Response<HyperOutgoingBody>> {
    // Create a new store for this request
    let mut store = workload_handle.new_store();

    // Get the pre-instantiated component
    let instance_pre = workload_handle.instance_pre();

    // Use the same implementation as dev.rs
    handle_component_request(store.as_context_mut(), instance_pre, req).await
}

/// Handle a component request using WASI HTTP (copied from wash/crates/src/cli/dev.rs)
pub async fn handle_component_request<'a>(
    mut store: StoreContextMut<'a, Ctx>,
    pre: InstancePre<Ctx>,
    req: hyper::Request<hyper::body::Incoming>,
) -> anyhow::Result<hyper::Response<HyperOutgoingBody>> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let req = store.data_mut().new_incoming_request(Scheme::Http, req)?;
    let out = store.data_mut().new_response_outparam(sender)?;
    let pre = ProxyPre::new(pre).context("failed to instantiate proxy pre")?;

    // Run the http request itself by instantiating and calling the component
    let proxy = pre.instantiate_async(&mut store).await?;

    proxy
        .wasi_http_incoming_handler()
        .call_handle(&mut store, req, out)
        .await?;

    match receiver.await {
        // If the client calls `response-outparam::set` then one of these
        // methods will be called.
        Ok(Ok(resp)) => Ok(resp),
        Ok(Err(e)) => Err(e.into()),

        // Otherwise the `sender` will get dropped along with the `Store`
        // meaning that the oneshot will get disconnected
        Err(e) => {
            error!(err = ?e, "error receiving http response");
            Err(anyhow::anyhow!(
                "oneshot channel closed but no response was sent"
            ))
        }
    }
}
