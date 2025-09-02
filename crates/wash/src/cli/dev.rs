use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::{Context as _, ensure};
use clap::Args;
use etcetera::AppStrategy;
// use indicatif::{ProgressBar, ProgressStyle};
use notify::{
    Event as NotifyEvent, RecursiveMode, Watcher,
    event::{EventKind, ModifyKind},
};
use tokio::{select, sync::mpsc};
use tracing::{debug, error, info, trace, warn};
use runtime::{HostBuilder, Plugin, WitInterface, plugin::http_server::HttpServer};

use crate::{
    cli::{
        CliCommand, CliContext, CommandOutput,
        component_build::build_component,
        doctor::{ProjectContext, check_project_specific_tools, detect_project_context},
    },
    component_build::BuildConfig,
    config::{Config, load_config},
    dev::DevPluginManager,
    // plugin::list_plugins,
    runtime::{prepare_component_dev, new_engine},
};

// TODO: Remove when bindings module is implemented for local runtime
#[allow(dead_code)]
pub enum HookType {
    BeforeDev,
    AfterDev,
    DevRegister,
}

/// Helper function to check if a path should be ignored during file watching
/// to prevent artifact directories from triggering rebuilds
///
/// # Arguments
/// * `path` - The file path to check
/// * `canonical_project_root` - The canonicalized project root directory
/// * `ignore_paths` - Set of canonicalized paths that should be ignored
fn is_ignored(
    path: &Path,
    _canonical_project_root: &Path,
    ignore_paths: &HashSet<PathBuf>,
) -> bool {
    let canonical_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    ignore_paths.iter().any(|p| canonical_path.starts_with(p))
}

/// Build a set of paths that should be ignored during file watching
/// Uses project-type-specific defaults when available
///
/// # Arguments
/// * `canonical_project_root` - The canonicalized project root directory
/// * `artifact_path` - Optional path to the build artifact
/// * `project_context` - Detected project type for specific ignore patterns
///
/// # Returns
/// A set of canonicalized paths that should be ignored during file watching
fn build_ignore_set(
    canonical_project_root: &Path,
    project_context: &ProjectContext,
) -> HashSet<PathBuf> {
    let mut ignore_paths = HashSet::new();

    // Common directories for all project types
    let mut dirs_to_ignore = vec![".git"];

    // Add project-type-specific ignore patterns
    match project_context {
        ProjectContext::Rust { .. } => {
            dirs_to_ignore.extend_from_slice(&["target"]);
        }
        ProjectContext::TypeScript { .. } => {
            dirs_to_ignore.extend_from_slice(&["node_modules", "dist", "build", ".next", ".nuxt"]);
        }
        ProjectContext::Go { .. } => {
            dirs_to_ignore.extend_from_slice(&["bin", "pkg", "vendor"]);
        }
        ProjectContext::Mixed { detected_types } => {
            // Add ignore patterns for all detected project types
            if detected_types.iter().any(|t| t == "Rust") {
                dirs_to_ignore.extend_from_slice(&["target"]);
            }
            if detected_types
                .iter()
                .any(|t| t == "TypeScript" || t == "JavaScript")
            {
                dirs_to_ignore.extend_from_slice(&[
                    "node_modules",
                    "dist",
                    "build",
                    ".next",
                    ".nuxt",
                ]);
            }
            if detected_types.iter().any(|t| t == "Go") {
                dirs_to_ignore.extend_from_slice(&["bin", "pkg", "vendor"]);
            }
        }
        ProjectContext::General => {
            // For general context, include common patterns from all project types
            dirs_to_ignore.extend_from_slice(&[
                "target",
                "build",
                "dist",
                "node_modules",
                ".next",
                ".nuxt",
                "bin",
                "pkg",
                "vendor",
            ]);
        }
    }

    // Build canonical paths for ignore directories relative to canonical project root
    for dir in &dirs_to_ignore {
        let dir_path = canonical_project_root.join(dir);
        if let Ok(canonical) = dir_path.canonicalize() {
            ignore_paths.insert(canonical);
        } else {
            // Even if the directory doesn't exist yet, include the absolute path
            ignore_paths.insert(dir_path);
        }
    }

    ignore_paths
}

#[derive(Debug, Clone, Args)]
pub struct DevCommand {
    /// The path to the project directory
    #[clap(name = "project-dir", default_value = ".")]
    pub project_dir: PathBuf,

    /// The path to the built Wasm file to be used in development
    #[clap(long = "artifact-path")]
    pub artifact_path: Option<PathBuf>,

    /// The address on which the HTTP server will listen
    #[clap(long = "address", default_value = "0.0.0.0:8000")]
    pub address: String,

    /// Configuration values to use for `wasi:config/runtime` in the form of `key=value` pairs.
    #[clap(long = "runtime-config", value_delimiter = ',')]
    pub runtime_config: Vec<String>,

    /// The root directory for the blobstore to use for `wasi:blobstore/blobstore`. Defaults to a subfolder in the wash data directory.
    #[clap(long = "blobstore-root")]
    pub blobstore_root: Option<PathBuf>,
}

impl CliCommand for DevCommand {
    async fn handle(&self, ctx: &CliContext) -> anyhow::Result<CommandOutput> {
        info!(path = ?self.project_dir, "starting development session for project");

        let config = load_config(
            &ctx.config_path(),
            Some(self.project_dir.as_path()),
            // Override the artifact path with the one provided in the command line
            Some(Config {
                build: Some(BuildConfig {
                    artifact_path: self.artifact_path.clone(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        )
        .context("failed to load config for development")?;

        // Validate project directory
        ensure!(
            self.project_dir.exists(),
            "Project directory does not exist: {}",
            self.project_dir.display()
        );
        ensure!(
            self.project_dir.is_dir(),
            "Project path is not a directory: {}",
            self.project_dir.display()
        );

        // Check for required tools (e.g., wasmCloud, WIT)
        let project_context = detect_project_context(&self.project_dir)
            .await
            .context("failed to detect project context")?;
        let (issues, recommendations) = check_project_specific_tools(&project_context)
            .await
            .context("failed to check project specific tools")?;
        if !issues.is_empty() {
            for issue in issues {
                warn!(issue = issue, "project tool issue");
            }
        } else {
            debug!("no issues found with project tools");
        }
        if !recommendations.is_empty() {
            for recommendation in recommendations {
                warn!(
                    recommendation = recommendation,
                    "project tool recommendation"
                );
            }
        } else {
            debug!("no recommendations found for project tools");
        }

        let artifact_path = match build_component(&self.project_dir, ctx, &config).await {
            // Edge case where the build was successful, but the artifact path in the config is different
            // than the one returned by the build process.
            Ok(build_result)
                if config
                    .build
                    .as_ref()
                    .and_then(|b| b.artifact_path.as_ref())
                    .is_some_and(|p| p != &build_result.artifact_path) =>
            {
                warn!(path = ?build_result.artifact_path, "component built successfully, but artifact path in config is different");
                // Ensure the artifact path is set in the config
                build_result.artifact_path
            }
            // Use the build result artifact path if the config does not specify one
            Ok(build_result) => {
                debug!(path = ?build_result.artifact_path, "component built successfully, using as artifact path");
                build_result.artifact_path
            }
            Err(e) => {
                // TODO(#18): Support continuing, npm start works like that.
                error!("failed to build component, will not start dev session");
                error!("{e}");
                return Err(e);
            }
        };

        // Deploy to local runtime
        let wasm_bytes = tokio::fs::read(&artifact_path)
            .await
            .context("failed to read artifact file")?;

        // TODO: Re-enable when plugin system is adapted to local runtime
        // Call pre-hooks before starting dev session
        // let pre_context = HashMap::new(); // Empty context for pre-hooks
        // let pre_runtime_context = Arc::new(RwLock::new(pre_context));
        // match ctx
        //     .call_pre_hooks(pre_runtime_context, HookType::BeforeDev)
        //     .await
        // {
        //     Ok(_) => {}
        //     Err(e) => {
        //         error!("pre-hook execution failed, will not start dev session");
        //         error!("{e}");
        //         return Err(e);
        //     }
        // }

        let mut plugin_manager = DevPluginManager::default();
        // TODO: Re-enable when plugin system is adapted to local runtime
        let plugins: Vec<crate::plugin::PluginComponent> = vec![];
        // let plugins = match list_plugins(ctx.runtime(), ctx.data_dir()).await {
        //     Ok(plugins) => plugins
        //         .into_iter()
        //         .filter(|plugin| {
        //             // Only register plugins that have the dev-hook
        //             plugin.metadata.hooks.contains(&HookType::DevRegister)
        //         })
        //         .collect(),
        //     Err(e) => {
        //         warn!(err = ?e, "failed to find plugins, continuing without plugins");
        //         vec![]
        //     }
        // };

        for plugin in plugins {
            let name = plugin.metadata.name.clone();
            let version = plugin.metadata.version.clone();
            if let Err(e) = plugin_manager.register_plugin(plugin) {
                error!(
                    err = ?e,
                    name,
                    version,
                    "failed to register plugin, continuing without plugin"
                );
            } else {
                debug!(name, version, "registered plugin");
            }
        }

        let plugin_manager = Arc::new(plugin_manager);

        debug!("Loaded component bytes: {} bytes", wasm_bytes.len());

        // Create the engine for the host
        let engine = new_engine().context("failed to create engine")?;

        // Prepare the component for development 
        let workload_handle = prepare_component_dev(&engine, &wasm_bytes, plugin_manager.clone())
            .await
            .context("failed to prepare component for development")?;
        
        let (component_tx, _component_rx) =
            tokio::sync::watch::channel::<runtime::WorkloadHandle>(workload_handle);

        // Parse the address to create socket address
        let socket_addr: std::net::SocketAddr = self.address.parse()
            .context("failed to parse HTTP server address")?;

        // Create HTTP server plugin
        let http_server = Arc::new(HttpServer::new(socket_addr));

        // Build the host with HTTP server plugin
        let host = HostBuilder::new()
            .with_engine(engine)
            .with_plugin("http".to_string(), http_server.clone())
            .build()
            .context("failed to build host")?;

        // Start the host (this starts all plugins including HTTP server)
        host.start().await.context("failed to start host")?;
        
        // Bind the workload to the HTTP server with wildcard host header
        // Create a WitInterface for wasi:http/incoming-handler without specific host config
        // This will default to wildcard "*" binding
        let mut http_interfaces = std::collections::HashSet::new();
        http_interfaces.insert(runtime::WitInterface {
            namespace: "wasi".to_string(),
            package: "http".to_string(),
            interfaces: vec!["incoming-handler".to_string()],
            version: Some(semver::Version::parse("0.2.0").unwrap()),
            config: std::collections::HashMap::new(), // Empty config will default to "*"
        });
        
        // Clone the workload handle from the channel
        let current_handle = component_tx.borrow().clone();
        
        http_server.bind_workload(
            &"dev".to_string(),
            current_handle,
            http_interfaces.clone(),
        )
        .await
        .context("failed to bind workload to HTTP server")?;
        
        debug!("Workload bound to HTTP server with wildcard host header");

        let _runtime_config = self
            .runtime_config
            .clone()
            .into_iter()
            .filter_map(|orig| match orig.split_once('=') {
                Some((k, v)) => Some((k.to_string(), v.to_string())),
                None => {
                    warn!(key = orig, "runtime config key without value, skipping");
                    None
                }
            })
            .collect::<HashMap<String, String>>();

        let blobstore_root = self
            .blobstore_root
            .clone()
            .unwrap_or_else(|| ctx.in_data_dir("dev_blobstore"));
        // Ensure the blobstore root directory exists
        if !blobstore_root.exists() {
            tokio::fs::create_dir_all(&blobstore_root)
                .await
                .context("failed to create blobstore root directory")?;
        }
        debug!(path = ?blobstore_root.display(), "using blobstore root directory");

        let protocol = "http"; // Default to http for now

        // Canonicalize project root once to ensure consistent path comparisons
        let canonical_project_root = self.project_dir.canonicalize().with_context(|| {
            format!(
                "failed to canonicalize project directory: {}",
                self.project_dir.display()
            )
        })?;
        debug!(
            original = ?self.project_dir.display(),
            canonical = ?canonical_project_root.display(),
            "canonicalized project root for file watching"
        );

        // Enable/disable watching to prevent having the output artifact trigger a rebuild
        // This starts as true to prevent a rebuild on the first run
        let pause_watch = Arc::new(AtomicBool::new(true));
        let watcher_paused = pause_watch.clone();
        let (stop_tx, mut stop_rx) = mpsc::channel::<()>(1);
        let (reload_tx, mut reload_rx) = mpsc::channel::<()>(1);

        // Build initial ignore set including artifact path and project-specific build directories
        let initial_ignore_set = build_ignore_set(&canonical_project_root, &project_context);
        debug!(
            ignore_count = initial_ignore_set.len(),
            project_type = ?project_context,
            "built initial file watcher ignore set"
        );
        let ignore_paths = Arc::new(initial_ignore_set);
        let ignore_paths_notify = ignore_paths.clone();

        let canonical_project_root_notify = canonical_project_root.clone();
        debug!(path = ?self.project_dir.display(), "setting up watcher");
        // Watch for changes and rebuild/deploy as needed
        let mut watcher = notify::recommended_watcher(move |res: _| match res {
            Ok(event) => {
                if let NotifyEvent {
                    kind:
                        EventKind::Create(_)
                        | EventKind::Modify(ModifyKind::Data(_))
                        | EventKind::Remove(_),
                    paths,
                    ..
                } = event
                {
                    // Check if any of the changed paths should be ignored to prevent
                    // recursive rebuilds from artifact directories
                    let set = &ignore_paths_notify;
                    if paths
                        .iter()
                        .any(|p| is_ignored(p, &canonical_project_root_notify, set))
                    {
                        trace!(paths = ?paths, "ignoring file changes in artifact directories");
                        return;
                    }
                    // If watch has been paused for any reason, skip notifications
                    if watcher_paused.load(Ordering::SeqCst) {
                        return;
                    }
                    trace!(paths = ?paths, "file event triggered dev loop");

                    // NOTE(brooksmtownsend): `try_send` here is used intentionally to prevent
                    // multiple file reloads from queuing up a backlog of reloads.
                    let _ = reload_tx.try_send(());
                }
            }
            Err(e) => {
                error!(err = ?e, "watch failed");
            }
        })?;

        watcher.watch(&canonical_project_root, RecursiveMode::Recursive)?;
        debug!("watching for file changes...");

        // Spawn a task to handle Ctrl + C signal
        tokio::spawn(async move {
            tokio::signal::ctrl_c()
                .await
                .context("failed to wait for ctrl_c signal")?;
            stop_tx
                .send(())
                .await
                .context("failed to send stop signal after receiving Ctrl + c")?;
            Result::<_, anyhow::Error>::Ok(())
        });

        // Enable file watching
        pause_watch.store(false, Ordering::SeqCst);
        // Make sure the reload channel is empty before starting the loop
        let _ = reload_rx.try_recv();

        info!(address = %format!("{}://{}", protocol, self.address), "listening for HTTP requests");

        // Clone for use in the reload loop
        let http_server_reload = http_server.clone();
        let http_interfaces_reload = http_interfaces.clone();

        loop {
            info!("watching for file changes (press Ctrl+c to stop)...");
            select! {
                // Process a file change/reload
                _ = reload_rx.recv() => {
                    pause_watch.store(true, Ordering::SeqCst);

                    info!("rebuilding component after file changed ...");

                    // TODO(IMPORTANT): ensure that this calls the build pre-post hooks
                    // TODO(#21): Skip wit fetch if no .wit change
                    // TODO(#22): Typescript: Skip install if no package.json change
                    let rebuild_result = build_component(
                        &self.project_dir,
                        ctx,
                        &config,
                    ).await;

                    match rebuild_result {
                        Ok(build_result) => {
                            // Use the new artifact path from the build result
                            let artifact_path = build_result.artifact_path;
                            info!(path = %artifact_path.display(), "component rebuilt successfully");

                            info!("deploying rebuilt component ...");
                            let wasm_bytes = tokio::fs::read(&artifact_path)
                                .await
                                .context("failed to read artifact file")?;

                            info!("Component rebuilt: {} bytes", wasm_bytes.len());
                            
                            // Prepare the rebuilt component
                            let rebuild_engine = new_engine().context("failed to create engine for rebuild")?;
                            let new_workload_handle = prepare_component_dev(&rebuild_engine, &wasm_bytes, plugin_manager.clone().clear_instances())
                                .await
                                .context("failed to prepare component")?;
                            
                            // Rebind the new workload to the HTTP server
                            http_server_reload.bind_workload(
                                &"dev".to_string(),
                                new_workload_handle.clone(),
                                http_interfaces_reload.clone(),
                            )
                            .await
                            .context("failed to rebind workload to HTTP server")?;
                            
                            component_tx.send_replace(new_workload_handle);
                            debug!("Workload rebound to HTTP server");

                            // Avoid jitter with reloads by pausing the watcher for a short time
                            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                            // Make sure that the reload channel is empty before unpausing the watcher
                            let _ = reload_rx.try_recv();
                            pause_watch.store(false, Ordering::SeqCst);
                        }
                        Err(e) => {
                            info!("failed to build component, will retry on next file change");
                            // TODO(#23): This doesn't include color output
                            // This nicely formats the error message
                            error!("{e}");
                            // If the build fails, we pause the watcher to prevent further reloads
                            let _ = reload_rx.try_recv();
                            pause_watch.store(false, Ordering::SeqCst);
                            continue;
                        }
                    }
                },

                // Process a stop
                _ = stop_rx.recv() => {
                    info!("Stopping development session ...");
                    pause_watch.store(true, Ordering::SeqCst);

                    break
                },
            }
        }

        // TODO: Re-enable when plugin system is adapted to local runtime
        // Call post-hooks with component bytes context
        // Base64 encode the bytes since context only supports HashMap<String, String>
        // let component_bytes_b64 = base64::engine::general_purpose::STANDARD.encode(&wasm_bytes);
        // let mut post_context = HashMap::new();
        // post_context.insert(
        //     "dev.component_bytes_base64".to_string(),
        //     component_bytes_b64,
        // );
        // let post_runtime_context = Arc::new(RwLock::new(post_context));
        // ctx.call_post_hooks(post_runtime_context, HookType::AfterDev)
        //     .await?;

        Ok(CommandOutput::ok(
            "Development command executed successfully".to_string(),
            None,
        ))
    }
}

// TODO: HTTP server functionality has been moved to the HTTP server plugin
// located at: crates/runtime/src/plugin/http_server.rs

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_is_ignored_rust_project() {
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let project_root = temp_dir
            .path()
            .canonicalize()
            .expect("failed to canonicalize temp dir");

        // Create target directory
        fs::create_dir_all(project_root.join("target")).expect("failed to create target dir");

        let context = ProjectContext::Rust {
            cargo_toml_path: project_root.join("Cargo.toml"),
        };
        let ignore_paths = build_ignore_set(&project_root, &context);

        // Test that target/ is ignored
        let target_file = project_root.join("target").join("test.txt");
        assert!(is_ignored(&target_file, &project_root, &ignore_paths));

        // Test that src/ is not ignored
        let src_file = project_root.join("src").join("lib.rs");
        assert!(!is_ignored(&src_file, &project_root, &ignore_paths));
    }

    #[test]
    fn test_is_ignored_typescript_project() {
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let project_root = temp_dir
            .path()
            .canonicalize()
            .expect("failed to canonicalize temp dir");

        fs::create_dir_all(project_root.join("node_modules"))
            .expect("failed to create node_modules");

        let context = ProjectContext::TypeScript {
            package_json_path: project_root.join("package.json"),
        };
        let ignore_paths = build_ignore_set(&project_root, &context);

        // Test that node_modules/ is ignored
        let node_modules_file = project_root.join("node_modules").join("test");
        assert!(is_ignored(&node_modules_file, &project_root, &ignore_paths));

        // Test that src/ is not ignored
        let src_file = project_root.join("src").join("index.ts");
        assert!(!is_ignored(&src_file, &project_root, &ignore_paths));
    }

    #[test]
    fn test_artifact_parent_not_ignored_by_default() {
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let project_root = temp_dir
            .path()
            .canonicalize()
            .expect("failed to canonicalize temp dir");
        let artifact_dir = project_root.join("custom");
        let artifact_path = artifact_dir.join("output.wasm");

        fs::create_dir_all(&artifact_dir).expect("failed to create artifact dir");
        fs::write(&artifact_path, "test content").expect("failed to create artifact file");

        let context = ProjectContext::General;
        let ignore_paths = build_ignore_set(&project_root, &context);

        // Sibling in custom/ is **not** ignored anymore
        let sibling_file = artifact_dir.join("other.wasm");
        fs::write(&sibling_file, "other content").expect("failed to create sibling file");
        assert!(!is_ignored(&sibling_file, &project_root, &ignore_paths));

        // Outside normal ignore dirs should also not be ignored
        let outside_file = project_root.join("src").join("main.rs");
        assert!(!is_ignored(&outside_file, &project_root, &ignore_paths));
    }

    #[test]
    fn test_mixed_project_includes_all_patterns() {
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let project_root = temp_dir
            .path()
            .canonicalize()
            .expect("failed to canonicalize temp dir");

        // Create directories for different project types
        fs::create_dir_all(project_root.join("target")).expect("failed to create target");
        fs::create_dir_all(project_root.join("node_modules"))
            .expect("failed to create node_modules");

        let context = ProjectContext::Mixed {
            detected_types: vec!["Rust".to_string(), "TypeScript".to_string()],
        };
        let ignore_paths = build_ignore_set(&project_root, &context);

        // Test that both Rust and TypeScript patterns are ignored
        let target_file = project_root.join("target").join("test");
        let node_modules_file = project_root.join("node_modules").join("test");

        assert!(is_ignored(&target_file, &project_root, &ignore_paths));
        assert!(is_ignored(&node_modules_file, &project_root, &ignore_paths));
    }

    #[test]
    fn test_relative_vs_absolute_path_consistency() {
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let project_root = temp_dir
            .path()
            .canonicalize()
            .expect("failed to canonicalize temp dir");

        // Create target directory
        fs::create_dir_all(project_root.join("target")).expect("failed to create target dir");

        let context = ProjectContext::Rust {
            cargo_toml_path: project_root.join("Cargo.toml"),
        };
        let ignore_paths = build_ignore_set(&project_root, &context);

        // Test with absolute path
        let absolute_target_file = project_root.join("target").join("test.txt");

        // Test with relative path (simulate what might come from file watcher)
        let relative_target_file = PathBuf::from("./target/test.txt");

        // Both should be consistently ignored when checked against canonical project root
        assert!(is_ignored(
            &absolute_target_file,
            &project_root,
            &ignore_paths
        ));

        // Note: For relative paths to work correctly, they need to be resolved
        // relative to the project root first, which is what our canonicalization handles
        let resolved_relative = project_root.join(
            relative_target_file
                .strip_prefix("./")
                .unwrap_or(&relative_target_file),
        );
        assert!(is_ignored(&resolved_relative, &project_root, &ignore_paths));
    }

    #[test]
    fn test_symlink_handling() {
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let project_root = temp_dir
            .path()
            .canonicalize()
            .expect("failed to canonicalize temp dir");

        // Create actual target directory outside project
        let external_target = temp_dir.path().join("external_target");
        fs::create_dir_all(&external_target).expect("failed to create external target");

        // Create symlink from project to external target
        let symlink_target = project_root.join("target");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&external_target, &symlink_target)
            .expect("failed to create symlink");
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&external_target, &symlink_target)
            .expect("failed to create symlink");

        let context = ProjectContext::Rust {
            cargo_toml_path: project_root.join("Cargo.toml"),
        };
        let ignore_paths = build_ignore_set(&project_root, &context);

        // Test file accessed through symlink
        let file_through_symlink = symlink_target.join("test.txt");
        fs::write(&external_target.join("test.txt"), "content").expect("failed to write test file");

        // Should be ignored because the symlink resolves to target/ pattern
        assert!(is_ignored(
            &file_through_symlink,
            &project_root,
            &ignore_paths
        ));

        // Also test direct access to the external target
        let external_file = external_target.join("test.txt");
        // This might or might not be ignored depending on canonicalization
        // The key is that symlinked paths are handled consistently
        let _is_external_ignored = is_ignored(&external_file, &project_root, &ignore_paths);
    }

    #[test]
    fn test_nested_dirs_not_ignored_without_explicit_entry() {
        let temp_dir = TempDir::new().expect("failed to create temp dir");
        let project_root = temp_dir
            .path()
            .canonicalize()
            .expect("failed to canonicalize temp dir");

        // Top-level target/ (explicitly ignored by build_ignore_set)
        let top_target = project_root.join("target");
        fs::create_dir_all(top_target.join("debug")).expect("failed to create top-level target");
        let top_level_file = top_target.join("debug").join("top.wasm");

        // Nested structure: project/subproject/target (NOT explicitly in ignore set)
        let subproject_dir = project_root.join("subproject");
        let nested_target = subproject_dir.join("target");
        fs::create_dir_all(nested_target.join("debug")).expect("failed to create nested target");
        let nested_file = nested_target.join("debug").join("nested.wasm");

        let context = ProjectContext::Rust {
            cargo_toml_path: project_root.join("Cargo.toml"),
        };
        let ignore_paths = build_ignore_set(&project_root, &context);

        // Baseline: top-level target/* IS ignored
        assert!(is_ignored(&top_level_file, &project_root, &ignore_paths));

        // subproject/target/* is NOT ignored
        assert!(!is_ignored(&nested_file, &project_root, &ignore_paths));

        // Sanity: subproject source is NOT ignored
        let subproject_src = subproject_dir.join("src").join("main.rs");
        assert!(!is_ignored(&subproject_src, &project_root, &ignore_paths));
    }

    // TODO: TLS functionality tests have been moved to crates/runtime/src/plugin/http_server.rs
}
