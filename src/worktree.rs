use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

const DEFAULT_WORKTREE_PREFIX: &str = "worktree";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorktreeCommand {
    pub program: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExistingWorktree {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub is_bare: bool,
    pub is_detached: bool,
    pub is_prunable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorktreeLandOutcome {
    pub branch: String,
    pub base_branch: String,
    pub commit: String,
    pub already_landed: bool,
    pub committed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorktreeRemovalOutcome {
    pub branch: Option<String>,
}

pub(crate) fn generated_branch_slug(seed: u64) -> String {
    let adjectives = [
        "brave", "calm", "clear", "green", "lucky", "quiet", "rapid", "silver",
    ];
    let nouns = [
        "river", "cloud", "field", "forest", "harbor", "meadow", "stone", "valley",
    ];
    let adjective = adjectives[(seed as usize) % adjectives.len()];
    let noun = nouns[((seed / adjectives.len() as u64) as usize) % nouns.len()];
    let suffix = seed & 0xffff;
    format!("{DEFAULT_WORKTREE_PREFIX}/{adjective}-{noun}-{suffix:04x}")
}

pub(crate) fn branch_to_path_slug(branch: &str) -> String {
    let mut slug = String::new();
    let mut last_was_dash = false;

    for ch in branch.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_was_dash = false;
        } else if !last_was_dash {
            slug.push('-');
            last_was_dash = true;
        }
    }

    let trimmed = slug.trim_matches('-').to_string();
    if trimmed.is_empty() {
        DEFAULT_WORKTREE_PREFIX.to_string()
    } else {
        trimmed
    }
}

pub(crate) fn expand_tilde_path(path: &str) -> PathBuf {
    if path == "~" {
        return std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(path));
    }

    if let Some(rest) = path.strip_prefix("~/") {
        return std::env::var("HOME")
            .map(|home| PathBuf::from(home).join(rest))
            .unwrap_or_else(|_| PathBuf::from(path));
    }

    PathBuf::from(path)
}

pub(crate) fn canonical_or_original(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn is_absolute_or_home_directory(directory: &str) -> bool {
    directory == "~"
        || directory.starts_with("~/")
        || Path::new(directory).is_absolute()
        // POSIX-style paths can cross the Windows boundary through WSL or a
        // shared config, even though Rust does not call them absolute there.
        || cfg!(windows) && directory.starts_with('/')
}

pub(crate) fn default_checkout_path(
    directory: &str,
    repo_root: &Path,
    repo_name: &str,
    branch: &str,
) -> PathBuf {
    let slug = branch_to_path_slug(branch);
    if is_absolute_or_home_directory(directory) {
        expand_tilde_path(directory).join(repo_name).join(slug)
    } else {
        repo_root.join(directory).join(slug)
    }
}

pub(crate) fn ensure_in_repo_worktree_ignored(
    repo_root: &Path,
    directory: &str,
) -> Result<(), String> {
    if is_absolute_or_home_directory(directory) {
        return Ok(());
    }

    let pattern = directory.trim_start_matches("./").trim_end_matches('/');
    if pattern.is_empty() {
        return Ok(());
    }

    if git_path_is_ignored(repo_root, pattern)? {
        return Ok(());
    }

    let exclude_path = git_exclude_path(repo_root)?;
    if let Some(parent) = exclude_path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }

    let mut contents = std::fs::read_to_string(&exclude_path).unwrap_or_default();
    if contents.lines().any(|line| line == pattern) {
        return Ok(());
    }
    if !contents.is_empty() && !contents.ends_with('\n') {
        contents.push('\n');
    }
    contents.push_str(pattern);
    contents.push('\n');
    std::fs::write(&exclude_path, contents).map_err(|err| err.to_string())
}

fn git_path_is_ignored(repo_root: &Path, path: &str) -> Result<bool, String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["check-ignore", "-q", path])
        .output()
        .map_err(|err| err.to_string())?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            Err(if stderr.is_empty() {
                format!("git check-ignore failed with status {}", output.status)
            } else {
                stderr
            })
        }
    }
}

fn git_exclude_path(repo_root: &Path) -> Result<PathBuf, String> {
    let relative = run_git_output(repo_root, &["rev-parse", "--git-path", "info/exclude"])?;
    let path = PathBuf::from(relative);
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(repo_root.join(path))
    }
}

pub(crate) fn build_worktree_remove_command(
    repo_root: &Path,
    path: &Path,
    force: bool,
) -> WorktreeCommand {
    let mut args = vec![
        "-C".to_string(),
        repo_root.display().to_string(),
        "worktree".to_string(),
        "remove".to_string(),
    ];
    if force {
        args.push("--force".to_string());
    }
    args.push(path.display().to_string());

    WorktreeCommand {
        program: "git".to_string(),
        args,
    }
}

pub(crate) fn is_dirty_worktree_remove_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("contains modified or untracked files")
        && lower.contains("use --force to delete it")
}

pub(crate) fn is_unsafe_worktree_remove_error(message: &str) -> bool {
    is_dirty_worktree_remove_error(message)
        || message == "worktree has uncommitted changes"
        || message.starts_with("worktree has ")
            && message.contains("commit(s) that have not landed on")
}

pub(crate) fn build_worktree_add_new_branch_command(
    repo_root: &Path,
    path: &Path,
    branch: &str,
    base: &str,
) -> WorktreeCommand {
    WorktreeCommand {
        program: "git".to_string(),
        args: vec![
            "-C".to_string(),
            repo_root.display().to_string(),
            "worktree".to_string(),
            "add".to_string(),
            "-b".to_string(),
            branch.to_string(),
            path.display().to_string(),
            base.to_string(),
        ],
    }
}

pub(crate) fn run_worktree_command(command: &WorktreeCommand) -> Result<(), String> {
    let output = std::process::Command::new(&command.program)
        .args(&command.args)
        .output()
        .map_err(|err| err.to_string())?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let message = if stderr.is_empty() { stdout } else { stderr };
    Err(if message.is_empty() {
        format!("{} failed with status {}", command.program, output.status)
    } else {
        message
    })
}

fn run_git_output(cwd: &Path, args: &[&str]) -> Result<String, String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .map_err(|err| err.to_string())?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if output.status.success() {
        return Ok(stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(if stderr.is_empty() {
        if stdout.is_empty() {
            format!(
                "git {} failed with status {}",
                args.join(" "),
                output.status
            )
        } else {
            stdout
        }
    } else {
        stderr
    })
}

fn current_branch(cwd: &Path) -> Result<String, String> {
    let branch = run_git_output(cwd, &["branch", "--show-current"])?;
    if branch.is_empty() {
        Err(format!("{} has a detached HEAD", cwd.display()))
    } else {
        Ok(branch)
    }
}

fn ensure_clean(cwd: &Path, label: &str) -> Result<(), String> {
    if run_git_output(cwd, &["status", "--porcelain"])?.is_empty() {
        Ok(())
    } else {
        Err(format!("{label} has uncommitted changes"))
    }
}

fn git_state_path_exists(cwd: &Path, name: &str) -> bool {
    let Ok(relative) = run_git_output(cwd, &["rev-parse", "--git-path", name]) else {
        return false;
    };
    let path = PathBuf::from(&relative);
    if path.is_absolute() {
        path.exists()
    } else {
        cwd.join(path).exists()
    }
}

fn commit_dirty_worktree(checkout: &Path) -> Result<bool, String> {
    if git_state_path_exists(checkout, "MERGE_HEAD")
        || git_state_path_exists(checkout, "rebase-merge")
        || git_state_path_exists(checkout, "rebase-apply")
        || git_state_path_exists(checkout, "CHERRY_PICK_HEAD")
    {
        return Err("worktree has a rebase or merge in progress".into());
    }
    if run_git_output(checkout, &["status", "--porcelain"])?.is_empty() {
        return Ok(false);
    }
    run_git_output(checkout, &["add", "--all"])?;
    if run_git_output(checkout, &["diff", "--cached", "--name-only"])?.is_empty() {
        return Ok(false);
    }
    let branch = current_branch(checkout)?;
    let message = format!("land {branch}");
    run_git_output(checkout, &["commit", "-m", &message])?;
    Ok(true)
}

fn commits_ahead(cwd: &Path, base: &str, branch: &str) -> Result<usize, String> {
    run_git_output(cwd, &["rev-list", "--count", &format!("{base}..{branch}")])?
        .parse::<usize>()
        .map_err(|err| format!("invalid git rev-list count: {err}"))
}

fn worktree_is_clean(checkout: &Path) -> bool {
    run_git_output(checkout, &["status", "--porcelain"])
        .ok()
        .is_some_and(|status| status.is_empty())
}

fn heads_match(checkout: &Path, parent: &Path) -> bool {
    match (
        run_git_output(checkout, &["rev-parse", "HEAD"]),
        run_git_output(parent, &["rev-parse", "HEAD"]),
    ) {
        (Ok(checkout_head), Ok(parent_head)) => checkout_head == parent_head,
        _ => false,
    }
}

pub(crate) fn worktree_commits_are_on_parent(checkout: &Path, parent: &Path) -> bool {
    let Ok(base_branch) = current_branch(parent) else {
        return heads_match(checkout, parent);
    };
    let Ok(branch) = current_branch(checkout) else {
        return heads_match(checkout, parent);
    };
    if branch == base_branch {
        return true;
    }
    commits_ahead(checkout, &base_branch, &branch).ok() == Some(0)
}

pub(crate) fn worktree_already_landed(checkout: &Path, parent: &Path) -> bool {
    worktree_is_clean(checkout) && worktree_commits_are_on_parent(checkout, parent)
}

fn run_verify_command(cwd: &Path, argv: &[String]) -> Result<(), String> {
    let Some(program) = argv.first() else {
        return Ok(());
    };
    let output = std::process::Command::new(program)
        .args(&argv[1..])
        .current_dir(cwd)
        .output()
        .map_err(|err| format!("verify command failed to start: {err}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let details = if stderr.is_empty() { stdout } else { stderr };
    Err(if details.is_empty() {
        format!("verify command failed with status {}", output.status)
    } else {
        format!("verify command failed: {details}")
    })
}

/// Rebase a linked worktree onto its parent checkout, verify it, and move the
/// parent branch forward without ever creating a merge commit.
pub(crate) fn land_worktree(
    repo_root: &Path,
    checkout: &Path,
    verify_argv: &[String],
) -> Result<WorktreeLandOutcome, String> {
    let repo_key = canonical_or_original(repo_root);
    let lock = {
        static REPO_LOCKS: OnceLock<Mutex<std::collections::HashMap<PathBuf, Arc<Mutex<()>>>>> =
            OnceLock::new();
        let mut locks = REPO_LOCKS
            .get_or_init(|| Mutex::new(std::collections::HashMap::new()))
            .lock()
            .map_err(|_| "worktree landing lock is poisoned".to_string())?;
        locks
            .entry(repo_key)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };
    let _guard = lock
        .lock()
        .map_err(|_| "repository landing lock is poisoned".to_string())?;

    let committed = commit_dirty_worktree(checkout)?;
    ensure_clean(checkout, "worktree")?;
    ensure_clean(repo_root, "parent checkout")?;
    let base_branch = current_branch(repo_root)?;
    let branch = current_branch(checkout)?;
    if branch == base_branch {
        return Err("worktree branch is the parent checkout branch".into());
    }

    for attempt in 0..2 {
        let ahead = commits_ahead(checkout, &base_branch, &branch)?;
        if ahead == 0 {
            let commit = run_git_output(checkout, &["rev-parse", "HEAD"])?;
            return Ok(WorktreeLandOutcome {
                branch,
                base_branch,
                commit,
                already_landed: true,
                committed,
            });
        }

        run_git_output(checkout, &["rebase", &base_branch]).map_err(|err| {
            format!(
                "rebase onto {base_branch} stopped in {}: {err}",
                checkout.display()
            )
        })?;
        run_verify_command(checkout, verify_argv)?;
        ensure_clean(repo_root, "parent checkout")?;

        match run_git_output(repo_root, &["merge", "--ff-only", &branch]) {
            Ok(_) => {
                let commit = run_git_output(repo_root, &["rev-parse", "HEAD"])?;
                return Ok(WorktreeLandOutcome {
                    branch,
                    base_branch,
                    commit,
                    already_landed: false,
                    committed,
                });
            }
            Err(err) if attempt == 0 => {
                tracing::info!(error = %err, "base moved while landing; rebasing once more");
            }
            Err(err) => return Err(format!("could not fast-forward {base_branch}: {err}")),
        }
    }
    Err("landing exhausted its retry".into())
}

/// Remove a worktree and its local branch. Without `force`, both uncommitted
/// files and commits absent from the parent branch stop deletion.
pub(crate) fn remove_worktree_and_branch(
    repo_root: &Path,
    checkout: &Path,
    force: bool,
) -> Result<WorktreeRemovalOutcome, String> {
    let branch = current_branch(checkout).ok();
    if !force {
        ensure_clean(checkout, "worktree")?;
        if let Some(branch) = branch.as_deref() {
            let base = current_branch(repo_root)?;
            let ahead = commits_ahead(repo_root, &base, branch)?;
            if ahead > 0 {
                return Err(format!(
                    "worktree has {ahead} commit(s) that have not landed on {base}; use --force to delete it"
                ));
            }
        }
    }

    run_worktree_command(&build_worktree_remove_command(repo_root, checkout, force))?;
    if let Some(branch_name) = branch.as_deref() {
        let delete_flag = if force { "-D" } else { "-d" };
        run_git_output(repo_root, &["branch", delete_flag, branch_name])?;
    }
    Ok(WorktreeRemovalOutcome { branch })
}

pub(crate) fn parse_worktree_list_porcelain(output: &str) -> Vec<ExistingWorktree> {
    let mut entries = Vec::new();
    let mut path: Option<PathBuf> = None;
    let mut branch = None;
    let mut is_bare = false;
    let mut is_detached = false;
    let mut is_prunable = false;

    let finish = |entries: &mut Vec<ExistingWorktree>,
                  path: &mut Option<PathBuf>,
                  branch: &mut Option<String>,
                  is_bare: &mut bool,
                  is_detached: &mut bool,
                  is_prunable: &mut bool| {
        if let Some(path) = path.take() {
            entries.push(ExistingWorktree {
                path,
                branch: branch.take(),
                is_bare: *is_bare,
                is_detached: *is_detached,
                is_prunable: *is_prunable,
            });
        }
        *is_bare = false;
        *is_detached = false;
        *is_prunable = false;
    };

    for line in output.lines() {
        if line.trim().is_empty() {
            finish(
                &mut entries,
                &mut path,
                &mut branch,
                &mut is_bare,
                &mut is_detached,
                &mut is_prunable,
            );
            continue;
        }
        if let Some(value) = line.strip_prefix("worktree ") {
            path = Some(PathBuf::from(value));
        } else if let Some(value) = line.strip_prefix("branch ") {
            branch = Some(
                value
                    .strip_prefix("refs/heads/")
                    .unwrap_or(value)
                    .to_string(),
            );
        } else if line == "detached" {
            is_detached = true;
        } else if line == "bare" {
            is_bare = true;
        } else if line.starts_with("prunable") {
            is_prunable = true;
        }
    }

    finish(
        &mut entries,
        &mut path,
        &mut branch,
        &mut is_bare,
        &mut is_detached,
        &mut is_prunable,
    );
    entries
}

pub(crate) fn list_existing_worktrees(repo_root: &Path) -> Result<Vec<ExistingWorktree>, String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["worktree", "list", "--porcelain"])
        .output()
        .map_err(|err| err.to_string())?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Ok(parse_worktree_list_porcelain(&stdout));
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(if stderr.is_empty() {
        format!("git worktree list failed with status {}", output.status)
    } else {
        stderr
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_temp_path(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!("herdr-{name}-{}-{nanos}", std::process::id()))
    }

    fn run_git(repo: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .status()
            .unwrap();
        assert!(
            status.success(),
            "git command failed: git -C {} {}",
            repo.display(),
            args.join(" ")
        );
    }

    fn create_committed_repo(name: &str) -> PathBuf {
        let repo = unique_temp_path(name);
        std::fs::create_dir_all(&repo).unwrap();
        run_git(&repo, &["init", "--quiet"]);
        run_git(&repo, &["config", "user.email", "herdr@example.invalid"]);
        run_git(&repo, &["config", "user.name", "Herdr Test"]);
        std::fs::write(repo.join("README.md"), "test\n").unwrap();
        run_git(&repo, &["add", "README.md"]);
        run_git(&repo, &["commit", "--quiet", "-m", "initial"]);
        repo
    }

    #[test]
    fn generated_branch_slug_is_worktree_namespaced_and_stable() {
        assert_eq!(generated_branch_slug(0), "worktree/brave-river-0000");
        assert_eq!(generated_branch_slug(9), "worktree/calm-cloud-0009");
    }

    #[test]
    fn parses_git_worktree_list_porcelain() {
        let output = "\
worktree /repo/main
HEAD abc
branch refs/heads/main

worktree /repo/issue
HEAD def
branch refs/heads/worktree/issue

worktree /repo/detached
HEAD fed
detached
prunable stale

";

        assert_eq!(
            parse_worktree_list_porcelain(output),
            vec![
                ExistingWorktree {
                    path: PathBuf::from("/repo/main"),
                    branch: Some("main".into()),
                    is_bare: false,
                    is_detached: false,
                    is_prunable: false,
                },
                ExistingWorktree {
                    path: PathBuf::from("/repo/issue"),
                    branch: Some("worktree/issue".into()),
                    is_bare: false,
                    is_detached: false,
                    is_prunable: false,
                },
                ExistingWorktree {
                    path: PathBuf::from("/repo/detached"),
                    branch: None,
                    is_bare: false,
                    is_detached: true,
                    is_prunable: true,
                },
            ]
        );
    }

    #[test]
    fn branch_to_path_slug_makes_branch_safe_folder_name() {
        assert_eq!(
            branch_to_path_slug("worktree/brave-river"),
            "worktree-brave-river"
        );
        assert_eq!(
            branch_to_path_slug("issue/137 Worktree Spaces"),
            "issue-137-worktree-spaces"
        );
        assert_eq!(branch_to_path_slug("///"), "worktree");
    }

    #[test]
    fn expand_tilde_path_uses_home_when_available() {
        let old_home = std::env::var_os("HOME");
        std::env::set_var("HOME", "/home/me");
        assert_eq!(
            expand_tilde_path("~/.herdr/worktrees"),
            PathBuf::from("/home/me/.herdr/worktrees")
        );
        assert_eq!(
            expand_tilde_path("/tmp/worktrees"),
            PathBuf::from("/tmp/worktrees")
        );
        if let Some(home) = old_home {
            std::env::set_var("HOME", home);
        } else {
            std::env::remove_var("HOME");
        }
    }

    #[test]
    fn default_checkout_path_joins_relative_directory_to_repo_root() {
        assert_eq!(
            default_checkout_path(
                ".herdr/worktrees",
                Path::new("/repo/herdr"),
                "herdr",
                "worktree/brave-river",
            ),
            PathBuf::from("/repo/herdr/.herdr/worktrees/worktree-brave-river")
        );
    }

    #[test]
    fn default_checkout_path_keeps_repo_segment_for_absolute_directory() {
        assert_eq!(
            default_checkout_path(
                "/home/me/.herdr/worktrees",
                Path::new("/repo/herdr"),
                "herdr",
                "worktree/brave-river",
            ),
            PathBuf::from("/home/me/.herdr/worktrees/herdr/worktree-brave-river")
        );
    }

    #[test]
    fn default_checkout_path_expands_tilde_directory() {
        let old_home = std::env::var_os("HOME");
        std::env::set_var("HOME", "/home/me");
        assert_eq!(
            default_checkout_path(
                "~/.herdr/worktrees",
                Path::new("/repo/herdr"),
                "herdr",
                "worktree/brave-river",
            ),
            PathBuf::from("/home/me/.herdr/worktrees/herdr/worktree-brave-river")
        );
        if let Some(home) = old_home {
            std::env::set_var("HOME", home);
        } else {
            std::env::remove_var("HOME");
        }
    }

    #[test]
    fn ensure_in_repo_worktree_ignored_writes_exclude_for_relative_directory() {
        let repo = create_committed_repo("exclude-write");
        ensure_in_repo_worktree_ignored(&repo, ".herdr/worktrees").unwrap();
        let exclude = std::fs::read_to_string(repo.join(".git/info/exclude")).unwrap();
        assert!(
            exclude.lines().any(|line| line == ".herdr/worktrees"),
            "exclude was {exclude:?}"
        );
        std::fs::create_dir_all(repo.join(".herdr/worktrees")).unwrap();
        std::fs::write(repo.join(".herdr/worktrees/probe"), "x\n").unwrap();
        let ignored = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["check-ignore", "-q", ".herdr/worktrees/probe"])
            .status()
            .unwrap()
            .success();
        let _ = std::fs::remove_dir_all(&repo);
        assert!(ignored);
    }

    #[test]
    fn ensure_in_repo_worktree_ignored_is_idempotent() {
        let repo = create_committed_repo("exclude-idempotent");
        ensure_in_repo_worktree_ignored(&repo, ".herdr/worktrees").unwrap();
        let first = std::fs::read_to_string(repo.join(".git/info/exclude")).unwrap();
        ensure_in_repo_worktree_ignored(&repo, ".herdr/worktrees").unwrap();
        let second = std::fs::read_to_string(repo.join(".git/info/exclude")).unwrap();
        let _ = std::fs::remove_dir_all(&repo);
        assert_eq!(first, second);
    }

    #[test]
    fn ensure_in_repo_worktree_ignored_skips_absolute_directory() {
        let repo = create_committed_repo("exclude-skip-absolute");
        let before = std::fs::read_to_string(repo.join(".git/info/exclude")).ok();
        ensure_in_repo_worktree_ignored(&repo, "/tmp/herdr-worktrees").unwrap();
        let after = std::fs::read_to_string(repo.join(".git/info/exclude")).ok();
        let _ = std::fs::remove_dir_all(&repo);
        assert_eq!(before, after);
    }

    #[test]
    fn worktree_remove_command_preserves_branch_by_not_deleting_it() {
        let command = build_worktree_remove_command(
            Path::new("/repo/herdr"),
            Path::new("/w/herdr/issue-137"),
            false,
        );
        assert_eq!(command.program, "git");
        assert_eq!(
            command.args,
            vec![
                "-C",
                "/repo/herdr",
                "worktree",
                "remove",
                "/w/herdr/issue-137"
            ]
        );
    }

    #[test]
    fn forced_worktree_remove_command_uses_git_force_flag() {
        let command = build_worktree_remove_command(
            Path::new("/repo/herdr"),
            Path::new("/w/herdr/issue-137"),
            true,
        );
        assert_eq!(
            command.args,
            vec![
                "-C",
                "/repo/herdr",
                "worktree",
                "remove",
                "--force",
                "/w/herdr/issue-137"
            ]
        );
    }

    #[test]
    fn dirty_remove_error_detection_matches_git_force_hint() {
        assert!(is_dirty_worktree_remove_error(
            "fatal: '/w/herdr' contains modified or untracked files, use --force to delete it"
        ));
        assert!(!is_dirty_worktree_remove_error(
            "fatal: '/w/herdr' is a missing but already registered worktree"
        ));
        assert!(!is_dirty_worktree_remove_error(
            "fatal: '/w/herdr' contains a locked worktree, use --force only if you know why"
        ));
    }

    #[test]
    fn worktree_add_command_creates_new_branch_from_base() {
        let command = build_worktree_add_new_branch_command(
            Path::new("/repo/herdr"),
            Path::new("/w/herdr/worktree-brave-river"),
            "worktree/brave-river",
            "HEAD",
        );
        assert_eq!(command.program, "git");
        assert_eq!(
            command.args,
            vec![
                "-C",
                "/repo/herdr",
                "worktree",
                "add",
                "-b",
                "worktree/brave-river",
                "/w/herdr/worktree-brave-river",
                "HEAD"
            ]
        );
    }

    #[test]
    fn run_worktree_add_and_remove_create_and_delete_checkout() {
        let repo = create_committed_repo("worktree-run-repo");
        let checkout = unique_temp_path("worktree-run-checkout");
        let branch = "worktree/test-create-remove";

        let add = build_worktree_add_new_branch_command(&repo, &checkout, branch, "HEAD");
        run_worktree_command(&add).unwrap();

        assert!(checkout.join("README.md").exists());
        let branch_name = std::process::Command::new("git")
            .arg("-C")
            .arg(&checkout)
            .args(["branch", "--show-current"])
            .output()
            .unwrap();
        assert!(branch_name.status.success());
        assert_eq!(
            String::from_utf8(branch_name.stdout).unwrap().trim(),
            branch
        );

        let remove = build_worktree_remove_command(&repo, &checkout, false);
        run_worktree_command(&remove).unwrap();
        assert!(!checkout.exists());

        let _ = std::fs::remove_dir_all(repo);
    }

    #[test]
    fn landing_rebases_and_fast_forwards_parent_checkout() {
        let repo = create_committed_repo("worktree-land-repo");
        let checkout = unique_temp_path("worktree-land-checkout");
        let branch = "worktree/test-land";
        run_worktree_command(&build_worktree_add_new_branch_command(
            &repo, &checkout, branch, "HEAD",
        ))
        .unwrap();
        std::fs::write(checkout.join("agent.txt"), "land me\n").unwrap();
        run_git(&checkout, &["add", "agent.txt"]);
        run_git(&checkout, &["commit", "--quiet", "-m", "agent work"]);

        let outcome = land_worktree(&repo, &checkout, &[]).unwrap();

        assert_eq!(outcome.branch, branch);
        assert!(!outcome.already_landed);
        assert_eq!(
            run_git_output(&repo, &["rev-parse", "HEAD"]).unwrap(),
            outcome.commit
        );
        assert_eq!(
            run_git_output(&repo, &["rev-parse", "HEAD"]).unwrap(),
            run_git_output(&checkout, &["rev-parse", "HEAD"]).unwrap()
        );
        assert!(worktree_already_landed(&checkout, &repo));
        remove_worktree_and_branch(&repo, &checkout, false).unwrap();
        let _ = std::fs::remove_dir_all(repo);
    }

    #[test]
    fn worktree_already_landed_when_it_shares_the_parent_commit() {
        let repo = create_committed_repo("worktree-already-landed-repo");
        let checkout = unique_temp_path("worktree-already-landed-checkout");
        let branch = "worktree/already-landed";
        run_worktree_command(&build_worktree_add_new_branch_command(
            &repo, &checkout, branch, "HEAD",
        ))
        .unwrap();

        assert!(worktree_already_landed(&checkout, &repo));

        std::fs::write(checkout.join("agent.txt"), "not landed\n").unwrap();
        run_git(&checkout, &["add", "agent.txt"]);
        run_git(&checkout, &["commit", "--quiet", "-m", "agent work"]);
        assert!(!worktree_already_landed(&checkout, &repo));

        run_git(&checkout, &["reset", "--soft", "HEAD~1"]);
        assert!(!worktree_already_landed(&checkout, &repo));

        remove_worktree_and_branch(&repo, &checkout, true).unwrap();
        let _ = std::fs::remove_dir_all(repo);
    }

    #[test]
    fn landing_commits_dirty_worktree_then_fast_forwards() {
        let repo = create_committed_repo("worktree-land-dirty-repo");
        let checkout = unique_temp_path("worktree-land-dirty-checkout");
        let branch = "worktree/test-land-dirty";
        run_worktree_command(&build_worktree_add_new_branch_command(
            &repo, &checkout, branch, "HEAD",
        ))
        .unwrap();
        std::fs::write(checkout.join("agent.txt"), "commit then land\n").unwrap();

        let outcome = land_worktree(&repo, &checkout, &[]).unwrap();

        assert!(outcome.committed);
        assert!(!outcome.already_landed);
        assert_eq!(
            run_git_output(&repo, &["rev-parse", "HEAD"]).unwrap(),
            outcome.commit
        );
        assert_eq!(
            std::fs::read_to_string(repo.join("agent.txt"))
                .unwrap()
                .replace("\r\n", "\n"),
            "commit then land\n"
        );
        remove_worktree_and_branch(&repo, &checkout, false).unwrap();
        let _ = std::fs::remove_dir_all(repo);
    }

    #[test]
    fn landing_refuses_a_dirty_parent_checkout() {
        let repo = create_committed_repo("worktree-land-dirty-parent");
        let checkout = unique_temp_path("worktree-land-dirty-parent-checkout");
        let branch = "worktree/test-land-dirty-parent";
        run_worktree_command(&build_worktree_add_new_branch_command(
            &repo, &checkout, branch, "HEAD",
        ))
        .unwrap();
        std::fs::write(checkout.join("agent.txt"), "land me\n").unwrap();
        run_git(&checkout, &["add", "agent.txt"]);
        run_git(&checkout, &["commit", "--quiet", "-m", "agent work"]);
        std::fs::write(repo.join("README.md"), "parent dirty\n").unwrap();

        let error = land_worktree(&repo, &checkout, &[]).unwrap_err();

        assert!(
            error.contains("parent checkout has uncommitted changes"),
            "{error}"
        );
        assert_eq!(
            std::fs::read_to_string(repo.join("README.md")).unwrap(),
            "parent dirty\n"
        );
        remove_worktree_and_branch(&repo, &checkout, true).unwrap();
        let _ = std::fs::remove_dir_all(repo);
    }

    #[test]
    fn landing_stops_before_parent_on_verify_failure() {
        let repo = create_committed_repo("worktree-verify-repo");
        let checkout = unique_temp_path("worktree-verify-checkout");
        let branch = "worktree/test-verify";
        let parent_before = run_git_output(&repo, &["rev-parse", "HEAD"]).unwrap();
        run_worktree_command(&build_worktree_add_new_branch_command(
            &repo, &checkout, branch, "HEAD",
        ))
        .unwrap();
        std::fs::write(checkout.join("agent.txt"), "do not land\n").unwrap();
        run_git(&checkout, &["add", "agent.txt"]);
        run_git(&checkout, &["commit", "--quiet", "-m", "agent work"]);

        let error = land_worktree(
            &repo,
            &checkout,
            &["git".into(), "rev-parse".into(), "missing-ref".into()],
        )
        .unwrap_err();

        assert!(error.starts_with("verify command failed:"));
        assert_eq!(
            run_git_output(&repo, &["rev-parse", "HEAD"]).unwrap(),
            parent_before
        );
        remove_worktree_and_branch(&repo, &checkout, true).unwrap();
        let _ = std::fs::remove_dir_all(repo);
    }

    #[test]
    fn safe_delete_refuses_unlanded_commits_and_force_deletes_branch() {
        let repo = create_committed_repo("worktree-delete-repo");
        let checkout = unique_temp_path("worktree-delete-checkout");
        let branch = "worktree/test-delete";
        run_worktree_command(&build_worktree_add_new_branch_command(
            &repo, &checkout, branch, "HEAD",
        ))
        .unwrap();
        std::fs::write(checkout.join("agent.txt"), "unlanded\n").unwrap();
        run_git(&checkout, &["add", "agent.txt"]);
        run_git(&checkout, &["commit", "--quiet", "-m", "agent work"]);

        let error = remove_worktree_and_branch(&repo, &checkout, false).unwrap_err();
        assert!(error.contains("1 commit(s) that have not landed"));
        assert!(checkout.exists());

        remove_worktree_and_branch(&repo, &checkout, true).unwrap();
        assert!(!checkout.exists());
        assert!(run_git_output(
            &repo,
            &["show-ref", "--verify", &format!("refs/heads/{branch}")]
        )
        .is_err());
        let _ = std::fs::remove_dir_all(repo);
    }
}
