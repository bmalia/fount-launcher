use crate::desktop_entry::{scan_desktop_entries, scan_desktop_entries_in_dirs, DesktopEntry};
use crate::matcher::{SearchMatcher, SearchResult};
use std::collections::HashMap;
use std::path::PathBuf;

/// Catalog storing in-memory indexed desktop entries and managing the fuzzy search matcher.
pub struct IndexCatalog {
    entries: Vec<DesktopEntry>,
    by_id: HashMap<String, usize>,
    matcher: SearchMatcher,
    custom_dirs: Option<Vec<PathBuf>>,
}

impl IndexCatalog {
    /// Creates a new catalog and immediately scans standard XDG application directories.
    pub fn new() -> Self {
        let mut catalog = Self {
            entries: Vec::new(),
            by_id: HashMap::new(),
            matcher: SearchMatcher::new(),
            custom_dirs: None,
        };
        catalog.reload();
        catalog
    }

    /// Creates a catalog that scans specified custom directories instead of system defaults.
    pub fn with_dirs(dirs: Vec<PathBuf>) -> Self {
        let mut catalog = Self {
            entries: Vec::new(),
            by_id: HashMap::new(),
            matcher: SearchMatcher::new(),
            custom_dirs: Some(dirs),
        };
        catalog.reload();
        catalog
    }

    /// Reloads all desktop applications from disk into the catalog and matcher.
    /// Returns the number of loaded applications.
    pub fn reload(&mut self) -> usize {
        let scanned = match &self.custom_dirs {
            Some(dirs) => scan_desktop_entries_in_dirs(dirs),
            None => scan_desktop_entries(),
        };

        self.by_id.clear();
        for (idx, entry) in scanned.iter().enumerate() {
            self.by_id.insert(entry.id.clone(), idx);
        }

        self.matcher.reload(scanned.clone());
        self.entries = scanned;
        self.entries.len()
    }

    /// Finds an entry by its desktop file ID.
    pub fn get_by_id(&self, id: &str) -> Option<&DesktopEntry> {
        let &idx = self.by_id.get(id)?;
        self.entries.get(idx)
    }

    /// Returns a slice of all indexed desktop entries.
    pub fn entries(&self) -> &[DesktopEntry] {
        &self.entries
    }

    /// Total count of indexed applications.
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
    fn test_live_system_scan() {
        let entries = crate::desktop_entry::scan_desktop_entries();
        println!("Scanned {} live desktop applications on host system", entries.len());
        assert!(!entries.is_empty(), "Should find desktop applications on a Linux host");

        let mut catalog = IndexCatalog::new();
        assert!(!catalog.is_empty());

        catalog.search("sh");
        catalog.tick(50);
        let results = catalog.results(5);
        println!("Search 'sh' found {} matches:", results.len());
        for res in &results {
            println!(" - {} (exec: {}, score: {})", res.entry.name, res.entry.exec, res.score);
        }
    }
}
