use crate::config_manager::LauncherConfig;
use crate::desktop_entry::{scan_desktop_entries, scan_desktop_entries_in_dirs};
use crate::entry::{LauncherEntry, default_system_actions};
use crate::matcher::{SearchMatcher, SearchResult};
use std::collections::HashMap;
use std::path::PathBuf;

/// Catalog storing in-memory indexed launcher entries (desktop apps + custom actions)
/// and managing the fuzzy search matcher.
pub struct IndexCatalog {
    entries: Vec<LauncherEntry>,
    custom_entries: Vec<LauncherEntry>,
    by_id: HashMap<String, usize>,
    matcher: SearchMatcher,
    custom_dirs: Option<Vec<PathBuf>>,
    include_system_actions: bool,
}

impl IndexCatalog {
    /// Creates a new catalog, immediately scanning standard XDG application directories.
    pub fn new() -> Self {
        let mut catalog = Self {
            entries: Vec::new(),
            custom_entries: Vec::new(),
            by_id: HashMap::new(),
            matcher: SearchMatcher::new(),
            custom_dirs: None,
            include_system_actions: false,
        };
        catalog.reload();
        catalog
    }

    /// Creates a catalog scanning specified custom directories instead of system defaults.
    pub fn with_dirs(dirs: Vec<PathBuf>) -> Self {
        let mut catalog = Self {
            entries: Vec::new(),
            custom_entries: Vec::new(),
            by_id: HashMap::new(),
            matcher: SearchMatcher::new(),
            custom_dirs: Some(dirs),
            include_system_actions: false,
        };
        catalog.reload();
        catalog
    }

    /// Creates a catalog initialized with the given `LauncherConfig`.
    pub fn with_config(config: LauncherConfig) -> Self {
        let custom_entries = config.to_launcher_entries();
        let custom_dirs = if config.custom_dirs.is_empty() {
            None
        } else {
            Some(config.custom_dirs)
        };
        let mut catalog = Self {
            entries: Vec::new(),
            custom_entries,
            by_id: HashMap::new(),
            matcher: SearchMatcher::new(),
            custom_dirs,
            include_system_actions: config.include_system_actions,
        };
        catalog.reload();
        catalog
    }

    /// Updates catalog configuration and reloads indexed entries.
    pub fn apply_config(&mut self, config: LauncherConfig) -> usize {
        self.custom_entries = config.to_launcher_entries();
        self.custom_dirs = if config.custom_dirs.is_empty() {
            None
        } else {
            Some(config.custom_dirs)
        };
        self.include_system_actions = config.include_system_actions;
        self.reload()
    }

    /// Enables or disables inclusion of default system actions (Power Off, Restart, Lock, etc.).
    pub fn set_include_system_actions(&mut self, include: bool) -> usize {
        self.include_system_actions = include;
        self.reload()
    }

    /// Adds a custom action to the catalog and refreshes the search index.
    pub fn add_custom_entry(&mut self, entry: LauncherEntry) -> usize {
        self.custom_entries.push(entry);
        self.reload()
    }

    /// Adds multiple custom actions to the catalog and refreshes the search index.
    pub fn add_custom_entries(&mut self, entries: Vec<LauncherEntry>) -> usize {
        self.custom_entries.extend(entries);
        self.reload()
    }

    /// Reloads all desktop applications from disk and merges with custom and system actions.
    /// Returns the total number of indexed entries.
    pub fn reload(&mut self) -> usize {
        let scanned = match &self.custom_dirs {
            Some(dirs) => scan_desktop_entries_in_dirs(dirs),
            None => scan_desktop_entries(),
        };

        let mut all_entries: Vec<LauncherEntry> = scanned
            .into_iter()
            .map(LauncherEntry::from_desktop_entry)
            .collect();

        if self.include_system_actions {
            all_entries.extend(default_system_actions());
        }

        all_entries.extend(self.custom_entries.clone());

        self.by_id.clear();
        for (idx, entry) in all_entries.iter().enumerate() {
            self.by_id.insert(entry.id.clone(), idx);
        }

        self.matcher.reload(all_entries.clone());
        self.entries = all_entries;
        self.entries.len()
    }

    /// Finds an entry by its ID.
    pub fn get_by_id(&self, id: &str) -> Option<&LauncherEntry> {
        let &idx = self.by_id.get(id)?;
        self.entries.get(idx)
    }

    /// Returns a slice of all indexed launcher entries.
    pub fn entries(&self) -> &[LauncherEntry] {
        &self.entries
    }

    /// Total count of indexed entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the catalog is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Updates the query pattern in the fuzzy matcher.
    pub fn search(&mut self, query: &str) {
        self.matcher.update_query(query);
    }

    /// Drives the background fuzzy scoring.
    pub fn tick(&mut self, timeout_ms: u64) -> bool {
        self.matcher.tick(timeout_ms)
    }

    /// Retrieves up to `limit` ranked search results.
    pub fn results(&mut self, limit: usize) -> Vec<SearchResult> {
        self.matcher.results(limit)
    }
}

impl Default for IndexCatalog {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config_manager::ConfigManager;
    use crate::entry::LauncherAction;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_index_catalog_lifecycle() {
        let temp = tempdir().unwrap();
        let apps_dir = temp.path().join("applications");
        fs::create_dir_all(&apps_dir).unwrap();

        fs::write(
            apps_dir.join("app1.desktop"),
            r#"
[Desktop Entry]
Type=Application
Name=Text Editor
Exec=gedit
"#,
        )
        .unwrap();

        fs::write(
            apps_dir.join("app2.desktop"),
            r#"
[Desktop Entry]
Type=Application
Name=Browser
Exec=firefox
"#,
        )
        .unwrap();

        let mut catalog = IndexCatalog::with_dirs(vec![apps_dir.clone()]);
        assert_eq!(catalog.len(), 2);
        assert!(catalog.get_by_id("app1.desktop").is_some());

        // Perform search
        catalog.search("editor");
        catalog.tick(50);
        let results = catalog.results(5);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entry.name, "Text Editor");

        // Add a third file and reload
        fs::write(
            apps_dir.join("app3.desktop"),
            r#"
[Desktop Entry]
Type=Application
Name=Terminal
Exec=gnome-terminal
"#,
        )
        .unwrap();

        let count = catalog.reload();
        assert_eq!(count, 3);
        assert_eq!(catalog.len(), 3);

        catalog.search("term");
        catalog.tick(50);
        let results = catalog.results(5);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entry.name, "Terminal");
    }

    #[test]
    fn test_catalog_with_custom_and_system_actions() {
        let temp = tempdir().unwrap();
        let apps_dir = temp.path().join("applications");
        fs::create_dir_all(&apps_dir).unwrap();

        fs::write(
            apps_dir.join("calc.desktop"),
            r#"
[Desktop Entry]
Type=Application
Name=Calculator
Exec=calc
"#,
        )
        .unwrap();

        let mut catalog = IndexCatalog::with_dirs(vec![apps_dir]);
        assert_eq!(catalog.len(), 1);

        // Add custom action
        catalog.add_custom_entry(LauncherEntry::custom_action(
            "action:screenshot",
            "Take Screenshot",
            Some("camera-photo".into()),
            Some("Capture the screen".into()),
            vec!["screenshot".into(), "screen cap".into(), "snip".into()],
            LauncherAction::Command {
                program: "grim".into(),
                args: vec![],
                terminal: false,
            },
        ));
        assert_eq!(catalog.len(), 2);

        // Enable default system actions (5 actions)
        catalog.set_include_system_actions(true);
        assert_eq!(catalog.len(), 7);

        // Search for "shutdown" -> should find Power Off
        catalog.search("shutdown");
        catalog.tick(50);
        let results = catalog.results(5);
        assert!(!results.is_empty());
        assert_eq!(results[0].entry.id, "action:poweroff");
        assert_eq!(results[0].entry.name, "Power Off");

        // Search for "snip" -> should find Screenshot custom action
        catalog.search("snip");
        catalog.tick(50);
        let results = catalog.results(5);
        assert!(!results.is_empty());
        assert_eq!(results[0].entry.id, "action:screenshot");
    }

    #[test]
    fn test_catalog_with_config() {
        let temp = tempdir().unwrap();
        let apps_dir = temp.path().join("applications");
        fs::create_dir_all(&apps_dir).unwrap();

        let json = format!(
            r#"{{
            "custom_dirs": ["{}"],
            "include_system_actions": true,
            "custom_actions": [
                {{
                    "id": "action:notes",
                    "name": "Quick Notes",
                    "aliases": ["memo", "scratchpad"],
                    "action": {{
                        "type": "shell",
                        "script": "nvim ~/notes.txt"
                    }}
                }}
            ]
        }}"#,
            apps_dir.display()
        );

        let config = ConfigManager::load_from_str(&json).unwrap();
        let mut catalog = IndexCatalog::with_config(config);
        assert!(catalog.get_by_id("action:notes").is_some());
        assert!(catalog.get_by_id("action:poweroff").is_some());

        // Test search for the custom action from config
        catalog.search("scratchpad");
        catalog.tick(50);
        let results = catalog.results(5);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entry.id, "action:notes");
    }

    #[test]
    fn test_live_system_scan() {
        let entries = crate::desktop_entry::scan_desktop_entries();
        println!(
            "Scanned {} live desktop applications on host system",
            entries.len()
        );
        assert!(
            !entries.is_empty(),
            "Should find desktop applications on a Linux host"
        );

        let mut catalog = IndexCatalog::new();
        catalog.set_include_system_actions(true);
        assert!(!catalog.is_empty());

        // Search for "power"
        catalog.search("power");
        catalog.tick(50);
        let results = catalog.results(5);
        println!("Search 'power' found {} matches:", results.len());
        for res in &results {
            println!(
                " - {} (id: {}, score: {})",
                res.entry.name, res.entry.id, res.score
            );
        }
        assert!(results.iter().any(|r| r.entry.id == "action:poweroff"));
    }
}
