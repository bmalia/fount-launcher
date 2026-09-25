//! `fount-core`: Fast, pure synchronous desktop search and indexing library.
//!
//! Provides XDG desktop file scanning, parsing, in-memory cataloging, and fuzzy ranking
//! via `nucleo`. Strictly free of any async runtime, D-Bus, or network dependencies.

pub mod desktop_entry;
pub mod index;
pub mod matcher;

pub use desktop_entry::{
    clean_exec, get_application_search_dirs, scan_desktop_entries, scan_desktop_entries_in_dirs,
    DesktopEntry,
};
pub use index::IndexCatalog;
pub use matcher::{SearchMatcher, SearchResult};
