//! The folders the composer has started agents in.
//!
//! They are kept in their own file beside the config rather than in the
//! session snapshot, because they describe where work on this machine goes,
//! not what one session had open: a named session, a `--no-session` run, and
//! the next session after a restart all offer the same starred folders.
//!
//! Stored at `~/.config/herdr/composer-folders.json`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
struct UsedFolders {
    #[serde(default)]
    folders: Vec<PathBuf>,
}

/// Tests start agents through the real composer path, and a test run must not
/// rewrite the list of the person running it, so tests have no file.
fn used_folders_path() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    Some(crate::config::config_dir().join("composer-folders.json"))
}

/// The saved folders, most recent first, or none when there is no file or it
/// cannot be read. A list that fails to load is an empty section, never an
/// error that stops herdr starting.
pub fn load_used_folders() -> Vec<PathBuf> {
    used_folders_path()
        .map(|path| load_from_path(&path))
        .unwrap_or_default()
}

pub fn save_used_folders(folders: &[PathBuf]) {
    let Some(path) = used_folders_path() else {
        return;
    };
    if let Err(err) = save_to_path(&path, folders) {
        tracing::warn!(path = %path.display(), %err, "could not save composer folders");
    }
}

fn load_from_path(path: &Path) -> Vec<PathBuf> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    match serde_json::from_str::<UsedFolders>(&content) {
        Ok(saved) => saved.folders,
        Err(err) => {
            tracing::warn!(path = %path.display(), %err, "could not read composer folders");
            Vec::new()
        }
    }
}

fn save_to_path(path: &Path, folders: &[PathBuf]) -> std::io::Result<()> {
    super::io::save_json_to_path(
        path,
        &UsedFolders {
            folders: folders.to_vec(),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!(
                "herdr-used-folders-{name}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ))
            .join("composer-folders.json")
    }

    #[test]
    fn saved_folders_come_back_in_the_same_order() {
        let path = temp_path("round-trip");
        let folders = vec![PathBuf::from("/b"), PathBuf::from("/a")];
        save_to_path(&path, &folders).unwrap();
        assert_eq!(load_from_path(&path), folders);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_missing_or_broken_file_is_an_empty_list() {
        let path = temp_path("broken");
        assert!(load_from_path(&path).is_empty());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "not json").unwrap();
        assert!(load_from_path(&path).is_empty());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
