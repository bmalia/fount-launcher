use freedesktop_desktop_entry::DesktopEntry as FdDesktopEntry;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::env;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Represents a parsed Linux XDG desktop entry (.desktop file).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopEntry {
    /// Desktop file ID (e.g. `org.gnome.Terminal.desktop` or `firefox.desktop`).
    pub id: String,
    /// Localized or default display name of the application.
    pub name: String,
    /// Cleaned executable command (field codes like `%u`, `%F` stripped).
    pub exec: String,
    /// Icon name or path (e.g. `firefox` or `/usr/share/icons/...`).
    pub icon: Option<String>,
    /// Tooltip or description.
    pub comment: Option<String>,
    /// Generic category description (e.g. "Web Browser").
    pub generic_name: Option<String>,
    /// List of categories (e.g. `["Network", "WebBrowser"]`).
    pub categories: Vec<String>,
    /// List of search keywords.
    pub keywords: Vec<String>,
    /// Path on disk to the `.desktop` file.
    pub path: PathBuf,
    /// Whether the app should be run inside a terminal emulator.
    pub terminal: bool,
    /// Combined searchable string used by fuzzy matchers.
    pub search_haystack: String,
}

impl DesktopEntry {
    /// Parses a `.desktop` file from disk given its file path and desktop ID.
    pub fn from_file<P: AsRef<Path>>(path: P, id: String) -> Option<Self> {
        let path_ref = path.as_ref();
        let content = std::fs::read_to_string(path_ref).ok()?;
        Self::from_str(&content, path_ref.to_path_buf(), id)
    }

    /// Parses a desktop entry from string content.
    pub fn from_str(content: &str, path: PathBuf, id: String) -> Option<Self> {
        let entry = FdDesktopEntry::from_str(path.clone(), content, None::<&[&str]>).ok()?;

        // Rule: Must be an Application
        if entry.type_() != Some("Application") {
            return None;
        }

        // Rule: Respect NoDisplay and Hidden
        if entry.no_display() || entry.hidden() {
            return None;
        }

        // Rule: Respect OnlyShowIn and NotShowIn for current desktop
        if !is_compatible_desktop(&entry) {
            return None;
        }

        // Rule: Respect TryExec if present
        if let Some(try_exec) = entry.try_exec()
            && !is_executable_available(try_exec)
        {
            return None;
        }

        let locales: &[&str] = &[];
        let name = entry.name(locales)?.to_string();
        let raw_exec = entry.exec()?.to_string();
        let exec = clean_exec(&raw_exec);
        let icon = entry.icon().map(|s| s.to_string());
        let comment = entry.comment(locales).map(|s| s.to_string());
        let generic_name = entry.generic_name(locales).map(|s| s.to_string());

        let categories: Vec<String> = entry
            .categories()
            .map(|cats| {
                cats.into_iter()
                    .map(|s| s.to_string())
                    .filter(|s| !s.trim().is_empty())
                    .collect()
            })
            .unwrap_or_default();

        let keywords: Vec<String> = entry
            .keywords(locales)
            .map(|kw| {
                kw.into_iter()
                    .map(|s| s.to_string())
                    .filter(|s| !s.trim().is_empty())
                    .collect()
            })
            .unwrap_or_default();

        let terminal = entry.terminal();

        // Build composite search haystack:
        // Priority order in string: Name > GenericName > Exec > Keywords > Categories > Comment
        let mut haystack_parts = vec![name.clone()];
        if let Some(ref gn) = generic_name {
            haystack_parts.push(gn.clone());
        }
        haystack_parts.push(exec.clone());
        if !keywords.is_empty() {
            haystack_parts.push(keywords.join(" "));
        }
        if !categories.is_empty() {
            haystack_parts.push(categories.join(" "));
        }
        if let Some(ref c) = comment {
            haystack_parts.push(c.clone());
        }
        let search_haystack = haystack_parts.join(" ");

        Some(Self {
            id,
            name,
            exec,
            icon,
            comment,
            generic_name,
            categories,
            keywords,
            path,
            terminal,
            search_haystack,
        })
    }
}

/// Strips XDG exec field codes (%f, %F, %u, %U, %d, %D, %n, %N, %i, %c, %k, %v, %m).
pub fn clean_exec(raw_exec: &str) -> String {
    let mut parts = Vec::new();
    for token in raw_exec.split_whitespace() {
        if token.starts_with('%') && token.len() == 2 {
            // Strip single field codes
            continue;
        }
        // Also strip quoted codes like "%u" or "%U"
        if (token.starts_with("\"%") || token.starts_with("'%")) && token.len() == 4 {
            continue;
        }
        parts.push(token);
    }
    parts.join(" ")
}

/// Checks whether an executable in TryExec exists or is in PATH.
fn is_executable_available(exec_name: &str) -> bool {
    let path = Path::new(exec_name);
    if path.is_absolute() || exec_name.contains('/') {
        return path.is_file();
    }

    if let Ok(paths) = env::var("PATH") {
        for dir in env::split_paths(&paths) {
            let candidate = dir.join(exec_name);
            if candidate.is_file() {
                return true;
            }
        }
    }
    // Fallback if PATH is unset
    false
}

/// Checks if the entry is compatible with the current desktop environment.
fn is_compatible_desktop(entry: &FdDesktopEntry) -> bool {
    let current_desktop = match env::var("XDG_CURRENT_DESKTOP") {
        Ok(val) => val,
        Err(_) => return true, // If not set, fail open to all desktops
    };

    let desktops: Vec<&str> = current_desktop.split(':').map(|s| s.trim()).collect();

    // Check OnlyShowIn: if present, current desktop MUST match at least one
    if let Some(only_show_in) = entry.only_show_in() {
        let matches = only_show_in.iter().any(|target| {
            desktops
                .iter()
                .any(|d| d.eq_ignore_ascii_case(target.as_ref()))
        });
        if !matches {
            return false;
        }
    }

    // Check NotShowIn: if present, current desktop MUST NOT match any
    if let Some(not_show_in) = entry.not_show_in() {
        let matches = not_show_in.iter().any(|target| {
            desktops
                .iter()
                .any(|d| d.eq_ignore_ascii_case(target.as_ref()))
        });
        if matches {
            return false;
        }
    }

    true
}

/// Returns standard XDG applications directories in precedence order (highest precedence first).
pub fn get_application_search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut seen = HashSet::new();

    // 1. $XDG_DATA_HOME/applications (default ~/.local/share/applications)
    let home = env::var_os("HOME").map(PathBuf::from);
    let data_home = match env::var_os("XDG_DATA_HOME").map(PathBuf::from) {
        Some(path) => path,
        None => home
            .as_ref()
            .map(|h| h.join(".local/share"))
            .unwrap_or_default(),
    };
    if !data_home.as_os_str().is_empty() {
        let app_dir = data_home.join("applications");
        if seen.insert(app_dir.clone()) {
            dirs.push(app_dir);
        }
    }

    // Flatpak user exports: ~/.local/share/flatpak/exports/share/applications
    if let Some(ref h) = home {
        let flatpak_user = h.join(".local/share/flatpak/exports/share/applications");
        if seen.insert(flatpak_user.clone()) {
            dirs.push(flatpak_user);
        }
    }

    // 2. $XDG_DATA_DIRS/applications (default /usr/local/share and /usr/share)
    let data_dirs = match env::var("XDG_DATA_DIRS") {
        Ok(val) if !val.is_empty() => env::split_paths(&val).collect(),
        _ => vec![
            PathBuf::from("/usr/local/share"),
            PathBuf::from("/usr/share"),
        ],
    };
    for data_dir in data_dirs {
        let app_dir = data_dir.join("applications");
        if seen.insert(app_dir.clone()) {
            dirs.push(app_dir);
        }
    }

    // Flatpak system exports: /var/lib/flatpak/exports/share/applications
    let flatpak_system = PathBuf::from("/var/lib/flatpak/exports/share/applications");
    if seen.insert(flatpak_system.clone()) {
        dirs.push(flatpak_system);
    }

    // Snap exports: /var/lib/snapd/desktop/applications
    let snap_dir = PathBuf::from("/var/lib/snapd/desktop/applications");
    if seen.insert(snap_dir.clone()) {
        dirs.push(snap_dir);
    }

    dirs
}

/// Scans all standard application directories and returns deduplicated `DesktopEntry` items.
pub fn scan_desktop_entries() -> Vec<DesktopEntry> {
    scan_desktop_entries_in_dirs(&get_application_search_dirs())
}

/// Scans specified directories in order of precedence.
/// Higher precedence directories override entries with the same desktop file ID.
pub fn scan_desktop_entries_in_dirs(dirs: &[PathBuf]) -> Vec<DesktopEntry> {
    let mut entries_by_id: HashMap<String, DesktopEntry> = HashMap::new();

    for dir in dirs {
        if !dir.is_dir() {
            continue;
        }

        for entry in WalkDir::new(dir)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("desktop") {
                continue;
            }

            // Desktop entry ID is relative to `dir` with `/` replaced by `-`
            let id = match path.strip_prefix(dir) {
                Ok(rel_path) => rel_path
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("-"),
                Err(_) => match path.file_name() {
                    Some(name) => name.to_string_lossy().to_string(),
                    None => continue,
                },
            };

            // If an entry with this ID was already added by a higher-priority directory, skip it
            if entries_by_id.contains_key(&id) {
                continue;
            }

            if let Some(parsed) = DesktopEntry::from_file(path, id.clone()) {
                entries_by_id.insert(id, parsed);
            }
        }
    }

    entries_by_id.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_clean_exec() {
        assert_eq!(clean_exec("firefox %u"), "firefox");
        assert_eq!(clean_exec("gimp-2.10 %F --verbose"), "gimp-2.10 --verbose");
        assert_eq!(clean_exec("code --unity-launch %F"), "code --unity-launch");
        assert_eq!(clean_exec("vlc \"%U\""), "vlc");
    }

    #[test]
    fn test_desktop_entry_parsing() {
        let sample = r#"
[Desktop Entry]
Version=1.0
Type=Application
Name=Firefox Web Browser
GenericName=Web Browser
Comment=Browse the World Wide Web
Exec=firefox %u
Icon=firefox
Terminal=false
Categories=Network;WebBrowser;
Keywords=internet;surf;browser;
"#;
        let entry = DesktopEntry::from_str(
            sample,
            PathBuf::from("/usr/share/applications/firefox.desktop"),
            "firefox.desktop".into(),
        )
        .expect("Failed to parse valid desktop entry");

        assert_eq!(entry.id, "firefox.desktop");
        assert_eq!(entry.name, "Firefox Web Browser");
        assert_eq!(entry.exec, "firefox");
        assert_eq!(entry.icon, Some("firefox".into()));
        assert_eq!(entry.generic_name, Some("Web Browser".into()));
        assert!(!entry.terminal);
        assert_eq!(entry.categories, vec!["Network", "WebBrowser"]);
        assert_eq!(entry.keywords, vec!["internet", "surf", "browser"]);
        assert!(entry.search_haystack.contains("Firefox"));
        assert!(entry.search_haystack.contains("Web Browser"));
    }

    #[test]
    fn test_filter_no_display_and_hidden() {
        let hidden_sample = r#"
[Desktop Entry]
Type=Application
Name=Hidden App
Exec=hidden
Hidden=true
"#;
        assert!(
            DesktopEntry::from_str(
                hidden_sample,
                PathBuf::from("/test.desktop"),
                "test.desktop".into()
            )
            .is_none()
        );

        let nodisplay_sample = r#"
[Desktop Entry]
Type=Application
Name=No Display App
Exec=nodisplay
NoDisplay=true
"#;
        assert!(
            DesktopEntry::from_str(
                nodisplay_sample,
                PathBuf::from("/test.desktop"),
                "test.desktop".into()
            )
            .is_none()
        );
    }

    #[test]
    fn test_filter_non_application() {
        let link_sample = r#"
[Desktop Entry]
Type=Link
Name=My Link
URL=https://example.com
"#;
        assert!(
            DesktopEntry::from_str(
                link_sample,
                PathBuf::from("/link.desktop"),
                "link.desktop".into()
            )
            .is_none()
        );
    }

    #[test]
    fn test_scanner_precedence_and_deduplication() {
        let temp = tempdir().unwrap();
        let user_apps = temp.path().join("user/applications");
        let sys_apps = temp.path().join("sys/applications");
        fs::create_dir_all(&user_apps).unwrap();
        fs::create_dir_all(&sys_apps).unwrap();

        // System has "editor.desktop" (Sys Editor)
        fs::write(
            sys_apps.join("editor.desktop"),
            r#"
[Desktop Entry]
Type=Application
Name=System Editor
Exec=sys-editor
"#,
        )
        .unwrap();

        // User overrides "editor.desktop" (User Editor)
        fs::write(
            user_apps.join("editor.desktop"),
            r#"
[Desktop Entry]
Type=Application
Name=User Custom Editor
Exec=user-editor
"#,
        )
        .unwrap();

        // System has "calculator.desktop"
        fs::write(
            sys_apps.join("calc.desktop"),
            r#"
[Desktop Entry]
Type=Application
Name=Calculator
Exec=calc
"#,
        )
        .unwrap();

        let dirs = vec![user_apps, sys_apps];
        let entries = scan_desktop_entries_in_dirs(&dirs);

        assert_eq!(entries.len(), 2);
        let editor = entries.iter().find(|e| e.id == "editor.desktop").unwrap();
        assert_eq!(
            editor.name, "User Custom Editor",
            "User level entry must take precedence"
        );
    }
}
