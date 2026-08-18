mod config;
#[cfg(test)]
mod config_tests;
mod discovery;
mod status;
#[cfg(test)]
mod test_support;

pub use self::{
    discovery::{
        derive_label_from_cwd, git_branch, git_space_metadata, in_git_repo, GitSpaceMetadata,
    },
    status::{git_status_cache_key, git_status_snapshot_for_cwd, GitStatusCacheEntry},
};
