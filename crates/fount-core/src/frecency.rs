use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Usage statistics for a single launcher entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryStats {
    /// Total number of times this entry has been selected.
    pub launch_count: u32,
    /// Unix timestamp in seconds of the most recent launch.
    pub last_launched_at: u64,
}

impl EntryStats {
    /// Computes the frecency score based on frequency and recency decay buckets.
    pub fn score(&self, now_secs: u64) -> f32 {
        let age_secs = now_secs.saturating_sub(self.last_launched_at);
        let age_hours = age_secs as f32 / 3600.0;

        // Recency decay multiplier
        let recency_multiplier = if age_hours < 4.0 {
            100.0 // Launched within the last 4 hours
        } else if age_hours < 24.0 {
            80.0  // Launched within the last 24 hours
        } else if age_hours < 24.0 * 7.0 {
            50.0  // Launched within the last week
        } else if age_hours < 24.0 * 30.0 {
            30.0  // Launched within the last month
        } else {
            10.0  // Older than a month
        };

        (self.launch_count as f32) * recency_multiplier
    }
}

/// Helper returning the current Unix timestamp in seconds.
pub fn current_unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Resolves the standard XDG state path: `$XDG_STATE_HOME/fount/frecency.bin` (or `~/.local/state/fount/frecency.bin`).
pub fn default_frecency_path() -> PathBuf {
    if let Ok(state_home) = std::env::var("XDG_STATE_HOME")
        && !state_home.is_empty()
    {
        return PathBuf::from(state_home).join("fount").join("frecency.bin");
    }
    if let Ok(home) = std::env::var("HOME")
        && !home.is_empty()
    {
        return PathBuf::from(home)
            .join(".local")
            .join("state")
            .join("fount")
            .join("frecency.bin");
    }
    PathBuf::from("/tmp/fount_frecency.bin")
}

/// High-performance in-memory frecency store persisted atomically using `bincode`.
#[derive(Debug, Clone, Default)]
pub struct FrecencyStore {
    entries: HashMap<String, EntryStats>,
    path: Option<PathBuf>,
}

impl FrecencyStore {
    /// Creates an empty in-memory store without an associated file path.
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            path: None,
        }
    }

    /// Loads or initializes a store from the default XDG state path.
    pub fn open_default() -> Self {
        Self::open_or_create(default_frecency_path())
    }

    /// Opens an existing `frecency.bin` file at `path`, or creates a new empty store associated with `path`.
    pub fn open_or_create(path: PathBuf) -> Self {
        if path.is_file()
            && let Ok(store) = Self::load_from_path(path.clone())
        {
            return store;
        }
        Self {
            entries: HashMap::new(),
            path: Some(path),
        }
    }

    /// Loads and deserializes a `FrecencyStore` from disk via `bincode`.
    pub fn load_from_path(path: PathBuf) -> io::Result<Self> {
        let bytes = fs::read(&path)?;
        let entries: HashMap<String, EntryStats> = bincode::deserialize(&bytes)
            .map_err(|e| io::Error::other(format!("Failed to deserialize frecency database: {e}")))?;

        Ok(Self {
            entries,
            path: Some(path),
        })
    }

    /// Atomically persists the in-memory frecency map to disk via `bincode`.
    ///
    /// Writes to a sibling temporary file (`.tmp`) and performs an atomic `fs::rename` to prevent corruption.
    pub fn save(&self) -> io::Result<()> {
        if let Some(ref path) = self.path {
            self.save_to_path(path)?;
        }
        Ok(())
    }

    /// Atomically saves the store to a specific path.
    pub fn save_to_path(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let tmp_path = match path.file_name() {
            Some(fname) => {
                let mut tmp_name = fname.to_os_string();
                tmp_name.push(".tmp");
                path.with_file_name(tmp_name)
            }
            None => path.with_extension("tmp"),
        };

        let bytes = bincode::serialize(&self.entries)
            .map_err(|e| io::Error::other(format!("Failed to serialize frecency database: {e}")))?;

        fs::write(&tmp_path, bytes)?;
        fs::rename(&tmp_path, path)?;

        Ok(())
    }

    /// Records a launch event for the given entry ID, updating launch count and timestamp.
    /// Returns the updated frecency score.
    pub fn record_launch(&mut self, id: &str) -> f32 {
        let now = current_unix_timestamp();
        let stats = self.entries.entry(id.to_string()).or_insert(EntryStats {
            launch_count: 0,
            last_launched_at: now,
        });

        stats.launch_count += 1;
        stats.last_launched_at = now;
        stats.score(now)
    }

    /// Records a launch event with an explicit timestamp (useful for testing or historical imports).
    pub fn record_launch_at(&mut self, id: &str, timestamp: u64) -> f32 {
        let stats = self.entries.entry(id.to_string()).or_insert(EntryStats {
            launch_count: 0,
            last_launched_at: timestamp,
        });

        stats.launch_count += 1;
        stats.last_launched_at = timestamp;
        stats.score(timestamp)
    }

    /// Gets the frecency score for a given entry ID at the specified reference timestamp.
    pub fn get_score(&self, id: &str, now_secs: u64) -> f32 {
        self.entries
            .get(id)
            .map(|s| s.score(now_secs))
            .unwrap_or(0.0)
    }

    /// Returns a reference to an entry's stats if tracked.
    pub fn get_stats(&self, id: &str) -> Option<&EntryStats> {
        self.entries.get(id)
    }

    /// Returns a reference to all tracked entries.
    pub fn all_stats(&self) -> &HashMap<String, EntryStats> {
        &self.entries
    }

    /// Number of tracked entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Clears all tracked entries.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_frecency_decay() {
        let now = 10_000_000;
        let recent = EntryStats {
            launch_count: 5,
            last_launched_at: now - 3600, // 1 hour ago
        };
        let older = EntryStats {
            launch_count: 5,
            last_launched_at: now - (24 * 3600 * 2), // 2 days ago
        };
        let very_old = EntryStats {
            launch_count: 5,
            last_launched_at: now - (24 * 3600 * 40), // 40 days ago
        };

        assert!(recent.score(now) > older.score(now));
        assert!(older.score(now) > very_old.score(now));
        assert_eq!(recent.score(now), 5.0 * 100.0);
        assert_eq!(older.score(now), 5.0 * 50.0);
        assert_eq!(very_old.score(now), 5.0 * 10.0);
    }

    #[test]
    fn test_bincode_roundtrip_and_persistence() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("frecency.bin");

        let mut store = FrecencyStore::open_or_create(file_path.clone());
        assert_eq!(store.len(), 0);

        store.record_launch_at("firefox.desktop", 1000);
        store.record_launch_at("firefox.desktop", 2000);
        store.record_launch_at("code.desktop", 1500);

        assert_eq!(store.len(), 2);
        assert_eq!(store.get_stats("firefox.desktop").unwrap().launch_count, 2);

        // Save atomically to disk
        store.save().expect("Failed to save frecency store");
        assert!(file_path.is_file());

        // Reload into a separate store instance
        let reloaded = FrecencyStore::load_from_path(file_path).expect("Failed to reload store");
        assert_eq!(reloaded.len(), 2);
        let firefox_stats = reloaded.get_stats("firefox.desktop").unwrap();
        assert_eq!(firefox_stats.launch_count, 2);
        assert_eq!(firefox_stats.last_launched_at, 2000);
    }
}
