//! `fount-core`: Fast, pure synchronous desktop search and indexing library.
//!
//! Provides XDG desktop file scanning, parsing, custom action definitions, in-memory cataloging,
//! and fuzzy ranking via `nucleo`. Strictly free of any async runtime, D-Bus, or network dependencies.

pub mod config_manager;
pub mod desktop_entry;
pub mod entry;
pub mod index;
pub mod matcher;

pub use config_manager::{ConfigManager, CustomActionDef, CustomEntryConfig, LauncherConfig};
pub use desktop_entry::{
    DesktopEntry, clean_exec, get_application_search_dirs, scan_desktop_entries,
    scan_desktop_entries_in_dirs,
};
pub use entry::{EntryKind, LauncherAction, LauncherEntry, default_system_actions};
pub use index::IndexCatalog;
pub use matcher::{SearchMatcher, SearchResult};
