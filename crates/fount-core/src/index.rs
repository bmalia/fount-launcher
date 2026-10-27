use crate::config_manager::{ConfigManager, LauncherConfig, default_config_path};
use crate::desktop_entry::{scan_desktop_entries, scan_desktop_entries_in_dirs};
use crate::entry::{LauncherEntry, default_system_actions};
use crate::frecency::{FrecencyStore, current_unix_timestamp};
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
    config_path: Option<PathBuf>,
    frecency: FrecencyStore,
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
            config_path: None,
            frecency: FrecencyStore::open_default(),
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
            config_path: None,
            frecency: FrecencyStore::open_default(),
        };
        catalog.reload();
        catalog
    }

    /// Creates a catalog backed by `$XDG_CONFIG_HOME/fount/config.json` (or `~/.config/fount/config.json`).
    /// Calling `reload()` on this catalog will also hot-reload changes made to the config file.
    pub fn from_default_config() -> Self {
        Self::from_config_file(default_config_path())
    }

    /// Creates a catalog backed by a specific JSON configuration file path.
    /// If the file does not exist or cannot be parsed yet, falls back to `LauncherConfig::default()`.
    pub fn from_config_file(path: PathBuf) -> Self {
        let config = ConfigManager::load_from_file(&path).unwrap_or_default();
        let mut catalog = Self::with_config(config);
        catalog.config_path = Some(path);
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
        let mut matcher = SearchMatcher::new();
        matcher.set_min_score_ratio(config.min_score_ratio);
        let mut catalog = Self {
            entries: Vec::new(),
            custom_entries,
            by_id: HashMap::new(),
            matcher,
            custom_dirs,
            include_system_actions: config.include_system_actions,
            config_path: None,
            frecency: FrecencyStore::open_default(),
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
        self.matcher.set_min_score_ratio(config.min_score_ratio);
        self.reload()
    }

    /// Sets the relative score threshold ratio (`0.0..=1.0`) for filtering weak matches.
    pub fn set_min_score_ratio(&mut self, ratio: f32) {
        self.matcher.set_min_score_ratio(ratio);
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
    /// If associated with a `config_path`, also hot-reloads configuration changes from disk.
    /// Returns the total number of indexed entries.
    pub fn reload(&mut self) -> usize {
        if let Some(ref path) = self.config_path
            && let Ok(config) = ConfigManager::load_from_file(path)
        {
            self.custom_entries = config.to_launcher_entries();
            self.custom_dirs = if config.custom_dirs.is_empty() {
                None
            } else {
                Some(config.custom_dirs)
            };
            self.include_system_actions = config.include_system_actions;
            self.matcher.set_min_score_ratio(config.min_score_ratio);
        }

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

    /// Retrieves up to `limit` ranked search results, applying frecency boosting.
    pub fn results(&mut self, limit: usize) -> Vec<SearchResult> {
        let now = current_unix_timestamp();
        self.matcher
            .results_with_frecency(limit, |id| self.frecency.get_score(id, now))
    }

    /// Returns a reference to the frecency store.
    pub fn frecency(&self) -> &FrecencyStore {
        &self.frecency
    }

    /// Returns a mutable reference to the frecency store.
    pub fn frecency_mut(&mut self) -> &mut FrecencyStore {
        &mut self.frecency
    }

    /// Replaces the frecency store (useful for tests or custom paths).
    pub fn set_frecency_store(&mut self, store: FrecencyStore) {
        self.frecency = store;
    }

    /// Records a launch event for the specified entry ID, boosting its future ranking.
    pub fn record_launch(&mut self, id: &str) -> f32 {
        self.frecency.record_launch(id)
    }

    /// Persists the frecency state to disk atomically.
    pub fn save_frecency(&self) -> std::io::Result<()> {
        self.frecency.save()
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

    #[test]
    fn test_catalog_frecency_integration() {
        let temp = tempdir().unwrap();
        let apps_dir = temp.path().join("applications");
        let frecency_file = temp.path().join("frecency.bin");
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

        fs::write(
            apps_dir.join("calendar.desktop"),
            r#"
[Desktop Entry]
Type=Application
Name=Calendar
Exec=calendar
"#,
        )
        .unwrap();

        let mut catalog = IndexCatalog::with_dirs(vec![apps_dir]);
        catalog.set_frecency_store(FrecencyStore::open_or_create(frecency_file.clone()));
        assert_eq!(catalog.len(), 2);

        // Record launch for calendar 5 times
        for _ in 0..5 {
            catalog.record_launch("calendar.desktop");
        }
        catalog.save_frecency().unwrap();
        assert!(frecency_file.is_file());

        // Empty query search should return Calendar as #1 because of frecency
        catalog.search("");
        catalog.tick(50);
        let results = catalog.results(2);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].entry.id, "calendar.desktop");

        // Search "cal": both match "cal", but Calendar has higher frecency score
        catalog.search("cal");
        catalog.tick(50);
        let results = catalog.results(2);
        assert_eq!(results[0].entry.id, "calendar.desktop");
    }

    #[test]
    fn test_catalog_from_config_file_and_hot_reload() {
        let temp = tempdir().unwrap();
        let apps_dir = temp.path().join("applications");
        let config_file = temp.path().join("config.json");
        fs::create_dir_all(&apps_dir).unwrap();

        fs::write(
            &config_file,
            format!(
                r#"{{
                    "custom_dirs": ["{}"],
                    "include_system_actions": false,
                    "custom_actions": [
                        {{
                            "id": "action:one",
                            "name": "Action One",
                            "action": {{ "type": "exec", "command": "echo 1" }}
                        }}
                    ]
                }}"#,
                apps_dir.display()
            ),
        )
        .unwrap();

        let mut catalog = IndexCatalog::from_config_file(config_file.clone());
        assert_eq!(catalog.len(), 1);
        assert!(catalog.get_by_id("action:one").is_some());

        // Modify config.json on disk to add a second custom action and reload
        fs::write(
            &config_file,
            format!(
                r#"{{
                    "custom_dirs": ["{}"],
                    "include_system_actions": false,
                    "custom_actions": [
                        {{
                            "id": "action:one",
                            "name": "Action One",
                            "action": {{ "type": "exec", "command": "echo 1" }}
                        }},
                        {{
                            "id": "action:two",
                            "name": "Action Two",
                            "action": {{ "type": "exec", "command": "echo 2" }}
                        }}
                    ]
                }}"#,
                apps_dir.display()
            ),
        )
        .unwrap();

        let new_count = catalog.reload();
        assert_eq!(new_count, 2);
        assert!(catalog.get_by_id("action:two").is_some());
    }
}
