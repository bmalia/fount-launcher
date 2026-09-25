use crate::desktop_entry::DesktopEntry;
use nucleo::pattern::{CaseMatching, Normalization};
use nucleo::{Config, Matcher, Nucleo, Utf32String};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// A ranked search result containing the matched desktop entry, match score, and character indices.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResult {
    /// The matching desktop entry.
    pub entry: DesktopEntry,
    /// Score computed by fuzzy matcher (higher is better).
    pub score: u32,
    /// Character indices in `entry.name` (or haystack) that matched the query, for UI highlighting.
    pub matched_indices: Vec<u32>,
}

/// High-performance fuzzy matching engine wrapping `nucleo::Nucleo`.
pub struct SearchMatcher {
    nucleo: Nucleo<DesktopEntry>,
    matcher: Matcher,
    current_query: String,
    total_items: usize,
}

impl SearchMatcher {
    /// Creates a new empty `SearchMatcher`.
    pub fn new() -> Self {
        let config = Config::DEFAULT;
        let nucleo = Nucleo::new(config.clone(), Arc::new(|| {}), None, 1);
        let matcher = Matcher::new(config);
        Self {
            nucleo,
            matcher,
            current_query: String::new(),
            total_items: 0,
        }
    }

    /// Creates a `SearchMatcher` populated with the given desktop entries.
    pub fn from_entries(entries: Vec<DesktopEntry>) -> Self {
        let mut matcher = Self::new();
        matcher.reload(entries);
        matcher
    }

    /// Reloads the matcher with a fresh set of desktop entries.
    pub fn reload(&mut self, entries: Vec<DesktopEntry>) {
        self.nucleo.restart(true);
        let injector = self.nucleo.injector();
        self.total_items = entries.len();

        for entry in entries {
            injector.push(entry, |data, cols| {
                cols[0] = Utf32String::from(data.search_haystack.as_str());
            });
        }

        // Re-apply query if one was set
        let query = std::mem::take(&mut self.current_query);
        self.update_query(&query);
    }

    /// Updates the search query pattern.
    pub fn update_query(&mut self, query: &str) {
        let query_str = query.trim();
        let append = !self.current_query.is_empty()
            && query_str.starts_with(&self.current_query)
            && query_str.len() > self.current_query.len();

        self.current_query = query_str.to_string();
        self.nucleo.pattern.reparse(
            0,
            query_str,
            CaseMatching::Smart,
            Normalization::Smart,
            append,
        );
    }

    /// Ticks the background scoring engine. Returns whether the snapshot changed.
    /// Recommended timeout: 10ms.
    pub fn tick(&mut self, timeout_ms: u64) -> bool {
        let status = self.nucleo.tick(timeout_ms);
        status.changed
    }

    /// Extracts the top `limit` ranked results from the current snapshot.
    pub fn results(&mut self, limit: usize) -> Vec<SearchResult> {
        let snap = self.nucleo.snapshot();
        let total_matched = snap.matched_item_count() as usize;
        let fetch_count = (limit.min(total_matched)) as u32;

        let mut results = Vec::with_capacity(fetch_count as usize);

        for item in snap.matched_items(0..fetch_count) {
            let mut matched_indices = Vec::new();
            let mut score = 0;

            if !self.current_query.is_empty() {
                // Compute score and matched indices on haystack
                let pattern = snap.pattern().column_pattern(0);
                if let Some(s) = pattern.indices(
                    item.matcher_columns[0].slice(..),
                    &mut self.matcher,
                    &mut matched_indices,
                ) {
                    score = s;
                    matched_indices.sort_unstable();
                    matched_indices.dedup();
                }
            }

            results.push(SearchResult {
                entry: item.data.clone(),
                score,
                matched_indices,
            });
        }

        results
    }

    /// Returns the total number of items indexed.
    pub fn item_count(&self) -> usize {
        self.total_items
    }

    /// Returns the current active query.
    pub fn current_query(&self) -> &str {
        &self.current_query
    }
}

impl Default for SearchMatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_test_entry(id: &str, name: &str, exec: &str, keywords: Vec<&str>) -> DesktopEntry {
        let kw: Vec<String> = keywords.into_iter().map(|s| s.to_string()).collect();
        let search_haystack = format!("{} {} {}", name, exec, kw.join(" "));
        DesktopEntry {
            id: id.to_string(),
            name: name.to_string(),
            exec: exec.to_string(),
            icon: Some(id.to_string()),
            comment: None,
            generic_name: None,
            categories: vec![],
            keywords: kw,
            path: PathBuf::from(format!("/usr/share/applications/{}.desktop", id)),
            terminal: false,
            search_haystack,
        }
    }

    #[test]
    fn test_search_matcher_ranking() {
        let entries = vec![
            make_test_entry("term", "GNOME Terminal", "gnome-terminal", vec!["shell", "command"]),
            make_test_entry("code", "Visual Studio Code", "code", vec!["editor", "development"]),
            make_test_entry("calc", "Calculator", "gnome-calculator", vec!["math"]),
        ];

        let mut matcher = SearchMatcher::from_entries(entries);

        // Search "term"
        matcher.update_query("term");
        matcher.tick(50);
        let results = matcher.results(10);
        assert!(!results.is_empty());
        assert_eq!(results[0].entry.id, "term");
        assert!(results[0].score > 0);
        assert!(!results[0].matched_indices.is_empty());

        // Search "code"
        matcher.update_query("code");
        matcher.tick(50);
        let results = matcher.results(10);
        assert_eq!(results[0].entry.id, "code");

        // Search non-existent
        matcher.update_query("xyz999nonsense");
        matcher.tick(50);
        let results = matcher.results(10);
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_matcher_reload() {
        let entries = vec![make_test_entry("one", "App One", "app1", vec![])];
        let mut matcher = SearchMatcher::from_entries(entries);
        assert_eq!(matcher.item_count(), 1);

        let new_entries = vec![
            make_test_entry("two", "App Two", "app2", vec![]),
            make_test_entry("three", "App Three", "app3", vec![]),
        ];
        matcher.reload(new_entries);
        assert_eq!(matcher.item_count(), 2);

        matcher.update_query("three");
        matcher.tick(50);
        let results = matcher.results(10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entry.id, "three");
    }
}
