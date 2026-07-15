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
    let parent = workspace_root
        .join(".traverse")
        .join("cache")
        .join("sha256");
    fs::create_dir_all(&parent).map_err(|error| {
        cache_error(
            PublicRegistryCacheErrorCode::CacheWriteFailed,
            &parent,
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
    commit_cache_entry(&temporary, &path)?;
    Ok(path)
}

fn commit_cache_entry(temporary: &Path, path: &Path) -> Result<(), PublicRegistryCacheError> {
    fs::rename(temporary, path).map_err(|error| {
        let _ = fs::remove_file(temporary);
        cache_error(
            PublicRegistryCacheErrorCode::CacheWriteFailed,
            path,
            format!("failed to commit public registry cache entry: {error}"),
        )
    })
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

    #[test]
    fn rejects_invalid_digest_shapes() {
        let root = unique_temp_dir();
        for digest in [
            "invalid",
            "sha256:short",
            "sha256:gggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggg",
        ] {
            let failure = cache_verified_public_registry_bytes(&root, digest, b"content")
                .expect_err("invalid digest must fail");
            assert_eq!(failure.code, PublicRegistryCacheErrorCode::InvalidDigest);
        }
    }

    #[test]
    fn reports_cache_read_and_directory_creation_failures() {
        let root = unique_temp_dir();
        let bytes = b"public contract";
        let digest = format!("sha256:{}", sha256_hex(bytes));
        let path = public_registry_cache_path(&root, &digest).expect("digest should be valid");
        fs::create_dir_all(&path).expect("cache path directory should be created");
        let read_failure = cache_verified_public_registry_bytes(&root, &digest, bytes)
            .expect_err("directory cache entry must not read as bytes");
        assert_eq!(
            read_failure.code,
            PublicRegistryCacheErrorCode::CacheReadFailed
        );

        let blocked_root = unique_temp_dir();
        fs::write(blocked_root.join(".traverse"), b"not a directory")
            .expect("blocking file should be created");
        let write_failure = cache_verified_public_registry_bytes(&blocked_root, &digest, bytes)
            .expect_err("cache directory creation must fail through a file");
        assert_eq!(
            write_failure.code,
            PublicRegistryCacheErrorCode::CacheWriteFailed
        );
    }

    #[test]
    fn reports_temporary_write_and_commit_failures() {
        let root = unique_temp_dir();
        let bytes = b"public contract";
        let digest = format!("sha256:{}", sha256_hex(bytes));
        let path = public_registry_cache_path(&root, &digest).expect("digest should be valid");
        let parent = path.parent().expect("cache path should have a parent");
        fs::create_dir_all(parent).expect("cache parent should be created");
        fs::create_dir(path.with_extension("tmp")).expect("temporary path should be blocked");
        let write_failure = cache_verified_public_registry_bytes(&root, &digest, bytes)
            .expect_err("writing through a directory must fail");
        assert_eq!(
            write_failure.code,
            PublicRegistryCacheErrorCode::CacheWriteFailed
        );

        let commit_root = unique_temp_dir();
        let temporary = commit_root.join("entry.tmp");
        let destination = commit_root.join("entry");
        fs::write(&temporary, bytes).expect("temporary entry should be written");
        fs::create_dir(&destination).expect("destination directory should block rename");
        let commit_failure = commit_cache_entry(&temporary, &destination)
            .expect_err("rename onto a directory must fail");
        assert_eq!(
            commit_failure.code,
            PublicRegistryCacheErrorCode::CacheWriteFailed
        );
        assert!(!temporary.exists());
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
