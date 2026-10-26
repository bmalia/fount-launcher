use config::{Config, ConfigError, File, FileFormat};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::entry::{LauncherAction, LauncherEntry};

/// Action type variants represented cleanly in JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CustomActionDef {
    /// Single command string: {"type": "exec", "command": "...", "terminal": false}
    Exec {
        command: String,
        #[serde(default)]
        terminal: bool,
    },
    /// Discrete binary and arguments: {"type": "command", "program": "...", "args": [...]}
    Command {
        program: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        terminal: bool,
    },
    /// Shell script string run via `sh -c`: {"type": "shell", "script": "..."}
    Shell { script: String },
    /// Custom message/event for the host application: {"type": "custom", "id": "..."}
    Custom { id: String },
}

impl From<CustomActionDef> for LauncherAction {
    fn from(def: CustomActionDef) -> Self {
        match def {
            CustomActionDef::Exec { command, terminal } => {
                LauncherAction::Exec { command, terminal }
            }
            CustomActionDef::Command {
                program,
                args,
                terminal,
            } => LauncherAction::Command {
                program,
                args,
                terminal,
            },
            CustomActionDef::Shell { script } => LauncherAction::Shell(script),
            CustomActionDef::Custom { id } => LauncherAction::Custom(id),
        }
    }
}

/// JSON-deserializable custom action configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomEntryConfig {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub action: CustomActionDef,
}

impl From<CustomEntryConfig> for LauncherEntry {
    fn from(cfg: CustomEntryConfig) -> Self {
        LauncherEntry::custom_action(
            cfg.id,
            cfg.name,
            cfg.icon,
            cfg.description,
            cfg.aliases,
            cfg.action.into(),
        )
    }
}

/// Top-level configuration object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LauncherConfig {
    #[serde(default)]
    pub custom_dirs: Vec<PathBuf>,
    #[serde(default = "default_true")]
    pub include_system_actions: bool,
    #[serde(default)]
    pub custom_actions: Vec<CustomEntryConfig>,
}

fn default_true() -> bool {
    true
}

impl Default for LauncherConfig {
    fn default() -> Self {
        Self {
            custom_dirs: Vec::new(),
            include_system_actions: true,
            custom_actions: Vec::new(),
        }
    }
}

impl LauncherConfig {
    /// Converts all configured custom action definitions into `LauncherEntry` items.
    pub fn to_launcher_entries(&self) -> Vec<LauncherEntry> {
        self.custom_actions
            .iter()
            .cloned()
            .map(LauncherEntry::from)
            .collect()
    }
}

pub struct ConfigManager;

impl ConfigManager {
    /// Loads configuration from an optional file path or string content.
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<LauncherConfig, ConfigError> {
        let builder = Config::builder()
            .set_default("include_system_actions", true)?
            .set_default("custom_dirs", Vec::<String>::new())?
            .set_default("custom_actions", Vec::<String>::new())?
            .add_source(
                File::from(path.as_ref())
                    .format(FileFormat::Json)
                    .required(false),
            );

        let config = builder.build()?;
        config.try_deserialize()
    }

    /// Loads directly from a JSON string (handy for testing or embedded configs).
    pub fn load_from_str(json_str: &str) -> Result<LauncherConfig, ConfigError> {
        let builder = Config::builder()
            .set_default("include_system_actions", true)?
            .set_default("custom_dirs", Vec::<String>::new())?
            .set_default("custom_actions", Vec::<String>::new())?
            .add_source(File::from_str(json_str, FileFormat::Json));

        let config = builder.build()?;
        config.try_deserialize()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_config_from_str() {
        let json = "{}";
        let config = ConfigManager::load_from_str(json).unwrap();
        assert!(config.include_system_actions);
        assert!(config.custom_dirs.is_empty());
        assert!(config.custom_actions.is_empty());
    }

    #[test]
    fn test_custom_actions_deserialization() {
        let json = r#"{
            "custom_dirs": ["/custom/apps"],
            "include_system_actions": false,
            "custom_actions": [
                {
                    "id": "action:screenshot",
                    "name": "Take Screenshot",
                    "description": "Capture screen area",
                    "icon": "camera-photo",
                    "aliases": ["snip", "screen"],
                    "action": {
                        "type": "command",
                        "program": "grim",
                        "args": ["-g"],
                        "terminal": false
                    }
                },
                {
                    "id": "action:shell_cmd",
                    "name": "Shell Test",
                    "action": {
                        "type": "shell",
                        "script": "echo hello"
                    }
                }
            ]
        }"#;

        let config = ConfigManager::load_from_str(json).unwrap();
        assert!(!config.include_system_actions);
        assert_eq!(config.custom_dirs, vec![PathBuf::from("/custom/apps")]);
        assert_eq!(config.custom_actions.len(), 2);

        let entries = config.to_launcher_entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, "action:screenshot");
        assert_eq!(entries[0].name, "Take Screenshot");
        assert_eq!(entries[1].id, "action:shell_cmd");
    }
}
