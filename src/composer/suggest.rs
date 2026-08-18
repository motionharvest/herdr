//! What the directory control offers while a path is being typed.
//!
//! Two sources, and what has been typed decides which one leads. Anything
//! holding a `/`, or standing for a home directory, names a place to look in,
//! so the folders on disk under that place lead. A bare word names nothing on
//! disk yet, so it is matched against the directories herdr already knows.
//!
//! The list is short on purpose. It is read while typing, out of the corner of
//! an eye, and a list long enough to need reading is a list that has to be
//! looked away at.

use std::path::{Path, PathBuf};

use super::{expand_home, Folder};

/// How many folders are offered at once.
pub const MOST: usize = 5;

/// The folders worth offering for what has been typed, best first.
pub fn matching(typed: &str, recent: &[Folder]) -> Vec<Folder> {
    let typed = typed.trim();
    if typed.is_empty() {
        return Vec::new();
    }

    let mut found = match looked_in(typed) {
        Some((dir, fragment)) => children(&dir, &fragment),
        None => Vec::new(),
    };
    found.extend(remembered(typed, recent));

    let mut folders: Vec<Folder> = Vec::new();
    for path in found {
        if folders.iter().any(|folder| folder.path == path) {
            continue;
        }
        folders.push(Folder::new(path));
        if folders.len() == MOST {
            break;
        }
    }
    folders
}

/// The directory a typed path is asking to look in, and the start of the name
/// it is asking for. `None` when what was typed names no directory to look in,
/// which is every word without a separator in it.
fn looked_in(typed: &str) -> Option<(PathBuf, String)> {
    if typed == "~" {
        return Some((expand_home("~"), String::new()));
    }
    let after = typed.rfind('/')? + 1;
    let (parent, fragment) = typed.split_at(after);
    let parent = match parent.trim_end_matches('/') {
        "" => PathBuf::from("/"),
        trimmed => expand_home(trimmed),
    };
    Some((parent, fragment.to_string()))
}

/// The directories inside `dir` whose names begin with `fragment`, in the order
/// they would be read in. A hidden folder is offered only once the dot that
/// hides it has been typed, because a list led by `.cache` and `.git` is a list
/// with the answer pushed off the end of it.
fn children(dir: &Path, fragment: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let wanted = fragment.to_lowercase();
    let mut found: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let path = entry.path();
            let hidden = name.starts_with('.') && !fragment.starts_with('.');
            (!hidden && name.to_lowercase().starts_with(&wanted) && path.is_dir())
                .then_some((name.to_lowercase(), path))
        })
        .collect();
    found.sort_by(|(left, _), (right, _)| left.cmp(right));
    found.into_iter().map(|(_, path)| path).collect()
}

/// The directories already listed that hold what was typed anywhere in them.
/// This is what makes a bare word worth typing: `herdr` finds the folder
/// without any of the path in front of it.
fn remembered(typed: &str, recent: &[Folder]) -> Vec<PathBuf> {
    let wanted = typed.to_lowercase();
    recent
        .iter()
        .filter(|folder| {
            folder.label.to_lowercase().contains(&wanted)
                || folder
                    .path
                    .to_string_lossy()
                    .to_lowercase()
                    .contains(&wanted)
        })
        .map(|folder| folder.path.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let unique = format!(
            "herdr-suggest-tests-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let path = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn tree(name: &str) -> PathBuf {
        let root = temp_dir(name);
        for child in ["herdr", "herdr-old", "hooks", "notes", ".cache"] {
            std::fs::create_dir(root.join(child)).unwrap();
        }
        std::fs::write(root.join("herdr.txt"), "").unwrap();
        root
    }

    fn names(folders: &[Folder]) -> Vec<String> {
        folders
            .iter()
            .map(|folder| {
                folder
                    .path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn typing_a_path_offers_the_folders_under_it() {
        let root = tree("under");
        let typed = format!("{}/", root.display());
        assert_eq!(
            names(&matching(&typed, &[])),
            ["herdr", "herdr-old", "hooks", "notes"]
        );
    }

    #[test]
    fn a_part_of_a_name_narrows_the_offer() {
        let root = tree("narrow");
        let typed = format!("{}/her", root.display());
        assert_eq!(names(&matching(&typed, &[])), ["herdr", "herdr-old"]);
    }

    #[test]
    fn only_folders_are_offered() {
        let root = tree("folders");
        let typed = format!("{}/herdr.", root.display());
        assert!(matching(&typed, &[]).is_empty(), "the file is not a folder");
    }

    #[test]
    fn a_hidden_folder_waits_for_the_dot_that_hides_it() {
        let root = tree("hidden");
        let bare = format!("{}/", root.display());
        assert!(!names(&matching(&bare, &[])).contains(&".cache".to_string()));
        let dotted = format!("{}/.", root.display());
        assert_eq!(names(&matching(&dotted, &[])), [".cache"]);
    }

    #[test]
    fn a_bare_word_searches_the_folders_already_listed() {
        let recent = [
            Folder::new(PathBuf::from("/home/nobody/lab/herdr")),
            Folder::new(PathBuf::from("/home/nobody/lab/what")),
        ];
        let found = matching("herd", &recent);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, PathBuf::from("/home/nobody/lab/herdr"));
    }

    #[test]
    fn nothing_typed_offers_nothing() {
        assert!(matching("   ", &[]).is_empty());
    }

    #[test]
    fn a_path_that_names_nowhere_offers_nothing() {
        assert!(matching("/definitely/not/here/x", &[]).is_empty());
    }

    #[test]
    fn no_more_than_five_are_offered() {
        let root = temp_dir("many");
        for index in 0..12 {
            std::fs::create_dir(root.join(format!("dir{index:02}"))).unwrap();
        }
        let typed = format!("{}/", root.display());
        assert_eq!(matching(&typed, &[]).len(), MOST);
    }

    #[test]
    fn a_folder_reached_two_ways_is_offered_once() {
        let root = tree("twice");
        let herdr = root.join("herdr");
        let recent = [Folder::new(herdr.clone())];
        let typed = format!("{}/herdr", root.display());
        let found = matching(&typed, &recent);
        assert_eq!(
            found.iter().filter(|folder| folder.path == herdr).count(),
            1
        );
    }

    #[test]
    fn a_lone_tilde_looks_in_the_home_directory() {
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
            return;
        };
        let (dir, fragment) = looked_in("~").unwrap();
        assert_eq!(dir, home);
        assert_eq!(fragment, "");
    }

    #[test]
    fn the_root_directory_is_the_place_a_leading_slash_looks_in() {
        let (dir, fragment) = looked_in("/us").unwrap();
        assert_eq!(dir, PathBuf::from("/"));
        assert_eq!(fragment, "us");
    }
}
