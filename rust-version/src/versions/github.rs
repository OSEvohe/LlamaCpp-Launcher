//! GitHub releases client for llama.cpp.
//!
//! Fetches available releases from the GitHub API with basic caching,
//! timeout, and explicit rate-limit / offline error types.

use std::sync::Mutex;

use reqwest::header::{ACCEPT, USER_AGENT};

use crate::models::{GitHubRelease, GitHubReleaseAsset};

const GITHUB_RELEASES_URL: &str = "https://api.github.com/repos/ggerganov/llama.cpp/releases";
const CACHE_TTL_SECS: u64 = 300; // 5 minutes

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Errors from the GitHub client layer.
#[derive(Debug)]
pub enum GitHubError {
    /// HTTP request failed (network, DNS, timeout, etc.).
    Request(String),
    /// GitHub API rate limit exceeded.
    RateLimit { reset_at: u64 },
    /// Unexpected HTTP status from GitHub.
    HttpError { status: u16, body: String },
    /// Failed to parse the GitHub API response.
    Parse(String),
}

impl std::fmt::Display for GitHubError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Request(msg) => write!(f, "GitHub request failed: {}", msg),
            Self::RateLimit { reset_at } => {
                write!(f, "GitHub API rate limit exceeded (resets at {})", reset_at)
            }
            Self::HttpError { status, body } => {
                write!(f, "GitHub HTTP error {}: {}", status, body)
            }
            Self::Parse(msg) => write!(f, "Failed to parse GitHub response: {}", msg),
        }
    }
}

// ---------------------------------------------------------------------------
// In-memory cache (process-lifetime, simple TTL)
// ---------------------------------------------------------------------------

struct ReleaseCache {
    data: Option<Vec<GitHubRelease>>,
    fetched_at: u128, // unix epoch in millis
}

impl ReleaseCache {
    fn is_valid(&self) -> bool {
        if let Some(data) = &self.data {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0);
            data.is_empty() == false && now.saturating_sub(self.fetched_at) < (CACHE_TTL_SECS as u128) * 1000
        } else {
            false
        }
    }
}

static CACHE: Mutex<ReleaseCache> = Mutex::new(ReleaseCache {
    data: None,
    fetched_at: 0,
});

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Fetch the list of available llama.cpp releases from GitHub.
///
/// Uses an in-memory TTL cache (5 min) to avoid hammering the API.
pub async fn fetch_releases() -> Result<Vec<GitHubRelease>, GitHubError> {
    // Check cache first
    {
        let cache = CACHE.lock().unwrap();
        if cache.is_valid() {
            return Ok(cache.data.clone().unwrap_or_default());
        }
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| GitHubError::Request(e.to_string()))?;

    let resp = client
        .get(GITHUB_RELEASES_URL)
        .header(USER_AGENT, "LLama-Launcher/0.1")
        .header(ACCEPT, "application/vnd.github.v3+json")
        .send()
        .await
        .map_err(|e| GitHubError::Request(e.to_string()))?;

    let status_code = resp.status();
    let status = status_code.as_u16();
    let headers = resp.headers().clone();
    let body = resp.text().await.map_err(|e| GitHubError::Request(e.to_string()))?;

    // Check for rate-limit 403
    if status == 403 {
        if let Some(reset) = headers
            .get("X-RateLimit-Reset")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
        {
            return Err(GitHubError::RateLimit { reset_at: reset });
        }
    }

    if !status_code.is_success() {
        return Err(GitHubError::HttpError {
            status,
            body: body.truncate(500),
        });
    }

    let releases: Vec<GitHubRelease> =
        serde_json::from_str(&body).map_err(|e| GitHubError::Parse(e.to_string()))?;

    // Update cache
    {
        let mut cache = CACHE.lock().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        cache.data = Some(releases.clone());
        cache.fetched_at = now;
    }

    Ok(releases)
}

/// Filter a release's assets to find the best Windows ``llama-server`` binary
/// suitable for install.
///
/// Prefers plain (CPU) builds over CUDA builds.
pub fn find_windows_asset(assets: &[GitHubReleaseAsset]) -> Option<GitHubReleaseAsset> {
    // First pass: prefer non-CUDA (CPU) builds
    let cpu_asset = assets
        .iter()
        .filter(|a| a.name.ends_with(".zip"))
        .filter(|a| a.name.contains("llama-server"))
        .filter(|a| a.name.contains("bin-win"))
        .filter(|a| !a.name.contains("cuda") && !a.name.contains("vulkan") && !a.name.contains("rocm"))
        .find(|a| !a.name.contains("-patches"))
        .cloned();

    if cpu_asset.is_some() {
        return cpu_asset;
    }

    // Fallback: any bin-win zip with llama-server
    assets
        .iter()
        .filter(|a| a.name.ends_with(".zip"))
        .filter(|a| a.name.contains("llama-server"))
        .filter(|a| a.name.contains("bin-win"))
        .find(|a| !a.name.contains("-patches"))
        .cloned()
}

/// Check whether a release tag looks like a valid llama.cpp release.
///
/// llama.cpp tags follow patterns like ``b3594``, ``b4000``, etc.
pub fn is_valid_release_tag(tag: &str) -> bool {
    tag.starts_with('b') && tag[1..].chars().all(|c| c.is_ascii_digit())
}

// ---------------------------------------------------------------------------
// Truncate helper (not in std)
// ---------------------------------------------------------------------------

trait TruncateExt {
    fn truncate(self, max: usize) -> String;
}

impl TruncateExt for String {
    fn truncate(self, max: usize) -> String {
        if self.len() <= max {
            self
        } else {
            format!("{}...", &self[..max])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_valid_release_tag() {
        assert!(is_valid_release_tag("b3594"));
        assert!(is_valid_release_tag("b4000"));
        assert!(!is_valid_release_tag("B3594"));
        assert!(!is_valid_release_tag("b3594a"));
        assert!(!is_valid_release_tag("v0.1"));
    }

    #[test]
    fn test_find_windows_asset_prefers_cpu() {
        let assets = vec![
            GitHubReleaseAsset {
                name: "llama-server-b3594-bin-win-cuda-master.zip".into(),
                size_bytes: 50_000_000,
                download_url: "https://example.com/cuda.zip".into(),
            },
            GitHubReleaseAsset {
                name: "llama-server-b3594-bin-win-ssl.zip".into(),
                size_bytes: 20_000_000,
                download_url: "https://example.com/cpu.zip".into(),
            },
        ];
        let asset = find_windows_asset(&assets);
        assert!(asset.is_some());
        assert!(asset.unwrap().name.contains("ssl"));
    }

    #[test]
    fn test_find_windows_asset_cuda_only() {
        let assets = vec![
            GitHubReleaseAsset {
                name: "llama-server-b3594-bin-win-cuda-master.zip".into(),
                size_bytes: 50_000_000,
                download_url: "https://example.com/cuda.zip".into(),
            },
        ];
        let asset = find_windows_asset(&assets);
        assert!(asset.is_some());
        assert!(asset.unwrap().name.contains("cuda"));
    }

    #[test]
    fn test_find_windows_asset_no_match() {
        let assets = vec![
            GitHubReleaseAsset {
                name: "llama-cli-b3594-bin-win.zip".into(),
                size_bytes: 10_000_000,
                download_url: "https://example.com/cli.zip".into(),
            },
        ];
        let asset = find_windows_asset(&assets);
        assert!(asset.is_none());
    }
}
