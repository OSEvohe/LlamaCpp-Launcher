//! GitHub catalog and install lifecycle for llama.cpp versions.
//!
//! # Module layout
//!
//! - ``github`` — GitHub releases client (fetch, cache, filter)
//! - ``installer`` — download, extract, validate, cleanup

pub mod github;
pub mod installer;

// Re-exports
pub use github::{fetch_releases, find_windows_asset, GitHubError};
pub use installer::{download_file, extract_zip, remove_dir_all_force, remove_file_force};
