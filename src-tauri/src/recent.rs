//! Recent-workspaces registry: the home screen's workspace list.
//!
//! The registry is a small JSON file in the application data directory. It is
//! advisory only: a missing, unreadable, or corrupt file reads as an empty
//! list, and a failed write never blocks opening or creating a workspace.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const MAX_ENTRIES: usize = 10;
const FILE_NAME: &str = "recent-workspaces.json";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentWorkspace {
    pub path: String,
    pub name: String,
    pub last_opened_at: String,
}

impl RecentWorkspace {
    pub fn new(workspace_path: &Path, opened_at: String) -> Self {
        let path = workspace_path.to_string_lossy().into_owned();
        let name = workspace_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());
        Self {
            path,
            name,
            last_opened_at: opened_at,
        }
    }
}

pub fn file_path(data_directory: &Path) -> PathBuf {
    data_directory.join(FILE_NAME)
}

pub fn load(file: &Path) -> Vec<RecentWorkspace> {
    let Ok(text) = std::fs::read_to_string(file) else {
        return Vec::new();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

pub fn save(file: &Path, entries: &[RecentWorkspace]) -> std::io::Result<()> {
    let text = serde_json::to_string_pretty(entries).map_err(std::io::Error::other)?;
    let temporary = file.with_extension("json.tmp");
    std::fs::write(&temporary, text)?;
    std::fs::rename(&temporary, file)
}

/// Move `workspace_path` to the front of the list, deduplicated by path and
/// capped at the newest `MAX_ENTRIES` entries.
pub fn record(entries: &mut Vec<RecentWorkspace>, workspace_path: &Path, opened_at: String) {
    let entry = RecentWorkspace::new(workspace_path, opened_at);
    entries.retain(|existing| existing.path != entry.path);
    entries.insert(0, entry);
    entries.truncate(MAX_ENTRIES);
}

pub fn forget(entries: &mut Vec<RecentWorkspace>, path: &str) -> bool {
    let before = entries.len();
    entries.retain(|entry| entry.path != path);
    entries.len() != before
}

/// The default workspace lives in the application data directory. Keep it in
/// the list even before its first recorded open, so existing installs always
/// see the path to their data. It has no open timestamp until opened once.
pub fn seed_default(entries: &mut Vec<RecentWorkspace>, default_path: &Path) {
    if !default_path.exists() {
        return;
    }
    let path = default_path.to_string_lossy();
    if entries.iter().any(|entry| entry.path == path) {
        return;
    }
    entries.push(RecentWorkspace::new(default_path, String::new()));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str) -> RecentWorkspace {
        RecentWorkspace {
            path: format!("/data/{name}"),
            name: name.to_string(),
            last_opened_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn missing_or_corrupt_files_read_as_an_empty_list() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("recent-workspaces.json");
        assert!(load(&file).is_empty());

        std::fs::write(&file, "not json").unwrap();
        assert!(load(&file).is_empty());

        std::fs::write(&file, r#"{"not":"a list"}"#).unwrap();
        assert!(load(&file).is_empty());
    }

    #[test]
    fn save_and_load_round_trip() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("recent-workspaces.json");
        let entries = vec![entry("alpha"), entry("beta")];
        save(&file, &entries).unwrap();
        assert_eq!(load(&file), entries);
    }

    #[test]
    fn record_dedupes_moves_to_front_and_caps_the_list() {
        let mut entries = vec![entry("alpha"), entry("beta")];
        record(&mut entries, Path::new("/data/beta"), "now".to_string());
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].path, "/data/beta");
        assert_eq!(entries[0].last_opened_at, "now");

        for index in 0..15 {
            record(
                &mut entries,
                Path::new(&format!("/data/w{index}")),
                "now".to_string(),
            );
        }
        assert_eq!(entries.len(), MAX_ENTRIES);
        assert_eq!(entries[0].path, "/data/w14");
    }

    #[test]
    fn forget_removes_exactly_one_path() {
        let mut entries = vec![entry("alpha"), entry("beta")];
        assert!(forget(&mut entries, "/data/alpha"));
        assert_eq!(entries, vec![entry("beta")]);
        assert!(!forget(&mut entries, "/data/alpha"));
    }

    #[test]
    fn seed_default_adds_only_an_existing_untracked_default() {
        let directory = tempfile::tempdir().unwrap();
        let default = directory.path().join("workspace");

        let mut entries = Vec::new();
        seed_default(&mut entries, &default);
        assert!(entries.is_empty());

        std::fs::create_dir_all(&default).unwrap();
        seed_default(&mut entries, &default);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "workspace");
        assert_eq!(entries[0].last_opened_at, "");

        seed_default(&mut entries, &default);
        assert_eq!(entries.len(), 1);

        let mut recorded = Vec::new();
        record(&mut recorded, &default, "now".to_string());
        seed_default(&mut recorded, &default);
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].last_opened_at, "now");
    }
}
