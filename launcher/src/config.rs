// Configuration file parsing for .hrlauncher.toml
//
// The config file defines sync rules that automatically watch filesystem paths
// and sync changes to connected clients.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Root configuration structure for .hrlauncher.toml
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Project metadata
    pub project: ProjectConfig,

    /// Sync rules - each rule watches paths and syncs changes to clients
    #[serde(rename = "sync")]
    pub sync_rules: Vec<SyncRule>,
}

/// Project metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    /// Project name (used for logging and display)
    pub name: String,

    /// Optional description
    #[serde(default)]
    pub description: Option<String>,
}

/// A sync rule defines what files to watch and where to sync them
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncRule {
    /// Optional name for this sync rule (for logging)
    #[serde(default)]
    pub name: Option<String>,

    /// Glob patterns for files to watch and sync
    /// Patterns are relative to the config file location
    /// Examples: "*.exe", "assets/**/*", "src/**/*.rs"
    pub include: Vec<String>,

    /// Optional glob patterns to exclude from syncing
    /// Applied after include patterns
    /// Examples: "**/*.tmp", "**/.DS_Store", ".git/**"
    #[serde(default)]
    pub exclude: Vec<String>,

    /// Destination path on clients (relative to client's working directory)
    /// Can use ~/ for home directory
    /// Examples: ".", "games/myproject/", "~/bin/"
    pub destination: String,

    /// Optional: Only sync to clients whose hostnames match these patterns
    /// Supports glob patterns like "windows-*", "dev-*", etc.
    /// If not specified, syncs to all connected clients
    #[serde(default)]
    pub clients: Vec<String>,

    /// Optional: If true, delete files on clients that don't exist in source
    /// Use with caution - this will remove files!
    #[serde(default)]
    pub mirror: bool,

    /// Optional: Execute configuration to run after files are synced
    #[serde(default)]
    pub execute: Option<ExecuteConfig>,
}

/// Configuration for executing a binary after sync
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteConfig {
    /// Command to execute (relative path or absolute path)
    /// Can reference synced files using destination path
    pub command: String,

    /// Optional command-line arguments
    #[serde(default)]
    pub args: Vec<String>,

    /// Optional environment variables to set
    #[serde(default)]
    pub env: HashMap<String, String>,

    /// Optional working directory (defaults to destination if not specified)
    #[serde(default)]
    pub working_dir: Option<String>,
}

impl Config {
    /// Load configuration from a .hrlauncher.toml file
    pub fn from_file(path: &Path) -> Result<Self> {
        let contents = std::fs::read_to_string(path)
            .context(format!("Failed to read config file: {}", path.display()))?;

        let config: Config = toml::from_str(&contents)
            .context(format!("Failed to parse config file: {}", path.display()))?;

        config.validate()?;
        Ok(config)
    }

    /// Find .hrlauncher.toml by searching current directory and ancestors
    pub fn find_and_load() -> Result<(PathBuf, Self)> {
        let mut current = std::env::current_dir()
            .context("Failed to get current directory")?;

        loop {
            let config_path = current.join(".hrlauncher.toml");
            if config_path.exists() {
                let config = Self::from_file(&config_path)?;
                return Ok((config_path, config));
            }

            // Try parent directory
            if !current.pop() {
                anyhow::bail!(
                    "No .hrlauncher.toml found in current directory or any parent directory"
                );
            }
        }
    }

    /// Validate the configuration
    fn validate(&self) -> Result<()> {
        // Ensure we have at least one sync rule
        if self.sync_rules.is_empty() {
            anyhow::bail!("Config must have at least one [[sync]] rule");
        }

        // Validate each sync rule
        for (idx, rule) in self.sync_rules.iter().enumerate() {
            let default_name = format!("sync rule {}", idx + 1);
            let rule_name = rule.name.as_deref().unwrap_or(&default_name);

            if rule.include.is_empty() {
                anyhow::bail!("{}: must have at least one include pattern", rule_name);
            }

            if rule.destination.is_empty() {
                anyhow::bail!("{}: destination cannot be empty", rule_name);
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_config() {
        let toml = r#"
[project]
name = "test-project"
description = "A test project"

[[sync]]
name = "executables"
include = ["*.exe", "*.dll"]
destination = "bin/"
clients = ["windows-*"]

[sync.execute]
command = "bin/game.exe"
args = ["--debug", "--windowed"]

[sync.execute.env]
RUST_LOG = "debug"

[[sync]]
include = ["assets/**/*"]
exclude = ["**/*.psd"]
destination = "assets/"
mirror = true
"#;

        let config: Config = toml::from_str(toml).expect("Failed to parse config");

        assert_eq!(config.project.name, "test-project");
        assert_eq!(config.sync_rules.len(), 2);

        let rule1 = &config.sync_rules[0];
        assert_eq!(rule1.name.as_deref(), Some("executables"));
        assert_eq!(rule1.include.len(), 2);
        assert_eq!(rule1.destination, "bin/");
        assert_eq!(rule1.clients, vec!["windows-*"]);

        let exec = rule1.execute.as_ref().expect("Execute config should exist");
        assert_eq!(exec.command, "bin/game.exe");
        assert_eq!(exec.args, vec!["--debug", "--windowed"]);
        assert_eq!(exec.env.get("RUST_LOG"), Some(&"debug".to_string()));

        let rule2 = &config.sync_rules[1];
        assert_eq!(rule2.include, vec!["assets/**/*"]);
        assert_eq!(rule2.exclude, vec!["**/*.psd"]);
        assert!(rule2.mirror);
    }
}
