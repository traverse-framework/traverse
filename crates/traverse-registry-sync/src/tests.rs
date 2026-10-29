#![allow(clippy::expect_used)]

use super::*;
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use traverse_registry::PublicRegistryCapabilityRecord;

const RELEASES_URL: &str = "https://api.github.com/repos/acme/registry/releases?per_page=100";

fn temp_dir(tag: &str) -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "traverse-registry-sync-{tag}-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn index_fixture(index_version: u64) -> PublicRegistryIndex {
    PublicRegistryIndex {
        index_version,
        generated_at: "2026-10-29T00:00:00Z".to_string(),
        source_commit: Some("abc123".to_string()),
        capabilities: vec![PublicRegistryCapabilityRecord {
            namespace: "traverse-starter".to_string(),
            id: "traverse-starter.process".to_string(),
            version: "1.0.0".to_string(),
            digest: "sha256:5647c39a".to_string(),
            artifact_url: "https://example.test/starter.wasm".to_string(),
            contract_digest: "sha256:5647c39a".to_string(),
            contract_url: "https://example.test/contract.json".to_string(),
            deprecated: false,
            summary: String::new(),
            description: String::new(),
            use_cases: Vec::new(),
            service_type: "stateless".to_string(),
            permitted_targets: Vec::new(),
            lifecycle: "active".to_string(),
            provenance: None,
            commercial_use: "unknown".to_string(),
            redistribution: "unknown".to_string(),
            verification_status: "unknown".to_string(),
            license_expression: None,
            ai: None,
            status: None,
            model_usage: Vec::new(),
            revocation: None,
        }],
        events: Vec::new(),
    }
}

fn index_text(index_version: u64) -> String {
    serde_json::to_string(&index_fixture(index_version)).expect("index json")
}

fn release(tag: &str, url: Option<&str>, digest: Option<&str>) -> Value {
    let assets = match url {
        Some(url) => {
            let mut asset = json!({"name": "index.json", "browser_download_url": url});
            if let Some(digest) = digest {
                asset["digest"] = json!(digest);
            }
            vec![
                json!({"name": "notes.txt", "browser_download_url": "x"}),
                asset,
            ]
        }
        None => Vec::new(),
    };
    json!({"tag_name": tag, "assets": assets})
}

/// In-memory [`RegistryHttp`] keyed by URL.
struct MapHttp(BTreeMap<String, String>);

impl RegistryHttp for MapHttp {
    fn get_text(&self, url: &str, _token: Option<&str>) -> Result<String, RegistrySyncError> {
        self.0.get(url).cloned().ok_or_else(|| {
            RegistrySyncError::new("registry_fetch_failed", format!("no fixture for {url}"))
        })
    }
}

/// Two releases (index-v7, index-v9) with digests that match their bytes.
fn fake_github() -> MapHttp {
    let v7 = index_text(7);
    let v9 = index_text(9);
    let releases = json!([
        release(
            "index-v7",
            Some("https://example.test/v7/index.json"),
            Some(&format!("sha256:{}", sha256_hex(v7.as_bytes())))
        ),
        release("index-v8", None, None),
        release(
            "index-v9",
            Some("https://example.test/v9/index.json"),
            Some(&format!(
                "sha256:{}",
                sha256_hex(v9.as_bytes()).to_uppercase()
            ))
        ),
    ]);
    MapHttp(BTreeMap::from([
        (RELEASES_URL.to_string(), releases.to_string()),
        ("https://example.test/v7/index.json".to_string(), v7),
        ("https://example.test/v9/index.json".to_string(), v9),
    ]))
}

fn fetcher(http: MapHttp, release: Option<&str>) -> GitHubRegistryIndexFetcher<MapHttp> {
    GitHubRegistryIndexFetcher::with_http(
        "acme/registry".to_string(),
        Some("token".to_string()),
        release.map(ToString::to_string),
        http,
    )
}

#[test]
fn default_source_and_override() {
    assert_eq!(
        registry_sync_default_or_override(None),
        DEFAULT_PUBLIC_REGISTRY_SOURCE
    );
    assert_eq!(
        registry_sync_default_or_override(Some("acme/registry".to_string())),
        "acme/registry"
    );
    let public = CurlGitHubRegistryIndexFetcher::for_source(None, None);
    assert_eq!(public.source_repo(), DEFAULT_PUBLIC_REGISTRY_SOURCE);
    assert_eq!(public.token, None);
    let private = CurlGitHubRegistryIndexFetcher::for_source(
        Some("acme/registry".to_string()),
        Some("index-v3".to_string()),
    );
    assert_eq!(private.source_repo(), "acme/registry");
    assert_eq!(private.release.as_deref(), Some("index-v3"));
}

#[test]
fn fetch_takes_the_highest_index_release_by_default() {
    let fetched = fetcher(fake_github(), None)
        .fetch_latest_index()
        .expect("latest");
    assert_eq!(fetched.source_repo, "acme/registry");
    assert_eq!(fetched.release_tag, "index-v9");
    assert_eq!(fetched.index.index_version, 9);
}

#[test]
fn fetch_honours_a_pinned_release() {
    let fetched = fetcher(fake_github(), Some("index-v7"))
        .fetch_latest_index()
        .expect("pinned");
    assert_eq!(fetched.release_tag, "index-v7");
    assert_eq!(fetched.index.index_version, 7);
}

#[test]
fn pinned_release_failures_are_stable() {
    let missing = fetcher(fake_github(), Some("index-v99"))
        .fetch_latest_index()
        .expect_err("missing release");
    assert_eq!(missing.code, "registry_index_release_missing");
    assert!(missing.message.contains("index-v99"));
    let no_asset = fetcher(fake_github(), Some("index-v8"))
        .fetch_latest_index()
        .expect_err("no asset");
    assert_eq!(no_asset.code, "registry_index_asset_missing");
    let invalid = fetcher(fake_github(), Some("latest"))
        .fetch_latest_index()
        .expect_err("invalid tag");
    assert_eq!(invalid.code, "registry_release_invalid");
    // The pin is checked before any request: an empty fake still yields it.
    let offline = fetcher(MapHttp(BTreeMap::new()), Some("index-vX"))
        .fetch_latest_index()
        .expect_err("invalid tag offline");
    assert_eq!(offline.code, "registry_release_invalid");
    assert_eq!(
        select_index_release(&json!([]), Some("v1"))
            .expect_err("direct")
            .code,
        "registry_release_invalid"
    );
}

#[test]
fn fetch_rejects_a_digest_mismatch() {
    let mut http = fake_github();
    http.0.insert(
        "https://example.test/v9/index.json".to_string(),
        index_text(10),
    );
    let error = fetcher(http, None)
        .fetch_latest_index()
        .expect_err("tampered index");
    assert_eq!(error.code, "registry_index_digest_mismatch");
}

#[test]
fn fetch_reports_malformed_releases_and_index() {
    let mut http = fake_github();
    http.0
        .insert(RELEASES_URL.to_string(), "{not json".to_string());
    assert_eq!(
        fetcher(http, None)
            .fetch_latest_index()
            .expect_err("bad releases")
            .code,
        "registry_release_parse_failed"
    );
    let releases = json!([release(
        "index-v1",
        Some("https://example.test/v1/index.json"),
        None
    )]);
    let http = MapHttp(BTreeMap::from([
        (RELEASES_URL.to_string(), releases.to_string()),
        (
            "https://example.test/v1/index.json".to_string(),
            "{\"index_version\": \"one\"}".to_string(),
        ),
    ]));
    assert_eq!(
        fetcher(http, None)
            .fetch_latest_index()
            .expect_err("malformed index")
            .code,
        "registry_index_parse_failed"
    );
    assert_eq!(
        fetcher(MapHttp(BTreeMap::new()), None)
            .fetch_latest_index()
            .expect_err("unreachable")
            .code,
        "registry_fetch_failed"
    );
}

#[test]
fn release_selection_skips_unrelated_entries() {
    let releases = json!([
        {"name": "untagged"},
        release("notes-v1", Some("https://example.test/notes"), None),
        release("index-v2", Some("https://example.test/v2/index.json"), None),
        release("index-v1", Some("https://example.test/v1/index.json"), None),
        {"tag_name": "index-v5", "assets": [{"name": "index.json"}]},
        {"tag_name": "index-v6"},
    ]);
    let (tag, url) = latest_index_release_asset(&releases).expect("latest");
    assert_eq!(tag, "index-v2");
    assert_eq!(url, "https://example.test/v2/index.json");
    assert_eq!(
        latest_index_release_asset(&json!([]))
            .expect_err("none")
            .code,
        "registry_index_release_missing"
    );
    assert_eq!(
        latest_index_release_asset(&json!({}))
            .expect_err("not array")
            .code,
        "registry_release_parse_failed"
    );
}

#[test]
fn parse_ignores_digests_it_cannot_check() {
    let text = index_text(4);
    assert!(parse_registry_index(&text, Some("sha512:abc")).is_ok());
    assert!(parse_registry_index(&text, None).is_ok());
}

struct StaticFetcher(Result<FetchedRegistryIndex, RegistrySyncError>);

impl RegistryIndexFetcher for StaticFetcher {
    fn fetch_latest_index(&self) -> Result<FetchedRegistryIndex, RegistrySyncError> {
        self.0.clone()
    }
}

fn fetched(index: PublicRegistryIndex) -> StaticFetcher {
    StaticFetcher(Ok(FetchedRegistryIndex {
        source_repo: "acme/registry".to_string(),
        release_tag: format!("index-v{}", index.index_version),
        index,
    }))
}

#[test]
fn sync_to_file_writes_a_loadable_spec_055_state() {
    let dir = temp_dir("write");
    let out = dir.join("synced-state.json");
    let state = sync_to_file(&fetched(index_fixture(9)), &out, "unix:1").expect("sync to file");
    assert_eq!(state.release_tag, "index-v9");
    let written: SyncedPublicRegistryState =
        serde_json::from_slice(&fs::read(&out).expect("written")).expect("state json");
    assert_eq!(written.release_tag, "index-v9");
    assert_eq!(written.source_repo, "acme/registry");
    assert_eq!(written.workspace_id, FILE_SYNC_WORKSPACE_ID);
    assert_eq!(written.governing_spec, "055-registry-sync");
    assert_eq!(written.capabilities.len(), 1);
    let leftovers: Vec<_> = fs::read_dir(&dir)
        .expect("dir")
        .map(|entry| entry.expect("entry").file_name())
        .collect();
    assert_eq!(
        leftovers,
        vec![std::ffi::OsString::from("synced-state.json")]
    );
}

#[test]
fn sync_to_file_failures_keep_the_existing_file() {
    let dir = temp_dir("keep");
    let out = dir.join("synced-state.json");
    fs::write(&out, b"previous").expect("existing");

    let fetch_error = StaticFetcher(Err(RegistrySyncError::new(
        "registry_fetch_failed",
        "offline".to_string(),
    )));
    assert_eq!(
        sync_to_file(&fetch_error, &out, "unix:1")
            .expect_err("fetch")
            .code,
        "registry_fetch_failed"
    );

    let mut malformed = index_fixture(9);
    malformed.capabilities[0].artifact_url = String::new();
    let invalid = sync_to_file(&fetched(malformed), &out, "unix:1").expect_err("invalid");
    assert_eq!(invalid.code, "registry_index_invalid");
    assert!(
        invalid.message.contains("artifact_url"),
        "{}",
        invalid.message
    );

    assert_eq!(fs::read(&out).expect("kept"), b"previous");
    assert_eq!(fs::read_dir(&dir).expect("dir").count(), 1);
}

#[test]
fn sync_to_file_reports_write_failures() {
    let dir = temp_dir("write-fail");
    let blocker = dir.join("not-a-dir");
    fs::write(&blocker, b"file").expect("blocker");
    let error = sync_to_file(
        &fetched(index_fixture(9)),
        &blocker.join("out.json"),
        "unix:1",
    )
    .expect_err("parent is a file");
    assert_eq!(error.code, "registry_sync_write_failed");

    let out_dir = dir.join("occupied");
    fs::create_dir_all(out_dir.join("child")).expect("non-empty dir");
    let error =
        sync_to_file(&fetched(index_fixture(9)), &out_dir, "unix:1").expect_err("out is a dir");
    assert_eq!(error.code, "registry_sync_write_failed");
    assert!(out_dir.join("child").is_dir());
}

#[test]
fn staging_root_is_a_hidden_sibling() {
    let pid = std::process::id();
    assert_eq!(
        staging_root(Path::new("state.json")),
        Path::new(".").join(format!(".state.json.sync-{pid}"))
    );
    assert_eq!(
        staging_root(Path::new("/tmp/a/state.json")),
        Path::new("/tmp/a").join(format!(".state.json.sync-{pid}"))
    );
}

#[test]
fn timestamp_uses_the_cli_format() {
    let stamp = unix_timestamp_string();
    assert!(stamp.starts_with("unix:"));
    assert!(stamp["unix:".len()..].parse::<u64>().is_ok());
}

#[cfg(unix)]
#[test]
fn curl_http_reads_files_and_reports_every_failure() {
    let dir = temp_dir("curl");
    let ok = dir.join("ok.txt");
    fs::write(&ok, b"hello").expect("ok file");
    let url = format!("file://{}", ok.display());
    assert_eq!(curl_text(&url, None).expect("file url"), "hello");
    assert_eq!(curl_text(&url, Some("t")).expect("with token"), "hello");

    let missing =
        curl_text(&format!("file://{}/absent", dir.display()), None).expect_err("missing file");
    assert_eq!(missing.code, "registry_fetch_failed");
    assert!(missing.message.contains("failed to fetch"));

    let binary = dir.join("binary.bin");
    fs::write(&binary, [0xff_u8, 0xfe]).expect("binary file");
    let not_utf8 = curl_text(&format!("file://{}", binary.display()), None).expect_err("not utf-8");
    assert!(not_utf8.message.contains("not valid UTF-8"));

    let silent = CurlHttp {
        program: "false".to_string(),
    }
    .get_text("https://example.test", None)
    .expect_err("silent failure");
    assert!(silent.message.contains("curl exited with status"));

    let absent = CurlHttp {
        program: "traverse-no-such-curl".to_string(),
    }
    .get_text("https://example.test", None)
    .expect_err("no curl");
    assert!(absent.message.contains("failed to execute curl"));
}
