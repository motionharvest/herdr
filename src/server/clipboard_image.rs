use rand::Rng;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::time::Duration;

const STAGED_CLIPBOARD_IMAGE_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

pub(crate) struct StagedClipboardImage {
    pub(crate) path: PathBuf,
    pub(crate) paste_text: String,
}

pub(crate) fn stage(
    _client_id: u64,
    extension: &str,
    data: &[u8],
) -> io::Result<StagedClipboardImage> {
    use std::os::unix::fs::OpenOptionsExt;

    let extension = sanitize_extension(extension);
    let dir = ensure_staging_dir()?;
    cleanup_stale(&dir);

    let id = random_id(5);

    for attempt in 0..100 {
        let file_name = if attempt == 0 {
            format!("{id}.{extension}")
        } else {
            format!("{id}-{attempt}.{extension}")
        };
        let path = dir.join(file_name);
        let mut file = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        };
        file.write_all(data)?;
        return Ok(StagedClipboardImage {
            paste_text: path.to_string_lossy().into_owned(),
            path,
        });
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "failed to allocate unique clipboard image staging path",
    ))
}

pub(crate) fn remove_files(paths: Vec<PathBuf>) {
    for path in paths {
        let _ = fs::remove_file(path);
    }
}

fn random_id(length: usize) -> String {
    let alphabet = b"abcdefghijklmnopqrstuvwxyz123456789";
    let mut id = String::with_capacity(length);
    let mut rng = rand::thread_rng();

    for _ in 0..length {
        let random_index = rng.gen_range(0..alphabet.len());
        id.push(alphabet[random_index] as char);
    }

    id
}

fn sanitize_extension(extension: &str) -> &'static str {
    if extension.eq_ignore_ascii_case("png") {
        "png"
    } else if extension.eq_ignore_ascii_case("jpg") || extension.eq_ignore_ascii_case("jpeg") {
        "jpg"
    } else if extension.eq_ignore_ascii_case("gif") {
        "gif"
    } else if extension.eq_ignore_ascii_case("webp") {
        "webp"
    } else if extension.eq_ignore_ascii_case("bmp") {
        "bmp"
    } else {
        "png"
    }
}

fn staging_dir() -> PathBuf {
    std::env::temp_dir().join("herdr-paste")
}

fn ensure_staging_dir() -> io::Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;

    let dir = staging_dir();
    fs::create_dir_all(&dir)?;
    let metadata = fs::metadata(&dir)?;
    if !metadata.is_dir() {
        return Err(io::Error::other(format!(
            "clipboard image staging path is not a directory: {}",
            dir.display()
        )));
    }
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    Ok(dir)
}

fn cleanup_stale(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        if modified.elapsed().unwrap_or_default() > STAGED_CLIPBOARD_IMAGE_MAX_AGE {
            let _ = fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_extension_accepts_known_image_extensions() {
        assert_eq!(sanitize_extension("PNG"), "png");
        assert_eq!(sanitize_extension("jpeg"), "jpg");
        assert_eq!(sanitize_extension("webp"), "webp");
        assert_eq!(sanitize_extension("sh"), "png");
    }

    #[test]
    fn staged_file_names_are_short() {
        let staged = stage(8, "bmp", b"img").expect("stage image");
        let file_name = staged
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("utf8 file name");

        assert!(file_name.ends_with(".bmp"));
        assert!(!file_name.contains("clipboard"));
        assert!(!file_name.contains("client"));

        let stem = file_name
            .strip_suffix(".bmp")
            .expect("bmp extension removed");
        assert_eq!(stem.len(), 5);
        assert!(stem
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || matches!(ch, '1'..='9')));

        let parent = staged
            .path
            .parent()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            .expect("parent dir");
        assert_eq!(parent, "herdr-paste");

        remove_files(vec![staged.path]);
    }

    #[test]
    fn random_id_uses_expected_charset_and_length() {
        let id = random_id(5);
        assert_eq!(id.len(), 5);
        assert!(id
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || matches!(ch, '1'..='9')));
    }
}
