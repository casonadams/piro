use std::path::PathBuf;
use std::sync::OnceLock;

use crate::provider::DEFAULT_MODEL_SPEC;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchOptions {
    pub model_spec: Option<String>,
    pub resume: bool,
    pub auto_approve: bool,
}

static LAUNCH: OnceLock<LaunchOptions> = OnceLock::new();

/// Set once by the standalone binary before the root mounts; NSH development hosts never set it.
pub fn set(options: LaunchOptions) -> Result<(), String> {
    LAUNCH
        .set(options)
        .map_err(|_| "launch options already set".to_string())
}

/// How the session starts. Without a data directory nothing is saved and no MCP server starts, which keeps
/// tests and NSH development hosts hermetic.
#[derive(Debug, Clone, Default)]
pub struct SessionConfig {
    pub model_spec: String,
    pub resume: bool,
    pub auto_approve: bool,
    pub data_dir: Option<PathBuf>,
}

pub fn session_config() -> SessionConfig {
    let launched = LAUNCH.get();
    SessionConfig {
        model_spec: model_spec(),
        resume: launched.is_some_and(|o| o.resume),
        auto_approve: launched.is_some_and(|o| o.auto_approve),
        data_dir: launched
            .and_then(|_| std::env::var_os("HOME"))
            .map(|home| PathBuf::from(home).join(".piro")),
    }
}

fn model_spec() -> String {
    LAUNCH
        .get()
        .and_then(|o| o.model_spec.clone())
        .or_else(|| std::env::var("PIRO_MODEL").ok())
        .unwrap_or_else(|| DEFAULT_MODEL_SPEC.into())
}
