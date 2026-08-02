//! WSL interop support.
//!
//! Windows console programs launched from WSL (`powershell.exe`, `pwsh.exe`,
//! `cmd.exe`) run as real Windows processes behind a `/init` relay stub. The
//! stub's `/proc/<pid>/cwd` is frozen at the directory the program was launched
//! from, so herdr cannot learn the program's actual location from `/proc` — a
//! `Set-Location` inside PowerShell is invisible to Linux.
//!
//! The escape hatch is OSC 7: the shell reports its own location, and herdr
//! translates the Windows path it reports back into a Linux path. This module
//! owns that translation plus the interop-shell recognition used when a split
//! should relaunch the same Windows shell.

use std::path::PathBuf;
use std::sync::OnceLock;

/// Shells reachable through WSL interop that a split should relaunch instead of
/// falling back to the configured Linux shell.
const INTEROP_SHELLS: &[&str] = &["powershell.exe", "pwsh.exe", "cmd.exe"];

const DEFAULT_AUTOMOUNT_ROOT: &str = "/mnt";

pub fn is_wsl() -> bool {
    static IS_WSL: OnceLock<bool> = OnceLock::new();
    *IS_WSL.get_or_init(detect_wsl)
}

fn detect_wsl() -> bool {
    if !cfg!(target_os = "linux") {
        return false;
    }
    if std::env::var_os("WSL_DISTRO_NAME").is_some() || std::env::var_os("WSL_INTEROP").is_some() {
        return true;
    }
    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|release| {
            let release = release.to_ascii_lowercase();
            release.contains("microsoft") || release.contains("wsl")
        })
        .unwrap_or(false)
}

fn distro_name() -> Option<String> {
    std::env::var("WSL_DISTRO_NAME")
        .ok()
        .filter(|name| !name.is_empty())
}

/// The `[automount] root` from `/etc/wsl.conf`, which decides where Windows
/// drives appear. Defaults to `/mnt`.
fn automount_root() -> &'static str {
    static ROOT: OnceLock<String> = OnceLock::new();
    ROOT.get_or_init(|| {
        std::fs::read_to_string("/etc/wsl.conf")
            .ok()
            .and_then(|contents| parse_automount_root(&contents))
            .unwrap_or_else(|| DEFAULT_AUTOMOUNT_ROOT.to_string())
    })
}

fn parse_automount_root(contents: &str) -> Option<String> {
    let mut in_automount = false;
    for line in contents.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_automount = line[1..line.len() - 1]
                .trim()
                .eq_ignore_ascii_case("automount");
            continue;
        }
        if !in_automount {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if !key.trim().eq_ignore_ascii_case("root") {
            continue;
        }
        let value = value.trim().trim_matches('"');
        if value.is_empty() {
            continue;
        }
        return Some(value.trim_end_matches('/').to_string());
    }
    None
}

/// Translate a location reported by a Windows program into a Linux path.
///
/// Handles the three forms a Windows shell reports from inside WSL:
/// `\\wsl.localhost\<distro>\path` and `\\wsl$\<distro>\path` (this distro's
/// own filesystem), and drive paths like `C:\Users\me` (mounted under the
/// automount root). Paths that are already POSIX pass through unchanged.
/// Anything else — another distro, a real network share, a non-filesystem
/// PowerShell provider such as `HKLM:\` — has no Linux equivalent and yields
/// `None`.
pub fn translate_reported_path(raw: &str) -> Option<PathBuf> {
    translate_reported_path_with(raw, distro_name().as_deref(), automount_root())
}

fn translate_reported_path_with(
    raw: &str,
    distro: Option<&str>,
    automount_root: &str,
) -> Option<PathBuf> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }

    let normalized = raw.replace('\\', "/");

    if let Some(unc) = normalized.strip_prefix("//") {
        return translate_unc_path(unc, distro);
    }

    if let Some(translated) = translate_drive_path(&normalized, automount_root) {
        return Some(translated);
    }

    normalized
        .starts_with('/')
        .then(|| PathBuf::from(normalized))
}

fn translate_unc_path(unc: &str, distro: Option<&str>) -> Option<PathBuf> {
    let mut parts = unc.splitn(3, '/');
    let host = parts.next()?;
    if !(host.eq_ignore_ascii_case("wsl.localhost") || host.eq_ignore_ascii_case("wsl$")) {
        // A real network share has no local path.
        return None;
    }

    let share = parts.next()?;
    if share.is_empty() {
        return None;
    }
    // Another distro's filesystem is not reachable at the same path here.
    if distro.is_some_and(|distro| !share.eq_ignore_ascii_case(distro)) {
        return None;
    }

    let rest = parts.next().unwrap_or("");
    Some(PathBuf::from(format!("/{}", rest.trim_start_matches('/'))))
}

fn translate_drive_path(normalized: &str, automount_root: &str) -> Option<PathBuf> {
    let mut chars = normalized.chars();
    let drive = chars.next()?;
    if !drive.is_ascii_alphabetic() || chars.next() != Some(':') {
        return None;
    }

    let rest = &normalized[2..];
    if !(rest.is_empty() || rest.starts_with('/')) {
        // `C:relative` is a Windows per-drive relative path; there is nothing
        // to anchor it to from here.
        return None;
    }

    let root = automount_root.trim_end_matches('/');
    Some(PathBuf::from(format!(
        "{root}/{}{rest}",
        drive.to_ascii_lowercase()
    )))
}

/// The program a split should launch to stay in the same shell, when the pane's
/// foreground process is a Windows shell running through interop.
///
/// `argv` for an interop process is `["/init", "<linux path to the exe>",
/// "<argv0 as typed>", ...]`, so the resolved path in `argv[1]` is preferred
/// over relying on `PATH` lookup.
pub fn interop_shell_program(name: &str, argv: Option<&[String]>) -> Option<String> {
    let lowered = name.to_ascii_lowercase();
    if !INTEROP_SHELLS.contains(&lowered.as_str()) {
        return None;
    }

    let resolved = argv.and_then(|argv| argv.get(1)).filter(|candidate| {
        candidate.starts_with('/') && candidate.to_ascii_lowercase().ends_with(&lowered)
    });

    Some(resolved.cloned().unwrap_or(lowered))
}

/// Append `name` to a `WSLENV` value so the variable crosses the interop
/// boundary into Windows processes. Returns `None` when it is already listed.
pub fn wslenv_with(existing: Option<&str>, name: &str) -> Option<String> {
    // A trailing separator would otherwise produce an empty entry.
    let existing = existing.unwrap_or("").trim_end_matches(':');
    let already_listed = existing
        .split(':')
        .filter(|entry| !entry.is_empty())
        .any(|entry| entry.split('/').next().unwrap_or(entry) == name);
    if already_listed {
        return None;
    }
    if existing.is_empty() {
        Some(name.to_string())
    } else {
        Some(format!("{existing}:{name}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn translate(raw: &str) -> Option<PathBuf> {
        translate_reported_path_with(raw, Some("Ubuntu"), "/mnt")
    }

    #[test]
    fn translates_wsl_unc_paths_for_this_distro() {
        assert_eq!(
            translate(r"\\wsl.localhost\Ubuntu\home\aaron\lab"),
            Some(PathBuf::from("/home/aaron/lab"))
        );
        assert_eq!(
            translate(r"\\wsl$\Ubuntu\home\aaron\lab"),
            Some(PathBuf::from("/home/aaron/lab"))
        );
        assert_eq!(
            translate(r"\\WSL.LOCALHOST\ubuntu\home\aaron"),
            Some(PathBuf::from("/home/aaron"))
        );
        assert_eq!(
            translate(r"\\wsl.localhost\Ubuntu"),
            Some(PathBuf::from("/"))
        );
    }

    /// The exact strings `parse_osc7_report` hands over for the payloads
    /// Windows PowerShell 5.1 emits.
    #[test]
    fn translates_paths_reported_by_real_powershell() {
        assert_eq!(
            translate("//wsl.localhost/Ubuntu/home/aaron/lab/herdr"),
            Some(PathBuf::from("/home/aaron/lab/herdr"))
        );
        assert_eq!(
            translate("C:/Program Files"),
            Some(PathBuf::from("/mnt/c/Program Files"))
        );
    }

    #[test]
    fn rejects_unc_paths_without_a_local_equivalent() {
        assert_eq!(translate(r"\\wsl.localhost\Debian\home\aaron"), None);
        assert_eq!(translate(r"\\fileserver\share\docs"), None);
        assert_eq!(translate(r"\\wsl.localhost"), None);
    }

    #[test]
    fn translates_drive_paths_under_the_automount_root() {
        assert_eq!(
            translate(r"C:\Users\aaron"),
            Some(PathBuf::from("/mnt/c/Users/aaron"))
        );
        assert_eq!(translate(r"D:\"), Some(PathBuf::from("/mnt/d/")));
        assert_eq!(translate("C:"), Some(PathBuf::from("/mnt/c")));
        assert_eq!(
            translate_reported_path_with(r"C:\Windows", Some("Ubuntu"), "/windows/"),
            Some(PathBuf::from("/windows/c/Windows"))
        );
    }

    #[test]
    fn passes_through_posix_paths_and_rejects_the_rest() {
        assert_eq!(
            translate("/home/aaron/lab"),
            Some(PathBuf::from("/home/aaron/lab"))
        );
        // PowerShell providers that are not the filesystem.
        assert_eq!(translate(r"HKLM:\SOFTWARE"), None);
        assert_eq!(translate(r"Cert:\CurrentUser"), None);
        assert_eq!(translate(r"C:relative\path"), None);
        assert_eq!(translate("relative/path"), None);
        assert_eq!(translate("   "), None);
    }

    #[test]
    fn recognizes_interop_shells_and_prefers_the_resolved_exe() {
        let argv = vec![
            "/init".to_string(),
            "/mnt/c/Windows/System32/WindowsPowerShell/v1.0//powershell.exe".to_string(),
            "powershell.exe".to_string(),
        ];
        assert_eq!(
            interop_shell_program("powershell.exe", Some(&argv)),
            Some("/mnt/c/Windows/System32/WindowsPowerShell/v1.0//powershell.exe".to_string())
        );
        assert_eq!(
            interop_shell_program("pwsh.exe", None),
            Some("pwsh.exe".to_string())
        );
        assert_eq!(interop_shell_program("zsh", None), None);
        assert_eq!(interop_shell_program("node", Some(&argv)), None);
    }

    #[test]
    fn falls_back_to_the_bare_name_when_argv_is_unhelpful() {
        let argv = vec!["/init".to_string(), "not-an-absolute-path".to_string()];
        assert_eq!(
            interop_shell_program("cmd.exe", Some(&argv)),
            Some("cmd.exe".to_string())
        );
    }

    #[test]
    fn parses_the_automount_root_from_wsl_conf() {
        let conf = "[boot]\nsystemd=true\n\n[automount]\n# comment\nroot = /windows/\noptions=\"metadata\"\n";
        assert_eq!(parse_automount_root(conf), Some("/windows".to_string()));
        assert_eq!(parse_automount_root("[network]\nroot=/nope\n"), None);
        assert_eq!(parse_automount_root(""), None);
    }

    #[test]
    fn appends_to_wslenv_without_duplicating() {
        assert_eq!(
            wslenv_with(None, "HERDR_ENV"),
            Some("HERDR_ENV".to_string())
        );
        assert_eq!(
            wslenv_with(Some("FOO/p"), "HERDR_ENV"),
            Some("FOO/p:HERDR_ENV".to_string())
        );
        assert_eq!(wslenv_with(Some("HERDR_ENV"), "HERDR_ENV"), None);
        assert_eq!(wslenv_with(Some("FOO:HERDR_ENV/u"), "HERDR_ENV"), None);
        // Windows Terminal leaves a trailing separator; do not emit `A::B`.
        assert_eq!(
            wslenv_with(Some("WT_SESSION:WT_PROFILE_ID:"), "HERDR_ENV"),
            Some("WT_SESSION:WT_PROFILE_ID:HERDR_ENV".to_string())
        );
        assert_eq!(
            wslenv_with(Some(":"), "HERDR_ENV"),
            Some("HERDR_ENV".to_string())
        );
    }
}
