//! Shared public-registry index sync (Spec `055-registry-sync` 1.2.0,
//! Spec `119-verified-registry-mcp-mode-a` 0.4.0, Decision 121).
//!
//! `traverse-cli registry sync` and `traverse-mcp sync` both fetch the
//! registry index through this crate: index-release discovery, the
//! `index.json` fetch, its digest check, validation, `--source-repo`, and
//! `TRAVERSE_REGISTRY_TOKEN`. Each binary owns where the synced state goes.

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use traverse_registry::{
    PublicRegistryIndex, PublicRegistryStateErrorCode, PublicRegistryStateFailure,
    SyncedPublicRegistryState, synced_public_registry_state_path,
    write_synced_public_registry_state,
};

/// The public registry synced when no `--source-repo` is given.
pub const DEFAULT_PUBLIC_REGISTRY_SOURCE: &str = "traverse-framework/registry";

/// Environment variable holding a read token for a non-default source repo.
pub const REGISTRY_TOKEN_ENV: &str = "TRAVERSE_REGISTRY_TOKEN";

/// One fetched registry index and where it came from.
#[derive(Debug, Clone)]
pub struct FetchedRegistryIndex {
    /// `owner/repo` the index was fetched from.
    pub source_repo: String,
    /// The `index-vN` release tag that supplied `index.json`.
    pub release_tag: String,
    /// The parsed index.
    pub index: PublicRegistryIndex,
}

/// Source of a registry index.
pub trait RegistryIndexFetcher {
    /// Fetch the index this fetcher is configured for: the highest `index-v*`
    /// release with an `index.json` asset, or a pinned release.
    ///
    /// # Errors
    ///
    /// Returns a stable [`RegistrySyncError`] when discovery, fetch, digest
    /// check, or parsing fails.
    fn fetch_latest_index(&self) -> Result<FetchedRegistryIndex, RegistrySyncError>;
}

/// Stable registry-sync failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistrySyncError {
    /// Stable machine-readable code.
    pub code: &'static str,
    /// Human-readable explanation.
    pub message: String,
}

impl RegistrySyncError {
    /// Build a failure from a stable code and a message.
    #[must_use]
    pub fn new(code: &'static str, message: String) -> Self {
        Self { code, message }
    }
}

/// The effective source repo: the override when supplied, otherwise the
/// default public registry.
#[must_use]
pub fn registry_sync_default_or_override(source_repo_override: Option<String>) -> String {
    source_repo_override.unwrap_or_else(|| DEFAULT_PUBLIC_REGISTRY_SOURCE.to_string())
}

/// Text fetch used by the GitHub fetcher.
pub trait RegistryHttp {
    /// Fetch `url` as UTF-8 text, with an optional bearer token.
    ///
    /// # Errors
    ///
    /// Returns `registry_fetch_failed` when the fetch fails.
    fn get_text(&self, url: &str, token: Option<&str>) -> Result<String, RegistrySyncError>;
}

/// [`RegistryHttp`] through the `curl` binary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurlHttp {
    program: String,
}

impl Default for CurlHttp {
    fn default() -> Self {
        Self {
            program: "curl".to_string(),
        }
    }
}

impl RegistryHttp for CurlHttp {
    fn get_text(&self, url: &str, token: Option<&str>) -> Result<String, RegistrySyncError> {
        let mut args = vec![
            "-fsSL".to_string(),
            "-H".to_string(),
            "Accept: application/vnd.github+json".to_string(),
            "-H".to_string(),
            "User-Agent: traverse-cli-registry-sync".to_string(),
        ];
        if let Some(token) = token {
            args.push("-H".to_string());
            args.push(format!("Authorization: Bearer {token}"));
        }
        args.push(url.to_string());
        let output = Command::new(&self.program)
            .args(&args)
            .output()
            .map_err(|error| {
                RegistrySyncError::new(
                    "registry_fetch_failed",
                    format!("failed to execute curl for {url}: {error}"),
                )
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let detail = if stderr.is_empty() {
                format!("curl exited with status {}", output.status)
            } else {
                stderr
            };
            return Err(RegistrySyncError::new(
                "registry_fetch_failed",
                format!("failed to fetch {url}: {detail}"),
            ));
        }
        String::from_utf8(output.stdout).map_err(|error| {
            RegistrySyncError::new(
                "registry_fetch_failed",
                format!("response from {url} was not valid UTF-8: {error}"),
            )
        })
    }
}

/// Fetch `url` as text with `curl`.
///
/// # Errors
///
/// Returns `registry_fetch_failed` when curl cannot run, exits non-zero, or
/// returns non-UTF-8 bytes.
pub fn curl_text(url: &str, token: Option<&str>) -> Result<String, RegistrySyncError> {
    CurlHttp::default().get_text(url, token)
}

/// Fetches a registry index from a GitHub repo's `index-v*` releases.
#[derive(Debug, Clone)]
pub struct GitHubRegistryIndexFetcher<H = CurlHttp> {
    source_repo: String,
    token: Option<String>,
    release: Option<String>,
    http: H,
}

/// The production fetcher: GitHub Releases through `curl`.
pub type CurlGitHubRegistryIndexFetcher = GitHubRegistryIndexFetcher<CurlHttp>;

impl GitHubRegistryIndexFetcher<CurlHttp> {
    /// Fetcher for `source_repo_override` (default public registry when
    /// `None`), optionally pinned to one `index-vN` release.
    ///
    /// The token from [`REGISTRY_TOKEN_ENV`] is read only for a non-default
    /// source. The default public registry stays unauthenticated even when
    /// the variable is set for an unrelated reason.
    #[must_use]
    pub fn for_source(source_repo_override: Option<String>, release: Option<String>) -> Self {
        let token = if source_repo_override.is_none() {
            None
        } else {
            std::env::var(REGISTRY_TOKEN_ENV).ok()
        };
        Self::with_http(
            registry_sync_default_or_override(source_repo_override),
            token,
            release,
            CurlHttp::default(),
        )
    }
}

impl<H: RegistryHttp> GitHubRegistryIndexFetcher<H> {
    /// Fetcher over an explicit [`RegistryHttp`].
    #[must_use]
    pub fn with_http(
        source_repo: String,
        token: Option<String>,
        release: Option<String>,
        http: H,
    ) -> Self {
        Self {
            source_repo,
            token,
            release,
            http,
        }
    }

    /// The `owner/repo` this fetcher reads.
    #[must_use]
    pub fn source_repo(&self) -> &str {
        &self.source_repo
    }
}

impl<H: RegistryHttp> RegistryIndexFetcher for GitHubRegistryIndexFetcher<H> {
    fn fetch_latest_index(&self) -> Result<FetchedRegistryIndex, RegistrySyncError> {
        // Reject a malformed pin before any network access.
        if let Some(pinned) = self.release.as_deref() {
            check_release_tag(pinned)?;
        }
        let releases_url = format!(
            "https://api.github.com/repos/{}/releases?per_page=100",
            self.source_repo
        );
        let releases = self.http.get_text(&releases_url, self.token.as_deref())?;
        let releases_json: Value = serde_json::from_str(&releases).map_err(|error| {
            RegistrySyncError::new(
                "registry_release_parse_failed",
                format!("failed to parse GitHub Releases response: {error}"),
            )
        })?;
        let asset = select_index_release(&releases_json, self.release.as_deref())?;
        let index_json = self.http.get_text(&asset.url, self.token.as_deref())?;
        let index = parse_registry_index(&index_json, asset.digest.as_deref())?;
        Ok(FetchedRegistryIndex {
            source_repo: self.source_repo.clone(),
            release_tag: asset.release_tag,
            index,
        })
    }
}

/// The `index.json` asset of one `index-vN` release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexReleaseAsset {
    /// The `index-vN` release tag.
    pub release_tag: String,
    /// Download URL of the `index.json` asset.
    pub url: String,
    /// The asset's published `sha256:` digest, when GitHub reports one.
    pub digest: Option<String>,
}

/// The highest `index-v*` release that has an `index.json` asset.
///
/// # Errors
///
/// Returns `registry_release_parse_failed` for a non-array response, or
/// `registry_index_release_missing` when no release qualifies.
pub fn latest_index_release_asset(
    releases_json: &Value,
) -> Result<(String, String), RegistrySyncError> {
    select_index_release(releases_json, None).map(|asset| (asset.release_tag, asset.url))
}

/// Select the `index.json` asset: the highest `index-v*` release that has
/// one, or the pinned `release`.
///
/// # Errors
///
/// - `registry_release_parse_failed`: the response is not an array.
/// - `registry_release_invalid`: `release` is not `index-v<N>`.
/// - `registry_index_release_missing`: no qualifying release, or the pinned
///   release does not exist.
/// - `registry_index_asset_missing`: the pinned release has no `index.json`.
pub fn select_index_release(
    releases_json: &Value,
    release: Option<&str>,
) -> Result<IndexReleaseAsset, RegistrySyncError> {
    let releases = releases_json.as_array().ok_or_else(|| {
        RegistrySyncError::new(
            "registry_release_parse_failed",
            "GitHub Releases response must be an array".to_string(),
        )
    })?;
    match release {
        Some(pinned) => select_pinned_release(releases, pinned),
        None => select_highest_release(releases),
    }
}

fn select_highest_release(releases: &[Value]) -> Result<IndexReleaseAsset, RegistrySyncError> {
    let mut selected: Option<(u64, IndexReleaseAsset)> = None;
    for release in releases {
        let Some(tag) = release.get("tag_name").and_then(Value::as_str) else {
            continue;
        };
        let Some(index_version) = index_release_version(tag) else {
            continue;
        };
        let Some(asset) = index_asset(release, tag) else {
            continue;
        };
        match &selected {
            Some((selected_version, _)) if *selected_version >= index_version => {}
            _ => selected = Some((index_version, asset)),
        }
    }
    selected.map(|(_, asset)| asset).ok_or_else(|| {
        RegistrySyncError::new(
            "registry_index_release_missing",
            "no index-v* release with an index.json asset was found".to_string(),
        )
    })
}

fn select_pinned_release(
    releases: &[Value],
    pinned: &str,
) -> Result<IndexReleaseAsset, RegistrySyncError> {
    check_release_tag(pinned)?;
    let release = releases
        .iter()
        .find(|release| release.get("tag_name").and_then(Value::as_str) == Some(pinned))
        .ok_or_else(|| {
            RegistrySyncError::new(
                "registry_index_release_missing",
                format!("release {pinned} was not found"),
            )
        })?;
    index_asset(release, pinned).ok_or_else(|| {
        RegistrySyncError::new(
            "registry_index_asset_missing",
            format!("release {pinned} has no index.json asset"),
        )
    })
}

fn check_release_tag(pinned: &str) -> Result<(), RegistrySyncError> {
    match index_release_version(pinned) {
        Some(_) => Ok(()),
        None => Err(RegistrySyncError::new(
            "registry_release_invalid",
            format!("--release must be index-v<N>, got {pinned}"),
        )),
    }
}

fn index_release_version(tag: &str) -> Option<u64> {
    tag.strip_prefix("index-v")
        .and_then(|version| version.parse::<u64>().ok())
}

fn index_asset(release: &Value, tag: &str) -> Option<IndexReleaseAsset> {
    let assets = release.get("assets").and_then(Value::as_array)?;
    assets.iter().find_map(|asset| {
        if asset.get("name").and_then(Value::as_str) != Some("index.json") {
            return None;
        }
        let url = asset.get("browser_download_url").and_then(Value::as_str)?;
        Some(IndexReleaseAsset {
            release_tag: tag.to_string(),
            url: url.to_string(),
            digest: asset
                .get("digest")
                .and_then(Value::as_str)
                .map(ToString::to_string),
        })
    })
}

/// Parse `index.json`, first checking its bytes against the asset's
/// published `sha256:` digest when one is given.
///
/// # Errors
///
/// Returns `registry_index_digest_mismatch` when the bytes do not match the
/// published digest, or `registry_index_parse_failed` when the JSON is not a
/// registry index.
pub fn parse_registry_index(
    index_json: &str,
    expected_digest: Option<&str>,
) -> Result<PublicRegistryIndex, RegistrySyncError> {
    if let Some(expected) = expected_digest.and_then(|digest| digest.strip_prefix("sha256:")) {
        let actual = sha256_hex(index_json.as_bytes());
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(RegistrySyncError::new(
                "registry_index_digest_mismatch",
                format!(
                    "index.json sha256 {actual} does not match the published sha256:{expected}"
                ),
            ));
        }
    }
    serde_json::from_str(index_json).map_err(|error| {
        RegistrySyncError::new(
            "registry_index_parse_failed",
            format!("failed to parse registry index.json: {error}"),
        )
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

/// Workspace id recorded in a synced-state file written by [`sync_to_file`].
pub const FILE_SYNC_WORKSPACE_ID: &str = "traverse-mcp";

/// Fetch the index and atomically write it as one Spec 055 synced-state file.
///
/// The file is written to a temporary sibling first and renamed into place,
/// so on any failure no partial file is left and an existing `out` is kept.
///
/// # Errors
///
/// Returns the fetcher's [`RegistrySyncError`], `registry_index_invalid`
/// when the index fails Spec 055 validation, or `registry_sync_write_failed`
/// when the file cannot be written.
pub fn sync_to_file(
    fetcher: &dyn RegistryIndexFetcher,
    out: &Path,
    synced_at: &str,
) -> Result<SyncedPublicRegistryState, RegistrySyncError> {
    let index = fetcher.fetch_latest_index()?;
    let staging = staging_root(out);
    let result = write_synced_public_registry_state(
        &staging,
        FILE_SYNC_WORKSPACE_ID,
        &index.source_repo,
        &index.release_tag,
        synced_at,
        index.index,
    )
    .map_err(|failure| state_failure(&failure))
    .and_then(|state| {
        let staged = synced_public_registry_state_path(&staging, FILE_SYNC_WORKSPACE_ID);
        fs::rename(&staged, out).map_err(|error| {
            RegistrySyncError::new(
                "registry_sync_write_failed",
                format!("failed to move synced state into place: {error}"),
            )
        })?;
        Ok(state)
    });
    let _ = fs::remove_dir_all(&staging);
    result
}

fn staging_root(out: &Path) -> PathBuf {
    let parent = out
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = out
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    parent.join(format!(".{name}.sync-{}", std::process::id()))
}

fn state_failure(failure: &PublicRegistryStateFailure) -> RegistrySyncError {
    let code = if failure
        .errors
        .iter()
        .any(|error| error.code == PublicRegistryStateErrorCode::StateWriteFailed)
    {
        "registry_sync_write_failed"
    } else {
        "registry_index_invalid"
    };
    RegistrySyncError::new(
        code,
        failure
            .errors
            .iter()
            .map(|error| error.message.clone())
            .collect::<Vec<_>>()
            .join("; "),
    )
}

/// `unix:<seconds>`, the `synced_at` format `traverse-cli` records.
#[must_use]
pub fn unix_timestamp_string() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    format!("unix:{seconds}")
}

#[cfg(test)]
mod tests;
