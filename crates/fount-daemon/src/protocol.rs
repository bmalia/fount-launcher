use fount_core::{LauncherAction, SearchResult};
use serde::{Deserialize, Serialize};

/// Requests sent from an IPC client (Unix socket or internal bridge) to the daemon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClientMessage {
    /// Query the fuzzy search engine for up to `limit` results.
    Search { query: String, limit: u32 },
    /// Trigger execution of an entry by its ID and record usage in the frecency store.
    Exec {
        id: String,
        /// If true, the daemon directly executes the entry's action; if false, only records frecency and assumes the client will handle execution.
        execute: bool,
    },
    /// Rescan desktop directories and reload configuration.
    Reload,
    /// Health check ping.
    Ping,
}

/// Responses sent from the daemon back to an IPC client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DaemonMessage {
    /// Ranked search results matching `query`.
    Results {
        query: String,
        items: Vec<SearchResultWire>,
    },
    /// Outcome of a `Launch` request.
    Launched {
        id: String,
        success: bool,
        error: Option<String>,
    },
    /// Outcome of a `Reload` request with the total number of indexed entries.
    Reloaded { count: u32 },
    /// Health check response.
    Pong,
    /// Error response if a request could not be processed.
    Error { message: String },
}

/// Compact wire representation of a ranked search result for IPC transport.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResultWire {
    /// Unique identifier of the entry (e.g. `firefox.desktop` or `action:poweroff`).
    pub id: String,
    /// Display title.
    pub name: String,
    /// Secondary subtitle / generic name / comment.
    pub description: Option<String>,
    /// Icon name or file path.
    pub icon: Option<String>,
    /// Executable command or action summary string.
    pub exec: String,
    /// Whether this entry is a custom action rather than a standard application.
    pub is_custom_action: bool,
    /// Blended fuzzy + frecency score.
    pub score: u32,
    /// Matched character indices for UI highlighting.
    pub matched_indices: Vec<u32>,
}

impl From<SearchResult> for SearchResultWire {
    fn from(res: SearchResult) -> Self {
        let exec = match &res.entry.action {
            LauncherAction::Exec { command, .. } => command.clone(),
            LauncherAction::Command { program, args, .. } => {
                if args.is_empty() {
                    program.clone()
                } else {
                    format!("{} {}", program, args.join(" "))
                }
            }
            LauncherAction::Shell(script) => script.clone(),
            LauncherAction::Custom(custom_id) => custom_id.clone(),
        };

        let is_custom_action = res.entry.is_custom_action();

        Self {
            id: res.entry.id,
            name: res.entry.name,
            description: res.entry.description,
            icon: res.entry.icon,
            exec,
            is_custom_action,
            score: res.score,
            matched_indices: res.matched_indices,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fount_core::LauncherEntry;

    #[test]
    fn test_bincode_and_json_roundtrip() {
        let req = ClientMessage::Search {
            query: "fire".into(),
            limit: 10,
        };

        let bytes = bincode::serialize(&req).unwrap();
        let decoded: ClientMessage = bincode::deserialize(&bytes).unwrap();
        assert_eq!(req, decoded);

        let json = serde_json::to_string(&req).unwrap();
        let decoded_json: ClientMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(req, decoded_json);
    }

    #[test]
    fn test_search_result_wire_conversion() {
        let entry = LauncherEntry::custom_action(
            "action:poweroff",
            "Power Off",
            Some("system-shutdown".into()),
            Some("Shut down the system".into()),
            vec!["shutdown".into()],
            LauncherAction::Command {
                program: "systemctl".into(),
                args: vec!["poweroff".into()],
                terminal: false,
            },
        );
        let res = SearchResult {
            entry,
            score: 140,
            matched_indices: vec![0, 1, 2],
        };

        let wire = SearchResultWire::from(res);
        assert_eq!(wire.id, "action:poweroff");
        assert_eq!(wire.name, "Power Off");
        assert_eq!(wire.exec, "systemctl poweroff");
        assert!(wire.is_custom_action);
        assert_eq!(wire.score, 140);
        assert_eq!(wire.matched_indices, vec![0, 1, 2]);
    }
}
