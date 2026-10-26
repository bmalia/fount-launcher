use crate::entry::LauncherEntry;
use nucleo::pattern::{CaseMatching, Normalization};
use nucleo::{Config, Matcher, Nucleo, Utf32String};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// A ranked search result containing the matched launcher entry, match score, and character indices.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResult {
    /// The matching launcher entry (application or custom action).
    pub entry: LauncherEntry,
    /// Score computed by fuzzy matcher (higher is better).
    pub score: u32,
    /// Character indices in haystack that matched the query, for UI highlighting.
    pub matched_indices: Vec<u32>,
}

/// High-performance fuzzy matching engine wrapping `nucleo::Nucleo<LauncherEntry>`.
pub struct SearchMatcher {
    nucleo: Nucleo<LauncherEntry>,
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

    /// Creates a `SearchMatcher` populated with the given launcher entries.
    pub fn from_entries(entries: Vec<LauncherEntry>) -> Self {
        let mut matcher = Self::new();
        matcher.reload(entries);
        matcher
    }

    /// Reloads the matcher with a fresh set of launcher entries.
    pub fn reload(&mut self, entries: Vec<LauncherEntry>) {
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
        self.results_with_frecency(limit, |_| 0.0)
    }

    /// Extracts the top `limit` ranked results, combining fuzzy scoring with a frecency bonus.
    ///
    /// When the query is empty, results are ordered purely by frecency score descending.
    /// When the query is non-empty, candidates are boosted: `score * (1.0 + 0.15 * ln(1.0 + frecency))`.
    pub fn results_with_frecency(
        &mut self,
        limit: usize,
        frecency_fn: impl Fn(&str) -> f32,
    ) -> Vec<SearchResult> {
        let snap = self.nucleo.snapshot();
        let total_matched = snap.matched_item_count() as usize;

        if total_matched == 0 {
            return Vec::new();
        }

        // When query is empty, return top entries sorted by frecency score
        if self.current_query.is_empty() {
            let mut candidates: Vec<(LauncherEntry, f32)> = Vec::with_capacity(total_matched);
            for item in snap.matched_items(0..total_matched as u32) {
                let f_score = frecency_fn(&item.data.id);
                candidates.push((item.data.clone(), f_score));
            }
            candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            candidates.truncate(limit);
            return candidates
                .into_iter()
                .map(|(entry, score)| SearchResult {
                    entry,
                    score: score.round() as u32,
                    matched_indices: Vec::new(),
                })
                .collect();
        }

        // Pool candidate matches from Nucleo snapshot
        let pool_size = (limit * 4).max(64).min(total_matched) as u32;
        let mut results = Vec::with_capacity(pool_size as usize);

        for item in snap.matched_items(0..pool_size) {
            let mut matched_indices = Vec::new();
            let mut score = 0;

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

            let frecency = frecency_fn(&item.data.id);
            let final_score = if frecency > 0.0 && score > 0 {
                (score as f32 * (1.0 + 0.15 * (1.0 + frecency).ln())).round() as u32
            } else {
                score
            };

            results.push(SearchResult {
                entry: item.data.clone(),
                score: final_score,
                matched_indices,
            });
        }

        // Re-sort candidate pool with blended scores
        results.sort_by_key(|a| std::cmp::Reverse(a.score));
        results.truncate(limit);
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
    use crate::entry::LauncherAction;

    fn make_test_entry(id: &str, name: &str, aliases: Vec<&str>) -> LauncherEntry {
        let al: Vec<String> = aliases.into_iter().map(|s| s.to_string()).collect();
        LauncherEntry::custom_action(
            id,
            name,
            Some(id.to_string()),
            Some(format!("Run {name}")),
            al,
            LauncherAction::Exec {
                command: id.to_string(),
                terminal: false,
            },
        )
    }

    #[test]
    fn test_search_matcher_ranking() {
        let entries = vec![
            make_test_entry("term", "GNOME Terminal", vec!["shell", "command"]),
            make_test_entry("code", "Visual Studio Code", vec!["editor", "development"]),
            make_test_entry("poweroff", "Power Off", vec!["shutdown", "turn off"]),
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

        // Search by alias: "shutdown" should rank "poweroff" first!
        matcher.update_query("shutdown");
        matcher.tick(50);
        let results = matcher.results(10);
        assert!(!results.is_empty());
        assert_eq!(results[0].entry.id, "poweroff");
        assert_eq!(results[0].entry.name, "Power Off");

        // Search non-existent
        matcher.update_query("xyz999nonsense");
        matcher.tick(50);
        let results = matcher.results(10);
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_matcher_reload() {
        let entries = vec![make_test_entry("one", "App One", vec![])];
        let mut matcher = SearchMatcher::from_entries(entries);
        assert_eq!(matcher.item_count(), 1);

        let new_entries = vec![
            make_test_entry("two", "App Two", vec![]),
            make_test_entry("three", "App Three", vec![]),
        ];
        matcher.reload(new_entries);
        assert_eq!(matcher.item_count(), 2);

        matcher.update_query("three");
        matcher.tick(50);
        let results = matcher.results(10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entry.id, "three");
    }

    #[test]
    fn test_frecency_ranking_and_empty_query() {
        let entries = vec![
            make_test_entry("gimp", "GNU Image Manipulation Program", vec!["gimp"]),
            make_test_entry("git-gui", "Git GUI", vec!["git"]),
            make_test_entry("calc", "Calculator", vec!["calc"]),
        ];
        let mut matcher = SearchMatcher::from_entries(entries);

        // Simulated frecency table: "git-gui" is heavily used, "gimp" is rarely used, "calc" is unused
        let frecency_map = std::collections::HashMap::from([
            ("git-gui".to_string(), 500.0),
            ("gimp".to_string(), 50.0),
        ]);
        let frecency_fn = |id: &str| *frecency_map.get(id).unwrap_or(&0.0);

        // 1. Empty query should return entries sorted by frecency score: git-gui first, then gimp
        matcher.update_query("");
        matcher.tick(50);
        let empty_results = matcher.results_with_frecency(5, frecency_fn);
        assert_eq!(empty_results.len(), 3);
        assert_eq!(empty_results[0].entry.id, "git-gui");
        assert_eq!(empty_results[1].entry.id, "gimp");

        // 2. Query "gi": both match "gi", but git-gui's frecency boosts it to #1
        matcher.update_query("gi");
        matcher.tick(50);
        let search_results = matcher.results_with_frecency(5, frecency_fn);
        assert_eq!(search_results[0].entry.id, "git-gui");
    }
}
