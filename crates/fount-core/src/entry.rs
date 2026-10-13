use crate::desktop_entry::DesktopEntry;
use serde::{Deserialize, Serialize};
use std::io;
use std::process::{Child, Command};

/// Kind of entry represented in the launcher.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryKind {
    /// Standard XDG desktop application entry.
    Application(Box<DesktopEntry>),
    /// Custom action or command (e.g. system control, script, shell snippet).
    CustomAction,
}

/// Action to execute when a launcher entry is selected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LauncherAction {
    /// Command line string to execute.
    Exec {
        command: String,
        terminal: bool,
    },
    /// Program path/binary and discrete arguments.
    Command {
        program: String,
        args: Vec<String>,
        terminal: bool,
    },
    /// Shell command string run via `sh -c`.
    Shell(String),
    /// Custom identifier or message to be handled by the caller/frontend.
    Custom(String),
}

impl LauncherAction {
    /// Spawns this action as a child process.
    pub fn execute(&self) -> io::Result<Child> {
        match self {
            LauncherAction::Command {
                program,
                args,
                terminal,
            } => {
                if *terminal {
                    spawn_in_terminal(&format!("{} {}", program, args.join(" ")))
                } else {
                    Command::new(program).args(args).spawn()
                }
            }
            LauncherAction::Exec { command, terminal } => {
                if *terminal {
                    spawn_in_terminal(command)
                } else {
                    Command::new("sh").arg("-c").arg(command).spawn()
                }
            }
            LauncherAction::Shell(script) => Command::new("sh").arg("-c").arg(script).spawn(),
            LauncherAction::Custom(id) => Err(io::Error::other(format!(
                "Custom action '{id}' must be handled by the caller/frontend"
            ))),
        }
    }
}

/// Helper to spawn command in the user's terminal emulator or standard fallback.
fn spawn_in_terminal(cmd: &str) -> io::Result<Child> {
    if let Ok(term) = std::env::var("TERMINAL") {
        return Command::new(term).arg("-e").arg(cmd).spawn();
    }
    // Try common terminal emulators
    for term in &["x-terminal-emulator", "foot", "alacritty", "kitty", "gnome-terminal"] {
        if let Ok(child) = Command::new(term).arg("-e").arg(cmd).spawn() {
            return Ok(child);
        }
    }
    // Fallback directly to sh
    Command::new("sh").arg("-c").arg(cmd).spawn()
}

/// A searchable item in the launcher, representing either an application or a custom action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LauncherEntry {
    /// Unique identifier for this entry (e.g. `app:firefox.desktop` or `action:poweroff`).
    pub id: String,
    /// Display name (e.g. "Firefox" or "Power Off").
    pub name: String,
    /// Icon name or path (e.g. "firefox" or "system-shutdown").
    pub icon: Option<String>,
    /// Secondary description or category (e.g. "Web Browser" or "Shut down the computer").
    pub description: Option<String>,
    /// Search aliases and keywords (e.g. `["shutdown", "turn off", "halt"]`).
    pub aliases: Vec<String>,
    /// Action to execute when selected.
    pub action: LauncherAction,
    /// The entry classification.
    pub kind: EntryKind,
    /// Precomputed haystack for fuzzy matching.
    pub search_haystack: String,
}

impl LauncherEntry {
    /// Creates a `LauncherEntry` from an XDG `DesktopEntry`.
    pub fn from_desktop_entry(entry: DesktopEntry) -> Self {
        let id = entry.id.clone();
        let name = entry.name.clone();
        let icon = entry.icon.clone();
        let description = entry.generic_name.clone().or_else(|| entry.comment.clone());
        let mut aliases = entry.keywords.clone();
        aliases.extend(entry.categories.clone());

        let action = LauncherAction::Exec {
            command: entry.exec.clone(),
            terminal: entry.terminal,
        };
        let search_haystack = entry.search_haystack.clone();

        Self {
            id,
            name,
            icon,
            description,
            aliases,
            action,
            kind: EntryKind::Application(Box::new(entry)),
            search_haystack,
        }
    }

    /// Creates a custom action `LauncherEntry`.
    pub fn custom_action(
        id: impl Into<String>,
        name: impl Into<String>,
        icon: Option<String>,
        description: Option<String>,
        aliases: Vec<String>,
        action: LauncherAction,
    ) -> Self {
        let id = id.into();
        let name = name.into();

        let mut parts = vec![name.clone()];
        if let Some(ref desc) = description {
            parts.push(desc.clone());
        }
        if !aliases.is_empty() {
            parts.push(aliases.join(" "));
        }
        match &action {
            LauncherAction::Exec { command, .. } => parts.push(command.clone()),
            LauncherAction::Command { program, args, .. } => {
                parts.push(program.clone());
                parts.push(args.join(" "));
            }
            LauncherAction::Shell(script) => parts.push(script.clone()),
            LauncherAction::Custom(custom_id) => parts.push(custom_id.clone()),
        }
        let search_haystack = parts.join(" ");

        Self {
            id,
            name,
            icon,
            description,
            aliases,
            action,
            kind: EntryKind::CustomAction,
            search_haystack,
        }
    }

    /// Returns a reference to the inner `DesktopEntry` if this is an application.
    pub fn as_desktop_entry(&self) -> Option<&DesktopEntry> {
        match &self.kind {
            EntryKind::Application(entry) => Some(entry.as_ref()),
            EntryKind::CustomAction => None,
        }
    }

    /// Whether this entry represents a custom action.
    pub fn is_custom_action(&self) -> bool {
        matches!(self.kind, EntryKind::CustomAction)
    }
}

impl From<DesktopEntry> for LauncherEntry {
    fn from(entry: DesktopEntry) -> Self {
        Self::from_desktop_entry(entry)
    }
}

/// Returns a default set of standard desktop system actions.
pub fn default_system_actions() -> Vec<LauncherEntry> {
    vec![
        LauncherEntry::custom_action(
            "action:poweroff",
            "Power Off",
            Some("system-shutdown".into()),
            Some("Shut down the computer".into()),
            vec![
                "shutdown".into(),
                "power off".into(),
                "turn off".into(),
                "halt".into(),
            ],
            LauncherAction::Command {
                program: "systemctl".into(),
                args: vec!["poweroff".into()],
                terminal: false,
            },
        ),
        LauncherEntry::custom_action(
            "action:reboot",
            "Restart",
            Some("system-reboot".into()),
            Some("Restart the computer".into()),
            vec!["reboot".into(), "restart".into(), "reset".into()],
            LauncherAction::Command {
                program: "systemctl".into(),
                args: vec!["reboot".into()],
                terminal: false,
            },
        ),
        LauncherEntry::custom_action(
            "action:suspend",
            "Suspend",
            Some("system-suspend".into()),
            Some("Put computer into low-power sleep mode".into()),
            vec!["suspend".into(), "sleep".into(), "standby".into()],
            LauncherAction::Command {
                program: "systemctl".into(),
                args: vec!["suspend".into()],
                terminal: false,
            },
        ),
        LauncherEntry::custom_action(
            "action:lock",
            "Lock Screen",
            Some("system-lock-screen".into()),
            Some("Lock the current desktop session".into()),
            vec!["lock".into(), "lock screen".into()],
            LauncherAction::Command {
                program: "loginctl".into(),
                args: vec!["lock-session".into()],
                terminal: false,
            },
        ),
        LauncherEntry::custom_action(
            "action:logout",
            "Log Out",
            Some("system-log-out".into()),
            Some("End the current desktop session".into()),
            vec!["logout".into(), "log out".into(), "sign out".into()],
            LauncherAction::Command {
                program: "loginctl".into(),
                args: vec!["terminate-session".into(), "".into()],
                terminal: false,
            },
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_from_desktop_entry() {
        let desktop = DesktopEntry {
            id: "firefox.desktop".into(),
            name: "Firefox".into(),
            exec: "firefox".into(),
            icon: Some("firefox".into()),
            comment: Some("Web browser".into()),
            generic_name: Some("Browser".into()),
            categories: vec!["Network".into()],
            keywords: vec!["web".into(), "internet".into()],
            path: PathBuf::from("/usr/share/applications/firefox.desktop"),
            terminal: false,
            search_haystack: "Firefox Browser firefox web internet Network Web browser".into(),
        };

        let launcher_entry = LauncherEntry::from(desktop);
        assert_eq!(launcher_entry.id, "firefox.desktop");
        assert_eq!(launcher_entry.name, "Firefox");
        assert_eq!(launcher_entry.description, Some("Browser".into()));
        assert!(launcher_entry.aliases.contains(&"web".to_string()));
        assert!(launcher_entry.aliases.contains(&"Network".to_string()));
        assert!(launcher_entry.as_desktop_entry().is_some());
        assert!(!launcher_entry.is_custom_action());
    }

    #[test]
    fn test_custom_action_creation() {
        let action = LauncherEntry::custom_action(
            "action:custom",
            "My Action",
            Some("icon".into()),
            Some("Desc".into()),
            vec!["alias1".into(), "alias2".into()],
            LauncherAction::Shell("echo hello".into()),
        );

        assert_eq!(action.id, "action:custom");
        assert!(action.is_custom_action());
        assert!(action.search_haystack.contains("My Action"));
        assert!(action.search_haystack.contains("alias1"));
        assert!(action.search_haystack.contains("echo hello"));
    }

    #[test]
    fn test_default_system_actions() {
        let actions = default_system_actions();
        assert_eq!(actions.len(), 5);

        let poweroff = actions.iter().find(|a| a.id == "action:poweroff").unwrap();
        assert_eq!(poweroff.name, "Power Off");
        assert!(poweroff.aliases.contains(&"shutdown".to_string()));
    }
}
