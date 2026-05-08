//! Codex app-server command resolution.
//!
//! The gateway prefers the Codex.app bundled binary and falls back to the
//! shell `codex` CLI only when the application bundle is not present.

use std::path::Path;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexRuntimeSource {
    CodexApp,
    PathCodex,
}

#[derive(Debug, Clone, Serialize)]
pub struct CodexRuntime {
    pub source: CodexRuntimeSource,
    pub command: Vec<String>,
}

/// Resolve the app-server command without starting the process.
pub fn resolve(disable_plugins: bool) -> CodexRuntime {
    let bundled = "/Applications/Codex.app/Contents/Resources/codex";
    let (source, binary) = if Path::new(bundled).exists() {
        (CodexRuntimeSource::CodexApp, bundled.to_string())
    } else {
        (CodexRuntimeSource::PathCodex, "codex".to_string())
    };
    let mut command = vec![binary, "app-server".to_string()];
    if disable_plugins {
        command.extend(["--disable".to_string(), "plugins".to_string()]);
    }
    CodexRuntime { source, command }
}
