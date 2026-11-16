use std::sync::OnceLock;

use crate::provider::DEFAULT_MODEL_SPEC;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchOptions {
    pub model_spec: Option<String>,
}

static LAUNCH: OnceLock<LaunchOptions> = OnceLock::new();

/// Set once by the standalone binary before the root mounts; NSH development hosts never set it.
pub fn set(options: LaunchOptions) -> Result<(), String> {
    LAUNCH
        .set(options)
        .map_err(|_| "launch options already set".to_string())
}

pub fn model_spec() -> String {
    LAUNCH
        .get()
        .and_then(|o| o.model_spec.clone())
        .or_else(|| std::env::var("PIRO_MODEL").ok())
        .unwrap_or_else(|| DEFAULT_MODEL_SPEC.into())
}
