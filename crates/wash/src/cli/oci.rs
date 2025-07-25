use std::path::PathBuf;

use anyhow::Context as _;
use clap::{Args, Subcommand};
use etcetera::AppStrategy;
use tracing::instrument;

use crate::{
    cli::{CliCommand, CliContext, CommandOutput},
    oci::{OCI_CACHE_DIR, OciConfig, pull_component, push_component},
    runtime::bindings::plugin::wasmcloud::wash::types::HookType,
};

#[derive(Subcommand, Debug, Clone)]
pub enum OciCommand {
    Pull(PullCommand),
    Push(PushCommand),
}

impl CliCommand for OciCommand {
    /// Handle the OCI command
    #[instrument(level = "debug", skip_all, name = "oci")]
    async fn handle(&self, ctx: &CliContext) -> anyhow::Result<CommandOutput> {
        match self {
            OciCommand::Pull(cmd) => cmd.handle(ctx).await,
            OciCommand::Push(cmd) => cmd.handle(ctx).await,
        }
    }
    fn enable_pre_hook(&self) -> Option<HookType> {
        match self {
            OciCommand::Pull(_) => None,
            OciCommand::Push(_) => Some(HookType::BeforePush),
        }
    }
    fn enable_post_hook(&self) -> Option<HookType> {
        match self {
            OciCommand::Pull(_) => None,
            OciCommand::Push(_) => Some(HookType::AfterPush),
        }
    }
}

#[derive(Args, Debug, Clone)]
pub struct PullCommand {
    /// The OCI reference to pull
    #[clap(name = "reference")]
    reference: String,
    /// The path to write the pulled component to
    #[clap(name = "component_path", default_value = "component.wasm")]
    component_path: PathBuf,
}

impl PullCommand {
    /// Handle the OCI command
    #[instrument(level = "debug", skip_all, name = "oci")]
    pub async fn handle(&self, ctx: &CliContext) -> anyhow::Result<CommandOutput> {
        let oci_config = OciConfig::new_with_cache(ctx.cache_dir().join(OCI_CACHE_DIR));
        let c = pull_component(&self.reference, oci_config).await?;

        // Write the component to the specified output path
        tokio::fs::write(&self.component_path, &c)
            .await
            .context("failed to write pulled component to output path")?;

        Ok(CommandOutput::ok(
            format!(
                "Pulled and saved component to {}",
                self.component_path.display()
            ),
            Some(serde_json::json!({
                "message": "OCI command executed successfully.",
                "output_path": self.component_path.to_string_lossy(),
                "bytes": c.len(),
                "success": true,
            })),
        ))
    }
}

#[derive(Args, Debug, Clone)]
pub struct PushCommand {
    /// The OCI reference to push
    #[clap(name = "reference")]
    reference: String,
    /// The path to the component to push
    #[clap(name = "component_path")]
    component_path: PathBuf,
}

impl PushCommand {
    /// Handle the OCI command
    #[instrument(level = "debug", skip_all, name = "oci")]
    pub async fn handle(&self, ctx: &CliContext) -> anyhow::Result<CommandOutput> {
        let component = tokio::fs::read(&self.component_path)
            .await
            .context("failed to read component file")?;

        let oci_config = OciConfig::new_with_cache(ctx.cache_dir().join(OCI_CACHE_DIR));

        push_component(&self.reference, &component, oci_config).await?;

        Ok(CommandOutput::ok(
            "OCI command executed successfully.".to_string(),
            Some(serde_json::json!({
                "message": "OCI command executed successfully.",
                "success": true,
            })),
        ))
    }
}
