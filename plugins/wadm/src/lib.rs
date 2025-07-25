//! Implementation of `wasmcloud:wash/plugin` for this [`crate::Component`]
mod bindings;

use crate::bindings::{
    wasi::logging::logging::{log, Level},
    wasmcloud::wash::types::{Command, CommandArgument, HookType, Metadata, Runner},
};

pub(crate) struct Component;

impl crate::bindings::exports::wasmcloud::wash::plugin::Guest for crate::Component {
    /// Called by wash to retrieve the plugin metadata
    fn info() -> Metadata {
        Metadata {
            id: "dev.wasmcloud.wadm".to_string(),
            name: "wadm".to_string(),
            description: "Generates a wadm manifest after a dev loop, or when pointing to a Wasm component project"
                .to_string(),
            contact: "wasmCloud Team".to_string(),
            url: "https://github.com/wasmcloud/wash".to_string(),
            license: "Apache-2.0".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            sub_commands: vec![],
            command: Some(Command {
                id: "wadm".to_string(),
                name: "wadm".to_string(),
                description: "Generates a wadm manifest for a component or component project".to_string(),
                flags: vec![
                    (
                        "generate".to_string(),
                        CommandArgument {
                            name: "generate".to_string(),
                            description: "Generate a wadm manifest for the given component or project".to_string(),
                            env: None,
                            default: Some("true".to_string()),
                            value: None,
                        }
                    ),
                    (
                        "dry-run".to_string(),
                        CommandArgument {
                            name: "dry-run".to_string(),
                            description: "Print the manifest to stdout instead of writing to disk".to_string(),
                            env: None,
                            default: Some("false".to_string()),
                            value: None,
                        }
                    ),
                ],
                arguments: vec![
                    CommandArgument {
                        name: "project-path".to_string(),
                        description: "Path to the component project directory".to_string(),
                        env: None,
                        default: None,
                        value: None,
                    }
                ],
                usage: vec!["Make sure to drink your Ovaltine!".to_string()],
            }),
            hooks: vec![HookType::AfterDev],
        }
    }
    fn hook(_r: Runner, ty: HookType) -> anyhow::Result<String, String> {
        match ty {
            HookType::AfterDev => {
                log(Level::Info, "wadm", "generating wadm manifests...");
                Ok("generating wadm manifests...".to_string())
            }
            _ => {
                log(Level::Warn, "wadm", "unsupported hook type");
                Err("unsupported hook type".to_string())
            }
        }
    }
    fn run(_: Runner, _: Command) -> anyhow::Result<String, String> {
        log(Level::Debug, "wadm", "executing command");
        log(Level::Info, "wadm", "generating wadm manifests...");
        Ok("generating wadm manifests...".to_string())
    }
    fn initialize(_: Runner) -> anyhow::Result<String, String> {
        Ok(String::with_capacity(0))
    }
}
