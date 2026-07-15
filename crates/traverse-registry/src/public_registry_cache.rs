use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicRegistryCacheErrorCode {
    InvalidDigest,
    DigestMismatch,
    CacheReadFailed,
    CacheWriteFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicRegistryCacheError {
    pub code: PublicRegistryCacheErrorCode,
    pub path: PathBuf,
    pub message: String,
}

/// Returns the shared content-addressed storage location for a SHA-256 digest.
#[must_use]
pub fn public_registry_cache_path(workspace_root: &Path, digest: &str) -> Option<PathBuf> {
    normalize_digest(digest).map(|digest| {
        workspace_root
            .join(".traverse")
            .join("cache")
            .join("sha256")
            .join(digest)
    })
}

/// Verifies bytes and persists them in the shared content-addressed cache.
///
/// Existing cache entries are read and digest-verified before reuse. A cache
/// miss is written through a temporary sibling so callers never observe a
/// partial artifact.
///
/// # Errors
///
/// Returns a stable error when the declared digest is invalid, content does
/// not match it, an existing entry cannot be read, or the cache cannot be
/// committed atomically.
pub fn cache_verified_public_registry_bytes(
    workspace_root: &Path,
    declared_digest: &str,
    bytes: &[u8],
) -> Result<PathBuf, PublicRegistryCacheError> {
    let Some(path) = public_registry_cache_path(workspace_root, declared_digest) else {
        return Err(cache_error(
            PublicRegistryCacheErrorCode::InvalidDigest,
            workspace_root,
            format!(
                "public registry digest must be sha256: followed by 64 hex characters: {declared_digest}"
            ),
        ));
    };
    let expected = normalize_digest(declared_digest).unwrap_or_default();
    if sha256_hex(bytes) != expected {
        return Err(cache_error(
            PublicRegistryCacheErrorCode::DigestMismatch,
            &path,
            format!(
                "public registry content digest mismatch for {}",
                path.display()
            ),
        ));
    }
    if path.exists() {
        let cached = fs::read(&path).map_err(|error| {
            cache_error(
                PublicRegistryCacheErrorCode::CacheReadFailed,
                &path,
                format!("failed to read cached public registry content: {error}"),
            )
        })?;
        if sha256_hex(&cached) != expected {
            return Err(cache_error(
                PublicRegistryCacheErrorCode::DigestMismatch,
                &path,
                format!(
                    "cached public registry content digest mismatch for {}",
                    path.display()
                ),
            ));
        }
        return Ok(path);
    }
    let parent = path.parent().ok_or_else(|| {
        cache_error(
            PublicRegistryCacheErrorCode::CacheWriteFailed,
            &path,
            "public registry cache path has no parent".to_string(),
        )
    })?;
    fs::create_dir_all(parent).map_err(|error| {
        cache_error(
            PublicRegistryCacheErrorCode::CacheWriteFailed,
            parent,
            format!("failed to create public registry cache directory: {error}"),
        )
    })?;
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, bytes).map_err(|error| {
        cache_error(
            PublicRegistryCacheErrorCode::CacheWriteFailed,
            &temporary,
            format!("failed to write public registry cache entry: {error}"),
        )
    })?;
    fs::rename(&temporary, &path).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        cache_error(
            PublicRegistryCacheErrorCode::CacheWriteFailed,
            &path,
            format!("failed to commit public registry cache entry: {error}"),
        )
    })?;
    Ok(path)
}

fn normalize_digest(digest: &str) -> Option<String> {
    let digest = digest.strip_prefix("sha256:")?;
    if digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Some(digest.to_ascii_lowercase())
    } else {
        None
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut value = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(value, "{byte:02x}");
    }
    value
}

fn cache_error(
    code: PublicRegistryCacheErrorCode,
    path: &Path,
    message: String,
) -> PublicRegistryCacheError {
    PublicRegistryCacheError {
        code,
        path: path.to_path_buf(),
        message,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn caches_and_revalidates_verified_content() {
        let root = unique_temp_dir();
        let bytes = b"public contract";
        let digest = format!("sha256:{}", sha256_hex(bytes));
        let path = cache_verified_public_registry_bytes(&root, &digest, bytes)
            .expect("content should cache");
        assert_eq!(fs::read(&path).expect("cache should read"), bytes);
        cache_verified_public_registry_bytes(&root, &digest, bytes)
            .expect("valid cache hit should succeed");
        fs::write(&path, b"tampered").expect("cache should be writable for test");
        let failure = cache_verified_public_registry_bytes(&root, &digest, bytes)
            .expect_err("tampered cache must not be reused");
        assert_eq!(failure.code, PublicRegistryCacheErrorCode::DigestMismatch);
    }

    #[test]
    fn rejects_mismatched_declared_digest() {
        let root = unique_temp_dir();
        let failure = cache_verified_public_registry_bytes(
            &root,
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            b"different",
        )
        .expect_err("mismatched bytes must fail");
        assert_eq!(failure.code, PublicRegistryCacheErrorCode::DigestMismatch);
    }

    fn unique_temp_dir() -> PathBuf {
        let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time should be after epoch")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("traverse-public-cache-test-{nanos}-{counter}"));
        fs::create_dir_all(&path).expect("temp directory must be created");
        path
    }
}
