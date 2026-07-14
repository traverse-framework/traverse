//! Durable, append-only segmented event journal.
//!
//! Governed by spec 066-durable-identity-event-delivery (FR-005..FR-009) and
//! spec 067-durable-journal-retention-and-write-limits (FR-001, FR-002).
//!
//! The bounded publish write path that drives this journal lives in
//! [`super::durable`].

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::broker::BrokerClock;
use super::types::TraverseEvent;

const SEGMENT_PREFIX: &str = "segment-";
const SEGMENT_SUFFIX: &str = ".jsonl";

/// Journal runtime configuration (067 FR-001 defaults: 64 MB / 10 minutes).
#[derive(Debug, Clone)]
pub struct JournalConfig {
    /// Maximum bytes in a segment before it rolls over.
    pub max_segment_bytes: u64,
    /// Maximum age of a segment before it rolls over, in seconds.
    pub max_segment_age_secs: u64,
    /// Retention by age: events older than this may be reclaimed.
    pub retention_max_age_secs: Option<u64>,
    /// Retention by size: total journal bytes above this may be reclaimed.
    pub retention_max_total_bytes: Option<u64>,
}

impl Default for JournalConfig {
    fn default() -> Self {
        Self {
            max_segment_bytes: 64 * 1024 * 1024,
            max_segment_age_secs: 600,
            retention_max_age_secs: None,
            retention_max_total_bytes: None,
        }
    }
}

/// Errors surfaced by the durable journal.
#[derive(Debug, PartialEq, Eq)]
pub enum JournalError {
    /// Filesystem operation failed.
    Io(String),
    /// A completed journal record is malformed (066 FR-009: fail loudly).
    Corrupt {
        path: String,
        line: usize,
        message: String,
    },
    /// Cursor string could not be parsed.
    InvalidCursor(String),
    /// The requested cursor points before the retained history (066 FR-008).
    CursorExpired { oldest_available_cursor: String },
    /// Journal was configured with invalid limits.
    InvalidConfig(String),
}

impl std::fmt::Display for JournalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(msg) => write!(f, "journal io failure: {msg}"),
            Self::Corrupt {
                path,
                line,
                message,
            } => write!(f, "journal corrupt at {path}:{line}: {message}"),
            Self::InvalidCursor(msg) => write!(f, "invalid journal cursor: {msg}"),
            Self::CursorExpired {
                oldest_available_cursor,
            } => write!(
                f,
                "journal cursor expired: oldest available cursor is {oldest_available_cursor}"
            ),
            Self::InvalidConfig(msg) => write!(f, "invalid journal config: {msg}"),
        }
    }
}

impl std::error::Error for JournalError {}

/// One durable record: an acknowledged event with its journal sequence, or a
/// revocation suppressing a previously written sequence from replay
/// (067 FR-004: a rejected event must not be delivered through any path).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct JournalRecordV1 {
    seq: u64,
    written_at_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    event: Option<TraverseEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    revokes: Option<u64>,
}

/// Metadata for one on-disk segment, derived entirely from its contents so
/// cursors stay independent of segment layout (066 FR-007).
#[derive(Debug, Clone)]
struct SegmentMeta {
    path: PathBuf,
    first_seq: u64,
    last_seq: u64,
    created_at_secs: u64,
    last_written_at_secs: u64,
    bytes: u64,
}

/// Append-only segmented journal with fsync-before-acknowledgement.
pub struct DurableEventJournal {
    root: PathBuf,
    config: JournalConfig,
    clock: Arc<dyn BrokerClock>,
    sealed: Vec<SegmentMeta>,
    active: Option<(SegmentMeta, fs::File)>,
    next_seq: u64,
}

impl DurableEventJournal {
    /// Open (or create) the journal under `root`, recovering existing
    /// segments. Recovery ignores only an incomplete final record of the
    /// newest segment and fails loudly on any malformed completed record
    /// (066 FR-009).
    ///
    /// # Errors
    ///
    /// Returns [`JournalError::InvalidConfig`] for zero limits,
    /// [`JournalError::Io`] on filesystem failures, and
    /// [`JournalError::Corrupt`] when a completed record is malformed.
    pub fn open(
        root: &Path,
        config: JournalConfig,
        clock: Arc<dyn BrokerClock>,
    ) -> Result<Self, JournalError> {
        validate_config(&config)?;
        fs::create_dir_all(root).map_err(|e| io_err("create journal root", &e))?;

        let mut segment_paths = Vec::new();
        let entries = fs::read_dir(root).map_err(|e| io_err("list journal segments", &e))?;
        for entry in entries {
            let entry = entry.map_err(|e| io_err("read journal segment entry", &e))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(SEGMENT_PREFIX) && name.ends_with(SEGMENT_SUFFIX) {
                segment_paths.push(entry.path());
            }
        }
        segment_paths.sort();

        let mut sealed = Vec::new();
        let mut next_seq = 1_u64;
        let last_index = segment_paths.len().saturating_sub(1);
        for (index, path) in segment_paths.iter().enumerate() {
            let allow_torn_tail = index == last_index;
            let records = read_segment_records(path, allow_torn_tail)?;
            let Some((first, last)) = records.first().zip(records.last()) else {
                // Nothing in this segment was ever acknowledged (fsync happens
                // before ack), so dropping the file loses no durable data.
                fs::remove_file(path).map_err(|e| io_err("remove empty journal segment", &e))?;
                continue;
            };
            if first.seq < next_seq {
                return Err(JournalError::Corrupt {
                    path: path.display().to_string(),
                    line: 1,
                    message: format!(
                        "sequence {} is not greater than prior segment sequence {}",
                        first.seq,
                        next_seq - 1
                    ),
                });
            }
            let bytes = fs::metadata(path)
                .map_err(|e| io_err("stat journal segment", &e))?
                .len();
            sealed.push(SegmentMeta {
                path: path.clone(),
                first_seq: first.seq,
                last_seq: last.seq,
                created_at_secs: first.written_at_secs,
                last_written_at_secs: last.written_at_secs,
                bytes,
            });
            next_seq = last.seq + 1;
        }

        Ok(Self {
            root: root.to_path_buf(),
            config,
            clock,
            sealed,
            active: None,
            next_seq,
        })
    }

    /// Append an event, fsync it, and return its cursor (066 FR-006: the
    /// record is durable before this returns). Rolls the active segment over
    /// at the configured size or age bound, whichever occurs first
    /// (067 FR-001).
    ///
    /// # Errors
    ///
    /// Returns [`JournalError::Io`] when the durable write fails; the event
    /// is not acknowledged in that case.
    pub fn append(&mut self, event: &TraverseEvent) -> Result<String, JournalError> {
        self.append_line(Some(event), None)
    }

    /// Durably record that the event at `revoked_cursor` was rejected and
    /// must never be delivered through replay (067 FR-004). Used when a
    /// caller abandoned a write that later completed, or when a durably
    /// written event could not be delivered.
    ///
    /// # Errors
    ///
    /// Returns [`JournalError::InvalidCursor`] for unparseable cursors and
    /// [`JournalError::Io`] when the durable write fails.
    pub fn append_revocation(&mut self, revoked_cursor: &str) -> Result<String, JournalError> {
        let revoked = revoked_cursor.parse::<u64>().map_err(|e| {
            JournalError::InvalidCursor(format!("revoked cursor `{revoked_cursor}`: {e}"))
        })?;
        self.append_line(None, Some(revoked))
    }

    fn append_line(
        &mut self,
        event: Option<&TraverseEvent>,
        revokes: Option<u64>,
    ) -> Result<String, JournalError> {
        let now_secs = self.now_secs()?;

        let needs_rollover = self.active.as_ref().is_some_and(|(meta, _)| {
            meta.bytes >= self.config.max_segment_bytes
                || now_secs.saturating_sub(meta.created_at_secs) >= self.config.max_segment_age_secs
        });
        if needs_rollover && let Some((meta, file)) = self.active.take() {
            drop(file);
            self.sealed.push(meta);
        }

        let mut active = match self.active.take() {
            Some(active) => active,
            None => self.open_segment(now_secs)?,
        };
        let result = append_record(&mut active, self.next_seq, now_secs, event, revokes);
        self.active = Some(active);
        let cursor = result?;
        self.next_seq += 1;
        Ok(cursor)
    }

    fn open_segment(&self, now_secs: u64) -> Result<(SegmentMeta, fs::File), JournalError> {
        let path = self.root.join(format!(
            "{SEGMENT_PREFIX}{:020}{SEGMENT_SUFFIX}",
            self.next_seq
        ));
        let file = fs::OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(&path)
            .map_err(|e| io_err("create journal segment", &e))?;
        Ok((
            SegmentMeta {
                path,
                first_seq: self.next_seq,
                last_seq: self.next_seq,
                created_at_secs: now_secs,
                last_written_at_secs: now_secs,
                bytes: 0,
            },
            file,
        ))
    }

    /// Replay up to `max_events` events strictly after `cursor`.
    ///
    /// `"0"` replays from the start of retained history. Cursors are opaque
    /// monotonic sequence identifiers independent of segment layout
    /// (066 FR-007).
    ///
    /// # Errors
    ///
    /// Returns [`JournalError::InvalidCursor`] for unparseable cursors,
    /// [`JournalError::CursorExpired`] with the oldest available cursor when
    /// the requested history was reclaimed (066 FR-008), and
    /// [`JournalError::Io`] / [`JournalError::Corrupt`] on read failures.
    pub fn replay_from(
        &self,
        cursor: &str,
        max_events: usize,
    ) -> Result<Vec<(String, TraverseEvent)>, JournalError> {
        let after = cursor
            .parse::<u64>()
            .map_err(|e| JournalError::InvalidCursor(format!("cursor `{cursor}`: {e}")))?;

        let oldest = self.oldest_retained_seq();
        if let Some(oldest_seq) = oldest
            && after + 1 < oldest_seq
        {
            return Err(JournalError::CursorExpired {
                oldest_available_cursor: (oldest_seq - 1).to_string(),
            });
        }

        // Revocations always carry a later sequence than the record they
        // suppress, so the full scan must finish before results are final.
        let mut revoked = std::collections::HashSet::new();
        let mut collected: Vec<(u64, TraverseEvent)> = Vec::new();
        let last_index = self.segment_count().saturating_sub(1);
        for (index, meta) in self.segments().enumerate() {
            if meta.last_seq <= after {
                continue;
            }
            let allow_torn_tail = index == last_index;
            for record in read_segment_records(&meta.path, allow_torn_tail)? {
                if let Some(revoked_seq) = record.revokes {
                    let _ = revoked.insert(revoked_seq);
                } else if record.seq > after
                    && let Some(event) = record.event
                {
                    collected.push((record.seq, event));
                }
            }
        }
        collected.retain(|(seq, _)| !revoked.contains(seq));
        collected.truncate(max_events);
        Ok(collected
            .into_iter()
            .map(|(seq, event)| (seq.to_string(), event))
            .collect())
    }

    /// The cursor from which the oldest retained event replays; callers that
    /// receive [`JournalError::CursorExpired`] resume from here.
    #[must_use]
    pub fn oldest_available_cursor(&self) -> String {
        match self.oldest_retained_seq() {
            Some(seq) => (seq - 1).to_string(),
            None => (self.next_seq - 1).to_string(),
        }
    }

    /// Reclaim expired history by deleting whole sealed segments only — never
    /// rewriting or truncating in place (067 FR-002). A segment is deleted
    /// only once every event in it falls outside the retention window; the
    /// active segment is never deleted, bounding the overhang to one rollover
    /// period.
    ///
    /// # Errors
    ///
    /// Returns [`JournalError::Io`] when a reclaimable segment cannot be
    /// deleted.
    pub fn prune(&mut self) -> Result<Vec<PathBuf>, JournalError> {
        let now_secs = self.now_secs()?;
        let mut deleted = Vec::new();

        if let Some(max_age) = self.config.retention_max_age_secs {
            while let Some(meta) = self.sealed.first() {
                if now_secs.saturating_sub(meta.last_written_at_secs) <= max_age {
                    break;
                }
                let meta = self.sealed.remove(0);
                fs::remove_file(&meta.path)
                    .map_err(|e| io_err("remove expired journal segment", &e))?;
                deleted.push(meta.path);
            }
        }

        if let Some(max_total) = self.config.retention_max_total_bytes {
            let mut total: u64 = self.segments().map(|meta| meta.bytes).sum();
            while total > max_total && !self.sealed.is_empty() {
                let meta = self.sealed.remove(0);
                fs::remove_file(&meta.path)
                    .map_err(|e| io_err("remove oversized journal segment", &e))?;
                total -= meta.bytes;
                deleted.push(meta.path);
            }
        }

        Ok(deleted)
    }

    fn now_secs(&self) -> Result<u64, JournalError> {
        let now = self.clock.now();
        let elapsed = now
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| JournalError::Io(format!("system time before epoch: {e}")))?;
        Ok(elapsed.as_secs())
    }

    fn segments(&self) -> impl Iterator<Item = &SegmentMeta> {
        self.sealed
            .iter()
            .chain(self.active.iter().map(|(meta, _)| meta))
    }

    fn segment_count(&self) -> usize {
        self.sealed.len() + usize::from(self.active.is_some())
    }

    fn oldest_retained_seq(&self) -> Option<u64> {
        self.segments().map(|meta| meta.first_seq).next()
    }
}

fn validate_config(config: &JournalConfig) -> Result<(), JournalError> {
    if config.max_segment_bytes == 0 {
        return Err(JournalError::InvalidConfig(
            "max_segment_bytes must be at least 1".to_string(),
        ));
    }
    if config.max_segment_age_secs == 0 {
        return Err(JournalError::InvalidConfig(
            "max_segment_age_secs must be at least 1".to_string(),
        ));
    }
    if config.retention_max_age_secs == Some(0) {
        return Err(JournalError::InvalidConfig(
            "retention_max_age_secs must be at least 1 when set".to_string(),
        ));
    }
    if config.retention_max_total_bytes == Some(0) {
        return Err(JournalError::InvalidConfig(
            "retention_max_total_bytes must be at least 1 when set".to_string(),
        ));
    }
    Ok(())
}

/// Serialize, durably write, and acknowledge one record into the active
/// segment, returning its cursor.
fn append_record(
    active: &mut (SegmentMeta, fs::File),
    seq: u64,
    now_secs: u64,
    event: Option<&TraverseEvent>,
    revokes: Option<u64>,
) -> Result<String, JournalError> {
    let record = JournalRecordV1 {
        seq,
        written_at_secs: now_secs,
        event: event.cloned(),
        revokes,
    };
    let mut line = serde_json::to_vec(&record)
        .map_err(|e| JournalError::Io(format!("serialize journal record: {e}")))?;
    line.push(b'\n');

    let (meta, file) = active;
    write_durable(file, &line)?;
    meta.bytes += line.len() as u64;
    meta.last_seq = seq;
    meta.last_written_at_secs = now_secs;
    Ok(seq.to_string())
}

/// Write and fsync one record line; the record is only acknowledged after
/// both succeed (066 FR-006).
fn write_durable(file: &mut fs::File, line: &[u8]) -> Result<(), JournalError> {
    let write_then_sync = |file: &mut fs::File| -> std::io::Result<()> {
        file.write_all(line)?;
        file.sync_data()
    };
    write_then_sync(file).map_err(|e| io_err("append journal record", &e))
}

/// Parse every record in a segment. A trailing chunk without a newline
/// terminator is an incomplete final record: ignored when `allow_torn_tail`
/// (the newest segment interrupted mid-write), corrupt otherwise. Any
/// newline-terminated record that fails to parse is corrupt (066 FR-009).
fn read_segment_records(
    path: &Path,
    allow_torn_tail: bool,
) -> Result<Vec<JournalRecordV1>, JournalError> {
    let bytes = fs::read(path).map_err(|e| io_err("read journal segment", &e))?;
    let ends_with_newline = bytes.last() == Some(&b'\n');

    let mut records: Vec<JournalRecordV1> = Vec::new();
    let chunks: Vec<&[u8]> = bytes
        .split(|byte| *byte == b'\n')
        .filter(|chunk| !chunk.is_empty())
        .collect();
    for (index, chunk) in chunks.iter().enumerate() {
        let is_torn_tail = !ends_with_newline && index + 1 == chunks.len();
        match serde_json::from_slice::<JournalRecordV1>(chunk) {
            Ok(record) => {
                if is_torn_tail {
                    // A record is only acknowledged once its full line
                    // (including the terminator) is fsynced; a tail without a
                    // terminator was never acknowledged, even if it parses.
                    if allow_torn_tail {
                        break;
                    }
                    return Err(corrupt(path, index, "unterminated record"));
                }
                if let Some(previous) = records.last()
                    && record.seq <= previous.seq
                {
                    return Err(corrupt(
                        path,
                        index,
                        &format!(
                            "sequence {} is not greater than prior sequence {}",
                            record.seq, previous.seq
                        ),
                    ));
                }
                records.push(record);
            }
            Err(error) => {
                if is_torn_tail && allow_torn_tail {
                    break;
                }
                return Err(corrupt(path, index, &format!("malformed record: {error}")));
            }
        }
    }
    Ok(records)
}

fn corrupt(path: &Path, index: usize, message: &str) -> JournalError {
    JournalError::Corrupt {
        path: path.display().to_string(),
        line: index + 1,
        message: message.to_string(),
    }
}

fn io_err(action: &str, error: &std::io::Error) -> JournalError {
    JournalError::Io(format!("{action}: {error}"))
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use crate::events::types::LifecycleStatus;
    use std::sync::Mutex;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    use uuid::Uuid;

    struct TestClock {
        now: Mutex<SystemTime>,
    }

    impl TestClock {
        fn at_secs(secs: u64) -> Arc<Self> {
            Arc::new(Self {
                now: Mutex::new(UNIX_EPOCH + Duration::from_secs(secs)),
            })
        }

        fn before_epoch() -> Arc<Self> {
            Arc::new(Self {
                now: Mutex::new(UNIX_EPOCH - Duration::from_secs(1)),
            })
        }

        fn advance(&self, secs: u64) {
            let mut now = self.now.lock().expect("test clock lock must not poison");
            *now += Duration::from_secs(secs);
        }
    }

    impl BrokerClock for TestClock {
        fn now(&self) -> SystemTime {
            *self.now.lock().expect("test clock lock must not poison")
        }
    }

    fn test_root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("traverse-journal-{name}-{}", Uuid::new_v4()))
    }

    fn test_event(marker: &str) -> TraverseEvent {
        TraverseEvent {
            id: Uuid::new_v4().to_string(),
            source: "traverse-runtime/test.capability".to_string(),
            event_type: "dev.traverse.test.journaled".to_string(),
            datacontenttype: "application/json".to_string(),
            time: "2026-07-13T00:00:00Z".to_string(),
            data: serde_json::json!({ "marker": marker }),
            owner: "test.capability".to_string(),
            version: "1.0.0".to_string(),
            lifecycle_status: LifecycleStatus::Active,
            subject_id: None,
            actor_id: None,
        }
    }

    fn open_journal(
        root: &Path,
        config: JournalConfig,
        clock: Arc<TestClock>,
    ) -> DurableEventJournal {
        DurableEventJournal::open(root, config, clock).expect("journal must open")
    }

    #[test]
    fn config_limits_are_validated() {
        let clock = TestClock::at_secs(1_000);
        let cases = [
            JournalConfig {
                max_segment_bytes: 0,
                ..JournalConfig::default()
            },
            JournalConfig {
                max_segment_age_secs: 0,
                ..JournalConfig::default()
            },
            JournalConfig {
                retention_max_age_secs: Some(0),
                ..JournalConfig::default()
            },
            JournalConfig {
                retention_max_total_bytes: Some(0),
                ..JournalConfig::default()
            },
        ];
        for config in cases {
            let err = DurableEventJournal::open(&test_root("bad-config"), config, clock.clone())
                .map(|_| ())
                .expect_err("zero limits must be rejected");
            assert!(matches!(err, JournalError::InvalidConfig(_)), "{err}");
        }
    }

    #[test]
    fn append_and_replay_round_trip() {
        let root = test_root("round-trip");
        let clock = TestClock::at_secs(1_000);
        let mut journal = open_journal(&root, JournalConfig::default(), clock);

        assert_eq!(journal.oldest_available_cursor(), "0");
        assert!(
            journal
                .replay_from("0", 10)
                .expect("empty journal must replay nothing")
                .is_empty()
        );

        let first = journal
            .append(&test_event("a"))
            .expect("append must succeed");
        let second = journal
            .append(&test_event("b"))
            .expect("append must succeed");
        assert_eq!(first, "1");
        assert_eq!(second, "2");

        let all = journal.replay_from("0", 10).expect("replay must succeed");
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].0, "1");
        assert_eq!(all[0].1.data["marker"], "a");

        let tail = journal.replay_from("1", 10).expect("replay must succeed");
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0].0, "2");

        let head = journal.replay_from("2", 10).expect("replay must succeed");
        assert!(head.is_empty());

        let capped = journal.replay_from("0", 1).expect("replay must succeed");
        assert_eq!(capped.len(), 1, "max_events must bound the replay");
    }

    #[test]
    fn segments_roll_over_by_size_and_age() {
        let root = test_root("rollover");
        let clock = TestClock::at_secs(1_000);
        let config = JournalConfig {
            max_segment_bytes: 1,
            ..JournalConfig::default()
        };
        let mut journal = open_journal(&root, config, clock.clone());
        journal
            .append(&test_event("a"))
            .expect("append must succeed");
        journal
            .append(&test_event("b"))
            .expect("append must succeed");
        journal
            .append(&test_event("c"))
            .expect("append must succeed");
        assert_eq!(journal.sealed.len(), 2, "size bound must seal segments");

        let across = journal.replay_from("0", 10).expect("replay must succeed");
        assert_eq!(across.len(), 3, "replay must cross segment boundaries");
        let capped = journal.replay_from("0", 2).expect("replay must succeed");
        assert_eq!(capped.len(), 2, "max_events must stop mid-journal");

        let age_root = test_root("rollover-age");
        let mut aged = open_journal(&age_root, JournalConfig::default(), clock.clone());
        aged.append(&test_event("a")).expect("append must succeed");
        clock.advance(601);
        aged.append(&test_event("b")).expect("append must succeed");
        assert_eq!(aged.sealed.len(), 1, "age bound must seal segments");
    }

    #[test]
    fn reopen_recovers_segments_and_continues_sequences() {
        let root = test_root("reopen");
        let clock = TestClock::at_secs(1_000);
        let config = JournalConfig {
            max_segment_bytes: 1,
            ..JournalConfig::default()
        };
        {
            let mut journal = open_journal(&root, config.clone(), clock.clone());
            journal
                .append(&test_event("a"))
                .expect("append must succeed");
            journal
                .append(&test_event("b"))
                .expect("append must succeed");
        }

        let mut reopened = open_journal(&root, config, clock);
        assert_eq!(reopened.oldest_available_cursor(), "0");
        let cursor = reopened
            .append(&test_event("c"))
            .expect("append must succeed");
        assert_eq!(cursor, "3", "sequence must continue across restart");
        let all = reopened.replay_from("0", 10).expect("replay must succeed");
        assert_eq!(all.len(), 3);
        assert_eq!(all[2].1.data["marker"], "c");
    }

    #[test]
    fn recovery_tolerates_only_an_incomplete_final_record() {
        let root = test_root("torn-tail");
        let clock = TestClock::at_secs(1_000);
        {
            let mut journal = open_journal(&root, JournalConfig::default(), clock.clone());
            journal
                .append(&test_event("a"))
                .expect("append must succeed");
        }
        let segment = fs::read_dir(&root)
            .expect("root must list")
            .next()
            .expect("segment must exist")
            .expect("entry must read")
            .path();

        let original = fs::read(&segment).expect("segment must read");
        let mut torn = original.clone();
        torn.extend_from_slice(b"{\"seq\":2,\"truncated");
        fs::write(&segment, &torn).expect("torn tail must write");
        let journal = open_journal(&root, JournalConfig::default(), clock.clone());
        let recovered = journal.replay_from("0", 10).expect("replay must succeed");
        assert_eq!(recovered.len(), 1, "unparseable torn tail must be ignored");

        let newline = original
            .iter()
            .position(|byte| *byte == b'\n')
            .expect("newline");
        let mut unterminated = original.clone();
        unterminated.extend_from_slice(&original[..newline]);
        fs::write(&segment, &unterminated).expect("unterminated record must write");
        let journal = open_journal(&root, JournalConfig::default(), clock);
        let recovered = journal.replay_from("0", 10).expect("replay must succeed");
        assert_eq!(
            recovered.len(),
            1,
            "a parseable but unterminated tail was never acknowledged and must be ignored"
        );
    }

    #[test]
    fn recovery_fails_loudly_on_malformed_completed_records() {
        let clock = TestClock::at_secs(1_000);

        let corrupt_root = test_root("corrupt-interior");
        fs::create_dir_all(&corrupt_root).expect("root must be creatable");
        fs::write(
            corrupt_root.join("segment-00000000000000000001.jsonl"),
            b"not-json\n",
        )
        .expect("corrupt segment must write");
        let err = DurableEventJournal::open(&corrupt_root, JournalConfig::default(), clock.clone())
            .map(|_| ())
            .expect_err("malformed completed record must fail");
        assert!(matches!(err, JournalError::Corrupt { .. }), "{err}");

        let torn_old_root = test_root("torn-old-segment");
        {
            let config = JournalConfig {
                max_segment_bytes: 1,
                ..JournalConfig::default()
            };
            let mut journal = open_journal(&torn_old_root, config, clock.clone());
            journal
                .append(&test_event("a"))
                .expect("append must succeed");
            journal
                .append(&test_event("b"))
                .expect("append must succeed");
        }
        let oldest = fs::read_dir(&torn_old_root)
            .expect("root must list")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .min()
            .expect("oldest segment must exist");
        let mut torn = fs::read(&oldest).expect("segment must read");
        torn.extend_from_slice(b"{\"seq\":9,\"truncated");
        fs::write(&oldest, &torn).expect("torn tail must write");
        let err =
            DurableEventJournal::open(&torn_old_root, JournalConfig::default(), clock.clone())
                .map(|_| ())
                .expect_err("a torn tail in an older segment must fail");
        assert!(matches!(err, JournalError::Corrupt { .. }), "{err}");

        let original = fs::read(&oldest).expect("segment must read");
        let unterminated_end = original
            .iter()
            .position(|byte| *byte == b'\n')
            .expect("newline");
        fs::write(&oldest, &original[..unterminated_end])
            .expect("unterminated valid record must write");
        let err =
            DurableEventJournal::open(&torn_old_root, JournalConfig::default(), clock.clone())
                .map(|_| ())
                .expect_err("a parseable unterminated record in an older segment must fail");
        assert!(matches!(err, JournalError::Corrupt { .. }), "{err}");
    }

    #[test]
    fn recovery_rejects_non_monotonic_sequences() {
        let clock = TestClock::at_secs(1_000);
        let record = |seq: u64| {
            let mut line = serde_json::to_vec(&JournalRecordV1 {
                seq,
                written_at_secs: 1_000,
                event: Some(test_event("x")),
                revokes: None,
            })
            .expect("record must serialize");
            line.push(b'\n');
            line
        };

        let within_root = test_root("non-monotonic-within");
        fs::create_dir_all(&within_root).expect("root must be creatable");
        let mut lines = record(2);
        lines.extend_from_slice(&record(2));
        fs::write(
            within_root.join("segment-00000000000000000002.jsonl"),
            &lines,
        )
        .expect("segment must write");
        let err = DurableEventJournal::open(&within_root, JournalConfig::default(), clock.clone())
            .map(|_| ())
            .expect_err("non-monotonic records within a segment must fail");
        assert!(matches!(err, JournalError::Corrupt { .. }), "{err}");

        let across_root = test_root("non-monotonic-across");
        fs::create_dir_all(&across_root).expect("root must be creatable");
        fs::write(
            across_root.join("segment-00000000000000000001.jsonl"),
            record(5),
        )
        .expect("segment must write");
        fs::write(
            across_root.join("segment-00000000000000000002.jsonl"),
            record(3),
        )
        .expect("segment must write");
        let err = DurableEventJournal::open(&across_root, JournalConfig::default(), clock)
            .map(|_| ())
            .expect_err("non-monotonic records across segments must fail");
        assert!(matches!(err, JournalError::Corrupt { .. }), "{err}");
    }

    #[test]
    fn recovery_drops_segments_with_no_acknowledged_records() {
        let clock = TestClock::at_secs(1_000);
        let root = test_root("empty-segment");
        fs::create_dir_all(&root).expect("root must be creatable");
        let empty = root.join("segment-00000000000000000001.jsonl");
        fs::write(&empty, b"").expect("empty segment must write");
        fs::write(root.join("ignored.txt"), b"not a segment").expect("stray file must write");
        let journal = open_journal(&root, JournalConfig::default(), clock);
        assert!(!empty.exists(), "unacknowledged segment must be removed");
        assert_eq!(journal.oldest_available_cursor(), "0");
    }

    #[test]
    fn filesystem_failures_surface_as_io_errors() {
        let clock = TestClock::at_secs(1_000);

        let blocked_parent = test_root("blocked-parent");
        fs::create_dir_all(&blocked_parent).expect("parent must be creatable");
        fs::write(blocked_parent.join("root"), b"file").expect("squatting file must write");
        let err = DurableEventJournal::open(
            &blocked_parent.join("root"),
            JournalConfig::default(),
            clock.clone(),
        )
        .map(|_| ())
        .expect_err("root creation over a file must fail");
        assert!(matches!(err, JournalError::Io(_)), "{err}");

        let dir_segment_root = test_root("dir-segment");
        fs::create_dir_all(dir_segment_root.join("segment-00000000000000000001.jsonl"))
            .expect("directory squatting on a segment must be creatable");
        let err =
            DurableEventJournal::open(&dir_segment_root, JournalConfig::default(), clock.clone())
                .map(|_| ())
                .expect_err("reading a directory as a segment must fail");
        assert!(matches!(err, JournalError::Io(_)), "{err}");

        let squat_root = test_root("squat-next-segment");
        fs::create_dir_all(squat_root.join("segment-00000000000000000001.jsonl"))
            .expect("squatting directory must be creatable");
        let mut journal =
            open_journal(&test_root("fresh"), JournalConfig::default(), clock.clone());
        journal.root = squat_root;
        let err = journal
            .append(&test_event("a"))
            .expect_err("creating a segment over a directory must fail");
        assert!(matches!(err, JournalError::Io(_)), "{err}");

        let read_only_root = test_root("read-only-file");
        fs::create_dir_all(&read_only_root).expect("root must be creatable");
        let path = read_only_root.join("segment.jsonl");
        fs::write(&path, b"").expect("file must write");
        let mut file = fs::OpenOptions::new()
            .read(true)
            .open(&path)
            .expect("file must open read-only");
        let err = write_durable(&mut file, b"line\n")
            .expect_err("writing through a read-only handle must fail");
        assert!(matches!(err, JournalError::Io(_)), "{err}");
    }

    #[test]
    fn pre_epoch_clock_fails_closed() {
        let root = test_root("pre-epoch");
        let clock = TestClock::before_epoch();
        let mut journal = open_journal(&root, JournalConfig::default(), clock);
        let append_err = journal
            .append(&test_event("a"))
            .expect_err("append with a pre-epoch clock must fail");
        assert!(matches!(append_err, JournalError::Io(_)), "{append_err}");
        let prune_err = journal
            .prune()
            .expect_err("prune with a pre-epoch clock must fail");
        assert!(matches!(prune_err, JournalError::Io(_)), "{prune_err}");
    }

    #[test]
    fn cursors_are_validated_and_expire_after_pruning() {
        let root = test_root("cursor-expiry");
        let clock = TestClock::at_secs(1_000);
        let config = JournalConfig {
            max_segment_bytes: 1,
            retention_max_age_secs: Some(10),
            ..JournalConfig::default()
        };
        let mut journal = open_journal(&root, config, clock.clone());

        let invalid = journal
            .replay_from("not-a-cursor", 10)
            .expect_err("malformed cursor must be rejected");
        assert!(
            matches!(invalid, JournalError::InvalidCursor(_)),
            "{invalid}"
        );

        journal
            .append(&test_event("a"))
            .expect("append must succeed");
        journal
            .append(&test_event("b"))
            .expect("append must succeed");
        journal
            .append(&test_event("c"))
            .expect("append must succeed");

        clock.advance(100);
        let deleted = journal.prune().expect("prune must succeed");
        assert_eq!(deleted.len(), 2, "expired sealed segments must be deleted");
        for path in &deleted {
            assert!(!path.exists(), "pruned segment file must be removed");
        }

        let expired = journal
            .replay_from("0", 10)
            .expect_err("cursor before retained history must expire");
        assert_eq!(
            expired,
            JournalError::CursorExpired {
                oldest_available_cursor: "2".to_string(),
            }
        );
        assert_eq!(journal.oldest_available_cursor(), "2");

        let resumed = journal
            .replay_from("2", 10)
            .expect("oldest available cursor must replay");
        assert_eq!(resumed.len(), 1);
        assert_eq!(resumed[0].1.data["marker"], "c");
    }

    #[test]
    fn prune_reclaims_whole_segments_only_and_spares_the_active_one() {
        let root = test_root("prune-rules");
        let clock = TestClock::at_secs(1_000);
        let config = JournalConfig {
            max_segment_bytes: 1,
            retention_max_age_secs: Some(1_000_000),
            retention_max_total_bytes: Some(1),
            ..JournalConfig::default()
        };
        let mut journal = open_journal(&root, config, clock.clone());
        journal
            .append(&test_event("a"))
            .expect("append must succeed");
        journal
            .append(&test_event("b"))
            .expect("append must succeed");

        let deleted = journal.prune().expect("prune must succeed");
        assert_eq!(
            deleted.len(),
            1,
            "size retention must delete oldest sealed segments only"
        );
        assert!(
            journal.active.is_some(),
            "the active segment must never be pruned"
        );
        let survivors = journal.replay_from(&journal.oldest_available_cursor(), 10);
        assert_eq!(survivors.expect("replay must succeed").len(), 1);

        let unlimited_root = test_root("prune-unlimited");
        let mut unlimited = open_journal(&unlimited_root, JournalConfig::default(), clock.clone());
        unlimited
            .append(&test_event("a"))
            .expect("append must succeed");
        assert!(
            unlimited
                .prune()
                .expect("prune without retention must succeed")
                .is_empty(),
            "no retention configured means nothing is reclaimed"
        );

        let missing_root = test_root("prune-missing-file");
        let config = JournalConfig {
            max_segment_bytes: 1,
            retention_max_age_secs: Some(1),
            ..JournalConfig::default()
        };
        let mut missing = open_journal(&missing_root, config, clock.clone());
        missing
            .append(&test_event("a"))
            .expect("append must succeed");
        missing
            .append(&test_event("b"))
            .expect("append must succeed");
        let sealed_path = missing.sealed[0].path.clone();
        fs::remove_file(&sealed_path).expect("sealed segment must be removable");
        clock.advance(100);
        let err = missing
            .prune()
            .expect_err("pruning an already-missing segment must surface an io error");
        assert!(matches!(err, JournalError::Io(_)), "{err}");
    }

    #[test]
    fn prune_size_rule_surfaces_removal_failures() {
        let root = test_root("prune-size-missing");
        let clock = TestClock::at_secs(1_000);
        let config = JournalConfig {
            max_segment_bytes: 1,
            retention_max_total_bytes: Some(1),
            ..JournalConfig::default()
        };
        let mut journal = open_journal(&root, config, clock);
        journal
            .append(&test_event("a"))
            .expect("append must succeed");
        journal
            .append(&test_event("b"))
            .expect("append must succeed");
        let sealed_path = journal.sealed[0].path.clone();
        fs::remove_file(&sealed_path).expect("sealed segment must be removable");
        let err = journal
            .prune()
            .expect_err("size pruning an already-missing segment must surface an io error");
        assert!(matches!(err, JournalError::Io(_)), "{err}");
    }

    #[test]
    fn revocations_suppress_events_from_replay() {
        let root = test_root("revocation");
        let clock = TestClock::at_secs(1_000);
        let mut journal = open_journal(&root, JournalConfig::default(), clock.clone());
        journal
            .append(&test_event("a"))
            .expect("append must succeed");
        let second = journal
            .append(&test_event("b"))
            .expect("append must succeed");
        journal
            .append(&test_event("c"))
            .expect("append must succeed");

        journal
            .append_revocation(&second)
            .expect("revocation must be durable");

        let replayed = journal.replay_from("0", 10).expect("replay must succeed");
        let markers: Vec<_> = replayed
            .iter()
            .map(|(_, event)| event.data["marker"].clone())
            .collect();
        assert_eq!(
            markers,
            vec![serde_json::json!("a"), serde_json::json!("c")],
            "the revoked event must not be delivered and the revocation record itself must not appear"
        );

        let capped = journal.replay_from("0", 2).expect("replay must succeed");
        assert_eq!(
            capped.len(),
            2,
            "max_events must apply after revocation filtering"
        );

        let reopened = open_journal(&root, JournalConfig::default(), clock);
        let recovered = reopened.replay_from("0", 10).expect("replay must succeed");
        assert_eq!(
            recovered.len(),
            2,
            "revocations must keep suppressing events across restart"
        );

        let mut invalid = reopened;
        let err = invalid
            .append_revocation("not-a-cursor")
            .expect_err("unparseable revoked cursor must be rejected");
        assert!(matches!(err, JournalError::InvalidCursor(_)), "{err}");
    }

    #[test]
    fn errors_render_stable_messages() {
        let cases: Vec<(JournalError, &str)> = vec![
            (JournalError::Io("boom".to_string()), "journal io failure"),
            (
                JournalError::Corrupt {
                    path: "p".to_string(),
                    line: 3,
                    message: "bad".to_string(),
                },
                "journal corrupt at p:3",
            ),
            (
                JournalError::InvalidCursor("bad".to_string()),
                "invalid journal cursor",
            ),
            (
                JournalError::CursorExpired {
                    oldest_available_cursor: "7".to_string(),
                },
                "oldest available cursor is 7",
            ),
            (
                JournalError::InvalidConfig("bad".to_string()),
                "invalid journal config",
            ),
        ];
        for (error, expected) in cases {
            assert!(
                error.to_string().contains(expected),
                "{error} must mention {expected}"
            );
        }
    }
}
