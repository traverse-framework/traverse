//! Durable write path for event publication.
//!
//! Governed by spec 067-durable-journal-retention-and-write-limits
//! (FR-003..FR-005): `publish()` waits for the durable journal write only up
//! to a configured timeout, a timed-out event is rejected — never silently
//! downgraded to in-memory-only delivery — and every timeout produces a
//! structured audit record (spec 036 NFR-006).

use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, PoisonError, RwLock};
use std::time::Duration;

use serde::Serialize;

use super::broker::BrokerClock;
use super::journal::{DurableEventJournal, JournalConfig, JournalError};
use super::types::{
    BrokerEvent, EventBroker, EventError, Subscription, SubscriptionPoll, TraverseEvent,
};

/// Prefix distinguishing durable-journal-backed subscription ids from the
/// inner broker's own `sub-*` ids, so [`DurableBroker::poll`] and
/// [`DurableBroker::cancel`] can route without a second lookup table.
const DURABLE_SUBSCRIPTION_PREFIX: &str = "durable-sub-";

/// Read-side of a durable journal: serves replay for cursors the live
/// broker's in-memory window no longer retains (spec 066 FR-005, FR-008).
/// [`RwLock<DurableEventJournal>`] is the production implementation, shared
/// with the write path via [`DurableBroker::open`]; tests inject a stub for
/// write-path-only scenarios that never exercise replay.
pub trait JournalSource: Send + Sync {
    /// Replay up to `max_events` events strictly after `cursor`.
    ///
    /// # Errors
    ///
    /// Returns [`JournalError`] when the cursor is malformed or expired, or
    /// the durable read fails.
    fn replay_from(
        &self,
        cursor: &str,
        max_events: usize,
    ) -> Result<Vec<(String, TraverseEvent)>, JournalError>;
}

impl JournalSource for RwLock<DurableEventJournal> {
    fn replay_from(
        &self,
        cursor: &str,
        max_events: usize,
    ) -> Result<Vec<(String, TraverseEvent)>, JournalError> {
        self.read()
            .unwrap_or_else(PoisonError::into_inner)
            .replay_from(cursor, max_events)
    }
}

/// Write-side adapter that locks a shared journal for each append, letting
/// [`DurableBroker::open`] give the writer thread and the read path the same
/// underlying storage so cursors stay consistent between them.
struct SharedJournalSink(Arc<RwLock<DurableEventJournal>>);

impl JournalSink for SharedJournalSink {
    fn append_event(&mut self, event: &TraverseEvent) -> Result<String, JournalError> {
        self.0
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .append(event)
    }

    fn append_revocation(&mut self, revoked_cursor: &str) -> Result<String, JournalError> {
        self.0
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .append_revocation(revoked_cursor)
    }
}

struct DurableSubscriptionState {
    event_type: String,
    subject_id: Option<String>,
    cursor: String,
}

#[derive(Default)]
struct DurableSubscriptions {
    next_id: u64,
    entries: HashMap<String, DurableSubscriptionState>,
}

fn map_journal_read_error(event_type: &str, error: JournalError) -> EventError {
    match error {
        JournalError::CursorExpired {
            oldest_available_cursor,
        } => EventError::CursorExpired {
            event_type: event_type.to_string(),
            oldest_available_cursor,
        },
        JournalError::InvalidCursor(msg) => EventError::InvalidCursor(msg),
        JournalError::Io(msg) | JournalError::InvalidConfig(msg) => EventError::JournalRead(msg),
        JournalError::Corrupt {
            path,
            line,
            message,
        } => EventError::JournalRead(format!(
            "corrupt journal record at {path}:{line}: {message}"
        )),
    }
}

/// Durable sink the writer thread appends through. [`DurableEventJournal`]
/// is the production implementation; tests inject slow or failing sinks to
/// drive the timeout and revocation paths deterministically.
pub trait JournalSink: Send {
    /// Durably append an event, returning its cursor.
    ///
    /// # Errors
    ///
    /// Returns [`JournalError`] when the durable write fails.
    fn append_event(&mut self, event: &TraverseEvent) -> Result<String, JournalError>;

    /// Durably suppress a previously written cursor from replay.
    ///
    /// # Errors
    ///
    /// Returns [`JournalError`] when the durable write fails.
    fn append_revocation(&mut self, revoked_cursor: &str) -> Result<String, JournalError>;
}

impl JournalSink for DurableEventJournal {
    fn append_event(&mut self, event: &TraverseEvent) -> Result<String, JournalError> {
        self.append(event)
    }

    fn append_revocation(&mut self, revoked_cursor: &str) -> Result<String, JournalError> {
        DurableEventJournal::append_revocation(self, revoked_cursor)
    }
}

/// Configuration for the durable write path (067 FR-003 default: 2 seconds).
#[derive(Debug, Clone, Copy)]
pub struct DurableBrokerConfig {
    /// Maximum time `publish()` waits for the durable write to complete.
    pub write_timeout: Duration,
}

impl Default for DurableBrokerConfig {
    fn default() -> Self {
        Self {
            write_timeout: Duration::from_secs(2),
        }
    }
}

/// Structured audit record for durable write-path failures (067 FR-005,
/// consistent with spec 036 NFR-006 observability).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JournalWriteAuditRecord {
    /// `journal_write_timeout` or `journal_revocation_failed`.
    pub kind: String,
    /// Id of the affected event.
    pub event_id: String,
    /// Type of the affected event.
    pub event_type: String,
    /// Human-readable failure detail.
    pub detail: String,
}

/// Receives structured audit records from the durable write path.
pub trait JournalWriteAuditSink: Send + Sync {
    /// Record one audit entry.
    fn record(&self, record: &JournalWriteAuditRecord);
}

enum AckState {
    Pending,
    Done(String),
    Failed(JournalError),
    Abandoned,
}

type Ack = Arc<(Mutex<AckState>, Condvar)>;

enum WriterJob {
    Write {
        event: TraverseEvent,
        ack: Ack,
    },
    Revoke {
        cursor: String,
        event_id: String,
        event_type: String,
        ack: Option<Ack>,
    },
}

/// [`EventBroker`] decorator that makes every published event durable before
/// it is delivered: the event is journaled with fsync-before-acknowledgement
/// (066 FR-006) and only then forwarded to the inner broker for live
/// delivery, adopting the journal-assigned cursor as the inner broker's own
/// cursor for that event (spec 066 FR-007) so the two stay numerically
/// consistent. A write that exceeds the configured timeout rejects the event
/// with `journal_write_timeout` (067 FR-003/FR-004); if the abandoned write
/// completes later, the writer durably revokes it so it can never surface
/// through replay.
///
/// `subscribe`/`poll` normally delegate to the inner broker's fast,
/// in-memory delivery. When a requested cursor is older than the inner
/// broker's retention window, [`DurableBroker`] falls back to the durable
/// journal (spec 066 FR-005, FR-008): if the journal still retains the
/// cursor, a durable-mode subscription is created that reads through
/// [`JournalSource::replay_from`] for the rest of its lifetime (applying the
/// identical `subject_id` filter as live delivery, FR-003); if the journal
/// has also reclaimed that history, `cursor_expired` is returned with the
/// journal's own oldest available cursor.
pub struct DurableBroker<B: EventBroker> {
    inner: B,
    jobs: mpsc::Sender<WriterJob>,
    write_timeout: Duration,
    audit: Arc<dyn JournalWriteAuditSink>,
    source: Arc<dyn JournalSource>,
    subscriptions: Mutex<DurableSubscriptions>,
}

impl<B: EventBroker> DurableBroker<B> {
    /// Wrap `inner` with a durable write path backed by `sink`, and a
    /// journal-backed replay fallback backed by `source`. Production callers
    /// should use [`DurableBroker::open`], which guarantees `sink` and
    /// `source` share the same underlying journal storage; this lower-level
    /// constructor exists so tests can inject independent write- and
    /// read-side doubles.
    pub fn new(
        inner: B,
        sink: impl JournalSink + 'static,
        source: Arc<dyn JournalSource>,
        config: DurableBrokerConfig,
        audit: Arc<dyn JournalWriteAuditSink>,
    ) -> Self {
        let (jobs, queue) = mpsc::channel();
        let writer_audit = Arc::clone(&audit);
        drop(std::thread::spawn(move || {
            run_writer(sink, &queue, writer_audit.as_ref());
        }));
        Self {
            inner,
            jobs,
            write_timeout: config.write_timeout,
            audit,
            source,
            subscriptions: Mutex::new(DurableSubscriptions::default()),
        }
    }

    /// Opens (or creates and recovers) a durable journal at `root` and wraps
    /// `inner` with a write path and journal-backed replay fallback sharing
    /// that same storage, so cursors stay consistent across live delivery
    /// and durable replay (spec 066 FR-007).
    ///
    /// # Errors
    ///
    /// Returns [`JournalError`] when the journal cannot be opened or
    /// recovered (066 FR-009).
    pub fn open(
        root: &Path,
        inner: B,
        journal_config: JournalConfig,
        broker_config: DurableBrokerConfig,
        audit: Arc<dyn JournalWriteAuditSink>,
        clock: Arc<dyn BrokerClock>,
    ) -> Result<Self, JournalError> {
        let journal = Arc::new(RwLock::new(DurableEventJournal::open(
            root,
            journal_config,
            clock,
        )?));
        // A freshly constructed `inner` has no memory of history that
        // predates this process (spec 066 FR-007 restart continuity): seed
        // it with the journal's own latest cursor so it correctly defers
        // cursors it cannot itself vouch for to durable replay, rather than
        // optimistically accepting them because it has simply never seen
        // this event type before.
        inner.seed_restart_floor(
            journal
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .latest_cursor(),
        );
        let source: Arc<dyn JournalSource> = Arc::clone(&journal) as Arc<dyn JournalSource>;
        Ok(Self::new(
            inner,
            SharedJournalSink(journal),
            source,
            broker_config,
            audit,
        ))
    }
}

impl<B: EventBroker> EventBroker for DurableBroker<B> {
    fn publish(&self, event: TraverseEvent) -> Result<(), EventError> {
        let ack: Ack = Arc::new((Mutex::new(AckState::Pending), Condvar::new()));
        self.jobs
            .send(WriterJob::Write {
                event: event.clone(),
                ack: Arc::clone(&ack),
            })
            .map_err(|_| EventError::JournalWrite("journal writer is unavailable".to_string()))?;

        let (lock, cvar) = &*ack;
        let guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
        let (mut state, _) = cvar
            .wait_timeout_while(guard, self.write_timeout, |state| {
                matches!(state, AckState::Pending)
            })
            .unwrap_or_else(PoisonError::into_inner);
        // Whatever happens next, the writer must treat this job as abandoned
        // unless we already saw its outcome.
        let outcome = std::mem::replace(&mut *state, AckState::Abandoned);
        drop(state);

        match outcome {
            AckState::Done(cursor) => {
                let event_id = event.id.clone();
                let event_type = event.event_type.clone();
                match self.inner.publish_with_cursor(event, &cursor) {
                    Ok(()) => Ok(()),
                    Err(error) => {
                        // Durably written but undeliverable: revoke so replay
                        // never surfaces an event that was never delivered live.
                        // Wait for the writer acknowledgement before returning;
                        // otherwise an immediate replay can race the revocation.
                        let revoke_ack: Ack =
                            Arc::new((Mutex::new(AckState::Pending), Condvar::new()));
                        let revoke = WriterJob::Revoke {
                            cursor,
                            event_id: event_id.clone(),
                            event_type: event_type.clone(),
                            ack: Some(Arc::clone(&revoke_ack)),
                        };
                        enqueue_revocation(
                            &self.jobs,
                            revoke,
                            self.audit.as_ref(),
                            &event_id,
                            &event_type,
                        )?;
                        let (lock, cvar) = &*revoke_ack;
                        let guard = lock.lock().unwrap_or_else(PoisonError::into_inner);
                        let (mut state, _) = cvar
                            .wait_timeout_while(guard, self.write_timeout, |state| {
                                matches!(state, AckState::Pending)
                            })
                            .unwrap_or_else(PoisonError::into_inner);
                        let revoke_outcome = std::mem::replace(&mut *state, AckState::Abandoned);
                        map_revocation_outcome(
                            revoke_outcome,
                            self.audit.as_ref(),
                            &event_id,
                            &event_type,
                            self.write_timeout,
                        )?;
                        Err(error)
                    }
                }
            }
            AckState::Failed(error) => Err(EventError::JournalWrite(error.to_string())),
            AckState::Pending | AckState::Abandoned => {
                let detail = format!(
                    "durable write exceeded {}ms; event rejected",
                    self.write_timeout.as_millis()
                );
                self.audit.record(&JournalWriteAuditRecord {
                    kind: "journal_write_timeout".to_string(),
                    event_id: event.id.clone(),
                    event_type: event.event_type.clone(),
                    detail: detail.clone(),
                });
                Err(EventError::JournalWriteTimeout(detail))
            }
        }
    }

    fn subscribe(&self, event_type: &str, from_cursor: &str) -> Result<Subscription, EventError> {
        self.subscribe_for_subject(event_type, from_cursor, None)
    }

    fn subscribe_for_subject(
        &self,
        event_type: &str,
        from_cursor: &str,
        subject_id: Option<&str>,
    ) -> Result<Subscription, EventError> {
        match self
            .inner
            .subscribe_for_subject(event_type, from_cursor, subject_id)
        {
            Ok(subscription) => Ok(subscription),
            Err(EventError::CursorExpired {
                event_type: expired_event_type,
                ..
            }) => {
                // The in-memory window no longer retains this cursor; check
                // whether the durable journal still does (066 FR-005,
                // FR-008). `max_events: 0` only validates the cursor.
                match self.source.replay_from(from_cursor, 0) {
                    Ok(_) => {
                        let cursor = normalize_cursor(from_cursor)?;
                        let mut subscriptions = self
                            .subscriptions
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner);
                        subscriptions.next_id = subscriptions.next_id.saturating_add(1);
                        let subscription_id =
                            format!("{DURABLE_SUBSCRIPTION_PREFIX}{}", subscriptions.next_id);
                        subscriptions.entries.insert(
                            subscription_id.clone(),
                            DurableSubscriptionState {
                                event_type: event_type.to_string(),
                                subject_id: subject_id.map(str::to_owned),
                                cursor: cursor.clone(),
                            },
                        );
                        Ok(Subscription {
                            subscription_id,
                            event_type: event_type.to_string(),
                            cursor,
                        })
                    }
                    Err(JournalError::CursorExpired {
                        oldest_available_cursor,
                    }) => Err(EventError::CursorExpired {
                        event_type: expired_event_type,
                        oldest_available_cursor,
                    }),
                    Err(other) => Err(map_journal_read_error(event_type, other)),
                }
            }
            Err(other) => Err(other),
        }
    }

    fn poll(
        &self,
        subscription_id: &str,
        max_events: usize,
    ) -> Result<SubscriptionPoll, EventError> {
        if !subscription_id.starts_with(DURABLE_SUBSCRIPTION_PREFIX) {
            return self.inner.poll(subscription_id, max_events);
        }

        let mut subscriptions = self
            .subscriptions
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let subscription = subscriptions
            .entries
            .get_mut(subscription_id)
            .ok_or_else(|| EventError::SubscriptionNotFound(subscription_id.to_string()))?;

        let mut delivered = Vec::new();
        let mut cursor = subscription.cursor.clone();
        if max_events > 0 {
            loop {
                let batch = self
                    .source
                    .replay_from(&cursor, max_events)
                    .map_err(|error| map_journal_read_error(&subscription.event_type, error))?;
                if batch.is_empty() {
                    break;
                }
                let batch_len = batch.len();
                for (record_cursor, event) in batch {
                    cursor = record_cursor;
                    if event.event_type != subscription.event_type {
                        continue;
                    }
                    if subscription
                        .subject_id
                        .as_deref()
                        .is_some_and(|subject| event.subject_id.as_deref() != Some(subject))
                    {
                        continue;
                    }
                    delivered.push(BrokerEvent {
                        cursor: cursor.clone(),
                        event,
                    });
                    if delivered.len() >= max_events {
                        break;
                    }
                }
                if delivered.len() >= max_events || batch_len < max_events {
                    break;
                }
            }
        }
        subscription.cursor.clone_from(&cursor);

        Ok(SubscriptionPoll {
            subscription_id: subscription_id.to_string(),
            event_type: subscription.event_type.clone(),
            cursor,
            events: delivered,
        })
    }

    fn cancel(&self, subscription_id: &str) -> Result<(), EventError> {
        if subscription_id.starts_with(DURABLE_SUBSCRIPTION_PREFIX) {
            let mut subscriptions = self
                .subscriptions
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            return if subscriptions.entries.remove(subscription_id).is_some() {
                Ok(())
            } else {
                Err(EventError::SubscriptionNotFound(
                    subscription_id.to_string(),
                ))
            };
        }
        self.inner.cancel(subscription_id)
    }
}

fn revocation_writer_unavailable(
    audit: &dyn JournalWriteAuditSink,
    event_id: &str,
    event_type: &str,
) -> EventError {
    audit.record(&JournalWriteAuditRecord {
        kind: "journal_revocation_failed".to_string(),
        event_id: event_id.to_string(),
        event_type: event_type.to_string(),
        detail: "journal writer is unavailable".to_string(),
    });
    EventError::JournalWrite("journal revocation failed: journal writer is unavailable".to_string())
}

fn enqueue_revocation(
    jobs: &mpsc::Sender<WriterJob>,
    revoke: WriterJob,
    audit: &dyn JournalWriteAuditSink,
    event_id: &str,
    event_type: &str,
) -> Result<(), EventError> {
    jobs.send(revoke)
        .map_err(|_| revocation_writer_unavailable(audit, event_id, event_type))
}

fn map_revocation_outcome(
    outcome: AckState,
    audit: &dyn JournalWriteAuditSink,
    event_id: &str,
    event_type: &str,
    timeout: Duration,
) -> Result<(), EventError> {
    match outcome {
        AckState::Done(_) => Ok(()),
        AckState::Failed(error) => Err(EventError::JournalWrite(format!(
            "journal revocation failed: {error}"
        ))),
        AckState::Pending | AckState::Abandoned => {
            let detail = format!(
                "journal revocation exceeded {}ms; event remains rejected",
                timeout.as_millis()
            );
            audit.record(&JournalWriteAuditRecord {
                kind: "journal_revocation_failed".to_string(),
                event_id: event_id.to_string(),
                event_type: event_type.to_string(),
                detail: detail.clone(),
            });
            Err(EventError::JournalWriteTimeout(detail))
        }
    }
}

fn normalize_cursor(raw: &str) -> Result<String, EventError> {
    raw.parse::<u64>()
        .map(|parsed| parsed.to_string())
        .map_err(|_| EventError::InvalidCursor(raw.to_string()))
}

fn run_writer(
    mut sink: impl JournalSink,
    jobs: &mpsc::Receiver<WriterJob>,
    audit: &dyn JournalWriteAuditSink,
) {
    while let Ok(job) = jobs.recv() {
        match job {
            WriterJob::Write { event, ack } => {
                let result = sink.append_event(&event);
                let (lock, cvar) = &*ack;
                let mut state = lock.lock().unwrap_or_else(PoisonError::into_inner);
                if matches!(*state, AckState::Abandoned) {
                    drop(state);
                    // The publisher already rejected this event; if the write
                    // completed anyway, durably suppress it (067 FR-004).
                    if let Ok(cursor) = result {
                        let _ = revoke(&mut sink, audit, &cursor, &event.id, &event.event_type);
                    }
                } else {
                    *state = match result {
                        Ok(cursor) => AckState::Done(cursor),
                        Err(error) => AckState::Failed(error),
                    };
                    cvar.notify_one();
                }
            }
            WriterJob::Revoke {
                cursor,
                event_id,
                event_type,
                ack,
            } => {
                let result = revoke(&mut sink, audit, &cursor, &event_id, &event_type);
                let _ = ack.map(|ack| acknowledge_revocation(&ack, result));
            }
        }
    }
}

fn acknowledge_revocation(ack: &Ack, result: Result<(), JournalError>) {
    let (lock, cvar) = &**ack;
    let mut state = lock.lock().unwrap_or_else(PoisonError::into_inner);
    *state = match result {
        Ok(()) => AckState::Done(String::new()),
        Err(error) => AckState::Failed(error),
    };
    cvar.notify_one();
}

fn revoke(
    sink: &mut impl JournalSink,
    audit: &dyn JournalWriteAuditSink,
    cursor: &str,
    event_id: &str,
    event_type: &str,
) -> Result<(), JournalError> {
    sink.append_revocation(cursor)
        .map(|_| ())
        .inspect_err(|error| {
            audit.record(&JournalWriteAuditRecord {
                kind: "journal_revocation_failed".to_string(),
                event_id: event_id.to_string(),
                event_type: event_type.to_string(),
                detail: error.to_string(),
            });
        })
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use crate::events::broker::InProcessBroker;
    use crate::events::catalog::{EventCatalog, EventCatalogEntry};
    use crate::events::journal::JournalConfig;
    use crate::events::types::LifecycleStatus;
    use std::sync::Mutex as StdMutex;
    use std::sync::mpsc::{Receiver, Sender, channel};
    use uuid::Uuid;

    const EVENT_TYPE: &str = "dev.traverse.test.durable";

    #[derive(Default)]
    struct RecordingAudit {
        records: StdMutex<Vec<JournalWriteAuditRecord>>,
        notify: StdMutex<Option<Sender<JournalWriteAuditRecord>>>,
    }

    impl RecordingAudit {
        fn with_notify(sender: Sender<JournalWriteAuditRecord>) -> Arc<Self> {
            Arc::new(Self {
                records: StdMutex::new(Vec::new()),
                notify: StdMutex::new(Some(sender)),
            })
        }

        fn kinds(&self) -> Vec<String> {
            self.records
                .lock()
                .expect("audit lock must not poison")
                .iter()
                .map(|record| record.kind.clone())
                .collect()
        }
    }

    impl JournalWriteAuditSink for RecordingAudit {
        fn record(&self, record: &JournalWriteAuditRecord) {
            self.records
                .lock()
                .expect("audit lock must not poison")
                .push(record.clone());
            if let Some(sender) = &*self.notify.lock().expect("notify lock must not poison") {
                let _ = sender.send(record.clone());
            }
        }
    }

    /// Sink whose `append_event` blocks until the test releases it, then
    /// reports the outcome the test scripted; revocations are reported back
    /// over a channel so tests can rendezvous deterministically.
    struct ScriptedSink {
        gate: Receiver<Result<String, JournalError>>,
        revocations: Sender<String>,
        fail_revocation: bool,
    }

    impl JournalSink for ScriptedSink {
        fn append_event(&mut self, _event: &TraverseEvent) -> Result<String, JournalError> {
            self.gate
                .recv()
                .unwrap_or_else(|_| Err(JournalError::Io("gate closed".to_string())))
        }

        fn append_revocation(&mut self, revoked_cursor: &str) -> Result<String, JournalError> {
            let _ = self.revocations.send(revoked_cursor.to_string());
            if self.fail_revocation {
                return Err(JournalError::Io("revocation rejected".to_string()));
            }
            Ok("0".to_string())
        }
    }

    fn test_event() -> TraverseEvent {
        TraverseEvent {
            id: Uuid::new_v4().to_string(),
            source: "traverse-runtime/test.capability".to_string(),
            event_type: EVENT_TYPE.to_string(),
            datacontenttype: "application/json".to_string(),
            time: "2026-07-13T00:00:00Z".to_string(),
            data: serde_json::json!({ "ok": true }),
            owner: "test.capability".to_string(),
            version: "1.0.0".to_string(),
            lifecycle_status: LifecycleStatus::Active,
            subject_id: None,
            actor_id: None,
        }
    }

    fn active_catalog() -> Arc<EventCatalog> {
        let catalog = EventCatalog::new();
        catalog
            .register(EventCatalogEntry {
                event_type: EVENT_TYPE.to_string(),
                version: "1.0.0".to_string(),
                owner: "test.capability".to_string(),
                lifecycle_status: LifecycleStatus::Active,
                consumer_count: 0,
            })
            .expect("catalog entry must register");
        Arc::new(catalog)
    }

    fn inner_broker() -> InProcessBroker {
        InProcessBroker::new(active_catalog()).expect("broker must build")
    }

    fn test_root(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("traverse-durable-{name}-{}", Uuid::new_v4()))
    }

    /// Read-side stub for write-path-only tests that never exercise durable
    /// replay: every read returns empty results, so any accidental fallback
    /// attempt fails loudly with `CursorExpired` rather than silently
    /// succeeding.
    struct NullJournalSource;

    impl JournalSource for NullJournalSource {
        fn replay_from(
            &self,
            _cursor: &str,
            _max_events: usize,
        ) -> Result<Vec<(String, TraverseEvent)>, JournalError> {
            Err(JournalError::CursorExpired {
                oldest_available_cursor: "0".to_string(),
            })
        }
    }

    fn null_source() -> Arc<dyn JournalSource> {
        Arc::new(NullJournalSource)
    }

    fn open_shared_journal(root: &std::path::Path) -> Arc<RwLock<DurableEventJournal>> {
        Arc::new(RwLock::new(
            DurableEventJournal::open(
                root,
                JournalConfig::default(),
                Arc::new(crate::events::broker::SystemClock),
            )
            .expect("journal must open"),
        ))
    }

    #[test]
    fn journal_sink_impl_appends_and_revokes_through_the_real_journal() {
        let root = test_root("sink-impl");
        let mut journal = DurableEventJournal::open(
            &root,
            JournalConfig::default(),
            Arc::new(crate::events::broker::SystemClock),
        )
        .expect("journal must open");
        let cursor =
            JournalSink::append_event(&mut journal, &test_event()).expect("append must succeed");
        JournalSink::append_revocation(&mut journal, &cursor).expect("revocation must succeed");
        assert!(
            journal
                .replay_from("0", 10)
                .expect("replay must succeed")
                .is_empty(),
            "the revoked event must not replay"
        );
    }

    #[test]
    fn durable_publish_journals_then_delivers() {
        let root = test_root("happy");
        let audit = Arc::new(RecordingAudit::default());
        let broker = DurableBroker::open(
            &root,
            inner_broker(),
            JournalConfig::default(),
            DurableBrokerConfig::default(),
            audit.clone(),
            Arc::new(crate::events::broker::SystemClock),
        )
        .expect("durable broker must open");

        let subscription = broker
            .subscribe(EVENT_TYPE, "0")
            .expect("subscribe must succeed");
        broker.publish(test_event()).expect("publish must succeed");

        let poll = broker
            .poll(&subscription.subscription_id, 10)
            .expect("poll must succeed");
        assert_eq!(poll.events.len(), 1, "live delivery must still work");
        broker
            .cancel(&subscription.subscription_id)
            .expect("cancel must succeed");

        let reader = DurableEventJournal::open(
            &root,
            JournalConfig::default(),
            Arc::new(crate::events::broker::SystemClock),
        )
        .expect("journal must reopen");
        let replayed = reader.replay_from("0", 10).expect("replay must succeed");
        assert_eq!(replayed.len(), 1, "the event must be durable");
        assert!(audit.kinds().is_empty(), "no audit records on success");
    }

    #[test]
    fn durable_subject_subscription_delegates_to_inner_broker() {
        let root = test_root("subject-subscription");
        let broker = DurableBroker::open(
            &root,
            inner_broker(),
            JournalConfig::default(),
            DurableBrokerConfig::default(),
            Arc::new(RecordingAudit::default()),
            Arc::new(crate::events::broker::SystemClock),
        )
        .expect("durable broker must open");
        let mut event = test_event();
        event.subject_id = Some("subject-match".to_string());
        broker.publish(event).expect("publish must succeed");

        let subscription = broker
            .subscribe_for_subject(EVENT_TYPE, "0", Some("subject-match"))
            .expect("subject subscription must succeed");
        let poll = broker
            .poll(&subscription.subscription_id, 10)
            .expect("poll must succeed");
        assert_eq!(poll.events.len(), 1);
        assert_eq!(
            poll.events[0].event.subject_id.as_deref(),
            Some("subject-match")
        );
    }

    #[test]
    fn undeliverable_event_is_revoked_after_durable_write() {
        let (gate_tx, gate_rx) = channel();
        let (revoked_tx, revoked_rx) = channel();
        let sink = ScriptedSink {
            gate: gate_rx,
            revocations: revoked_tx,
            fail_revocation: false,
        };
        let audit = Arc::new(RecordingAudit::default());
        let broker = DurableBroker::new(
            inner_broker(),
            sink,
            null_source(),
            DurableBrokerConfig::default(),
            audit,
        );

        let mut unregistered = test_event();
        unregistered.event_type = "dev.traverse.test.unknown".to_string();
        gate_tx
            .send(Ok("41".to_string()))
            .expect("gate must accept");
        let err = broker
            .publish(unregistered)
            .expect_err("unregistered event type must be rejected by the inner broker");
        assert!(matches!(err, EventError::UnregisteredEventType(_)), "{err}");

        let revoked = revoked_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the durably written but undelivered event must be revoked");
        assert_eq!(revoked, "41");
    }

    #[test]
    fn slow_durable_write_times_out_rejects_and_revokes() {
        let (gate_tx, gate_rx) = channel();
        let (revoked_tx, revoked_rx) = channel();
        let (audit_tx, audit_rx) = channel();
        let sink = ScriptedSink {
            gate: gate_rx,
            revocations: revoked_tx,
            fail_revocation: false,
        };
        let audit = RecordingAudit::with_notify(audit_tx);
        let broker = DurableBroker::new(
            inner_broker(),
            sink,
            null_source(),
            DurableBrokerConfig {
                write_timeout: Duration::from_millis(50),
            },
            audit.clone(),
        );

        let subscription = broker
            .subscribe(EVENT_TYPE, "0")
            .expect("subscribe must succeed");
        let err = broker
            .publish(test_event())
            .expect_err("a stalled durable write must time out");
        assert!(matches!(err, EventError::JournalWriteTimeout(_)), "{err}");
        assert!(
            err.to_string().contains("journal_write_timeout"),
            "the error code must be distinct: {err}"
        );

        let timeout_audit = audit_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("timeout must produce a structured audit record");
        assert_eq!(timeout_audit.kind, "journal_write_timeout");
        assert_eq!(timeout_audit.event_type, EVENT_TYPE);

        let poll = broker
            .poll(&subscription.subscription_id, 10)
            .expect("poll must succeed");
        assert!(
            poll.events.is_empty(),
            "a rejected event must not be delivered live"
        );

        // Release the stalled write; the writer must observe the abandoned
        // ack and durably revoke the late record.
        gate_tx.send(Ok("7".to_string())).expect("gate must accept");
        let revoked = revoked_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the late write must be revoked");
        assert_eq!(revoked, "7");
    }

    #[test]
    fn failed_durable_write_rejects_the_event() {
        let (gate_tx, gate_rx) = channel();
        let (revoked_tx, _revoked_rx) = channel();
        let sink = ScriptedSink {
            gate: gate_rx,
            revocations: revoked_tx,
            fail_revocation: false,
        };
        let audit = Arc::new(RecordingAudit::default());
        let broker = DurableBroker::new(
            inner_broker(),
            sink,
            null_source(),
            DurableBrokerConfig::default(),
            audit,
        );

        gate_tx
            .send(Err(JournalError::Io("disk gone".to_string())))
            .expect("gate must accept");
        let err = broker
            .publish(test_event())
            .expect_err("a failed durable write must reject the event");
        assert!(matches!(err, EventError::JournalWrite(_)), "{err}");
        assert!(err.to_string().contains("disk gone"), "{err}");
    }

    #[test]
    fn revocation_failures_are_audited() {
        let (gate_tx, gate_rx) = channel();
        let (revoked_tx, revoked_rx) = channel();
        let (audit_tx, audit_rx) = channel();
        let sink = ScriptedSink {
            gate: gate_rx,
            revocations: revoked_tx,
            fail_revocation: true,
        };
        let audit = RecordingAudit::with_notify(audit_tx);
        let broker = DurableBroker::new(
            inner_broker(),
            sink,
            null_source(),
            DurableBrokerConfig {
                write_timeout: Duration::from_millis(50),
            },
            audit.clone(),
        );

        let err = broker
            .publish(test_event())
            .expect_err("a stalled durable write must time out");
        assert!(matches!(err, EventError::JournalWriteTimeout(_)), "{err}");
        let timeout_audit = audit_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("timeout audit must arrive");
        assert_eq!(timeout_audit.kind, "journal_write_timeout");

        gate_tx.send(Ok("9".to_string())).expect("gate must accept");
        let _ = revoked_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("revocation must be attempted");
        let failure_audit = audit_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("revocation failure must be audited");
        assert_eq!(failure_audit.kind, "journal_revocation_failed");
        assert!(failure_audit.detail.contains("revocation rejected"));
    }

    #[test]
    fn immediate_revocation_failure_returns_a_stable_error() {
        let (gate_tx, gate_rx) = channel();
        let (revoked_tx, _revoked_rx) = channel();
        let sink = ScriptedSink {
            gate: gate_rx,
            revocations: revoked_tx,
            fail_revocation: true,
        };
        let broker = DurableBroker::new(
            inner_broker(),
            sink,
            null_source(),
            DurableBrokerConfig::default(),
            Arc::new(RecordingAudit::default()),
        );

        let mut unregistered = test_event();
        unregistered.event_type = "dev.traverse.test.unknown".to_string();
        gate_tx
            .send(Ok("11".to_string()))
            .expect("gate must accept");
        let error = broker
            .publish(unregistered)
            .expect_err("delivery and revocation failure must be returned");
        assert!(matches!(error, EventError::JournalWrite(_)), "{error}");
        assert!(error.to_string().contains("revocation rejected"));
    }

    #[test]
    fn disconnected_revocation_writer_is_audited() {
        let (jobs, receiver) = channel();
        drop(receiver);
        let revoke = WriterJob::Revoke {
            cursor: "1".to_string(),
            event_id: "event-1".to_string(),
            event_type: EVENT_TYPE.to_string(),
            ack: Some(Arc::new((Mutex::new(AckState::Pending), Condvar::new()))),
        };
        let audit = RecordingAudit::default();
        let error = enqueue_revocation(&jobs, revoke, &audit, "event-1", EVENT_TYPE)
            .expect_err("disconnected writer must fail closed");
        assert!(matches!(error, EventError::JournalWrite(_)), "{error}");
    }

    #[test]
    fn pending_revocation_ack_reports_timeout_and_audits_failure() {
        let audit = RecordingAudit::default();
        let timeout = map_revocation_outcome(
            AckState::Pending,
            &audit,
            "event-1",
            EVENT_TYPE,
            Duration::from_millis(25),
        )
        .expect_err("pending acknowledgement must time out");
        assert!(matches!(timeout, EventError::JournalWriteTimeout(_)));
        assert_eq!(audit.kinds(), vec!["journal_revocation_failed"]);
    }

    // --- journal-backed replay fallback (spec 066 FR-005, FR-007, FR-008; final #659 slice) ---

    #[derive(Debug)]
    struct ManualClock(StdMutex<std::time::SystemTime>);

    impl ManualClock {
        fn new() -> Self {
            Self(StdMutex::new(std::time::SystemTime::now()))
        }

        fn advance(&self, by: Duration) {
            if let Ok(mut guard) = self.0.lock()
                && let Some(next) = guard.checked_add(by)
            {
                *guard = next;
            }
        }
    }

    impl crate::events::broker::BrokerClock for ManualClock {
        fn now(&self) -> std::time::SystemTime {
            self.0
                .lock()
                .ok()
                .map_or(std::time::SystemTime::UNIX_EPOCH, |guard| *guard)
        }
    }

    /// Deterministically simulates a journal that has also reclaimed the
    /// requested history, independent of real segment rollover/pruning
    /// mechanics (already covered by `journal.rs`'s own test suite).
    struct AlwaysExpiredSource;

    impl JournalSource for AlwaysExpiredSource {
        fn replay_from(
            &self,
            _cursor: &str,
            _max_events: usize,
        ) -> Result<Vec<(String, TraverseEvent)>, JournalError> {
            Err(JournalError::CursorExpired {
                oldest_available_cursor: "5".to_string(),
            })
        }
    }

    fn short_retention_inner(
        clock: Arc<dyn crate::events::broker::BrokerClock>,
    ) -> crate::events::broker::InProcessBroker {
        crate::events::broker::InProcessBroker::with_clock(
            active_catalog(),
            crate::events::broker::BrokerConfig {
                retention_window: Duration::from_millis(10),
                max_queue_len: 16,
            },
            clock,
        )
        .expect("short-retention broker must build")
    }

    #[test]
    fn poll_falls_back_to_the_durable_journal_once_the_in_memory_window_expires() {
        let root = test_root("fallback-live");
        let clock = Arc::new(ManualClock::new());
        let journal = open_shared_journal(&root);
        let audit = Arc::new(RecordingAudit::default());
        let broker = DurableBroker::new(
            short_retention_inner(clock.clone()),
            SharedJournalSink(Arc::clone(&journal)),
            Arc::clone(&journal) as Arc<dyn JournalSource>,
            DurableBrokerConfig::default(),
            audit,
        );

        // Publish two events: cursor "0" is a sentinel that never expires
        // (it means "start of retention"), so triggering real expiry
        // requires resuming from a genuine prior cursor ("1") that is now
        // older than the latest retained cursor ("2").
        broker
            .publish(test_event())
            .expect("first publish must succeed");
        broker
            .publish(test_event())
            .expect("second publish must succeed");

        // Age the in-memory buffer out; `subscribe` must transparently fall
        // back to the durable journal within this single call rather than
        // surfacing the inner broker's `CursorExpired` to the caller.
        clock.advance(Duration::from_secs(1));
        let subscription = broker
            .subscribe(EVENT_TYPE, "1")
            .expect("subscribing on an expired cursor must fall back to the durable journal");
        assert!(
            subscription
                .subscription_id
                .starts_with(DURABLE_SUBSCRIPTION_PREFIX),
            "a durable fallback subscription must use the durable-mode id prefix"
        );

        let poll = broker
            .poll(&subscription.subscription_id, 10)
            .expect("durable poll must succeed");
        assert_eq!(
            poll.events.len(),
            1,
            "only the event strictly after cursor \"1\" must replay"
        );

        // Publish a second event *after* the durable subscription was
        // created and prove it still replays: durable-mode subscriptions
        // are not a one-time catchup, they keep working for new events too.
        broker
            .publish(test_event())
            .expect("second publish must succeed");
        let poll_again = broker
            .poll(&subscription.subscription_id, 10)
            .expect("second durable poll must succeed");
        assert_eq!(poll_again.events.len(), 1, "the new event must also replay");

        broker
            .cancel(&subscription.subscription_id)
            .expect("cancelling a durable-mode subscription must succeed");
        let after_cancel = broker.poll(&subscription.subscription_id, 10);
        assert!(matches!(
            after_cancel,
            Err(EventError::SubscriptionNotFound(_))
        ));
    }

    #[test]
    fn durable_fallback_applies_the_identical_subject_filter_as_live_delivery() {
        let root = test_root("fallback-subject");
        let clock = Arc::new(ManualClock::new());
        let journal = open_shared_journal(&root);
        let broker = DurableBroker::new(
            short_retention_inner(clock.clone()),
            SharedJournalSink(Arc::clone(&journal)),
            Arc::clone(&journal) as Arc<dyn JournalSource>,
            DurableBrokerConfig::default(),
            Arc::new(RecordingAudit::default()),
        );

        // A leading sentinel-only publish so the real assertions below can
        // resume from a genuine prior cursor ("1"), not the "0" sentinel
        // that never expires.
        broker
            .publish(test_event())
            .expect("leading publish must succeed");
        let mut matching = test_event();
        matching.subject_id = Some("subject-match".to_string());
        broker.publish(matching).expect("publish must succeed");
        let mut other = test_event();
        other.subject_id = Some("subject-other".to_string());
        broker.publish(other).expect("publish must succeed");

        clock.advance(Duration::from_secs(1));
        let subscription = broker
            .subscribe_for_subject(EVENT_TYPE, "1", Some("subject-match"))
            .expect("subject subscription must fall back to the durable journal");
        assert!(
            subscription
                .subscription_id
                .starts_with(DURABLE_SUBSCRIPTION_PREFIX)
        );

        let poll = broker
            .poll(&subscription.subscription_id, 10)
            .expect("durable poll must succeed");
        assert_eq!(
            poll.events.len(),
            1,
            "only the matching subject must replay"
        );
        assert_eq!(
            poll.events[0].event.subject_id.as_deref(),
            Some("subject-match")
        );
    }

    #[test]
    fn durable_fallback_surfaces_cursor_expired_when_the_journal_has_also_reclaimed_history() {
        let clock = Arc::new(ManualClock::new());
        let (gate_tx, gate_rx) = channel();
        let (revoked_tx, _revoked_rx) = channel();
        let sink = ScriptedSink {
            gate: gate_rx,
            revocations: revoked_tx,
            fail_revocation: false,
        };
        let broker = DurableBroker::new(
            short_retention_inner(clock.clone()),
            sink,
            Arc::new(AlwaysExpiredSource) as Arc<dyn JournalSource>,
            DurableBrokerConfig::default(),
            Arc::new(RecordingAudit::default()),
        );

        gate_tx.send(Ok("1".to_string())).expect("gate must accept");
        broker
            .publish(test_event())
            .expect("first publish must succeed");
        gate_tx.send(Ok("2".to_string())).expect("gate must accept");
        broker
            .publish(test_event())
            .expect("second publish must succeed");

        clock.advance(Duration::from_secs(1));
        let err = broker
            .subscribe(EVENT_TYPE, "1")
            .expect_err("both the in-memory and durable layers have expired this cursor");
        assert_eq!(
            err,
            EventError::CursorExpired {
                event_type: EVENT_TYPE.to_string(),
                oldest_available_cursor: "5".to_string(),
            },
            "the durable journal's own oldest-available cursor must be surfaced"
        );
    }

    #[test]
    fn durable_fallback_propagates_non_cursor_journal_read_errors() {
        struct FailingSource;
        impl JournalSource for FailingSource {
            fn replay_from(
                &self,
                _cursor: &str,
                _max_events: usize,
            ) -> Result<Vec<(String, TraverseEvent)>, JournalError> {
                Err(JournalError::Io("disk unavailable".to_string()))
            }
        }

        let clock = Arc::new(ManualClock::new());
        let (gate_tx, gate_rx) = channel();
        let (revoked_tx, _revoked_rx) = channel();
        let sink = ScriptedSink {
            gate: gate_rx,
            revocations: revoked_tx,
            fail_revocation: false,
        };
        let broker = DurableBroker::new(
            short_retention_inner(clock.clone()),
            sink,
            Arc::new(FailingSource) as Arc<dyn JournalSource>,
            DurableBrokerConfig::default(),
            Arc::new(RecordingAudit::default()),
        );

        gate_tx.send(Ok("1".to_string())).expect("gate must accept");
        broker
            .publish(test_event())
            .expect("first publish must succeed");
        gate_tx.send(Ok("2".to_string())).expect("gate must accept");
        broker
            .publish(test_event())
            .expect("second publish must succeed");

        clock.advance(Duration::from_secs(1));
        let err = broker
            .subscribe(EVENT_TYPE, "1")
            .expect_err("a durable read failure must not be silently swallowed");
        assert!(matches!(err, EventError::JournalRead(_)), "{err}");
    }

    #[test]
    fn poll_on_an_unknown_durable_subscription_id_returns_subscription_not_found() {
        let root = test_root("fallback-unknown-poll");
        let journal = open_shared_journal(&root);
        let broker = DurableBroker::new(
            inner_broker(),
            SharedJournalSink(Arc::clone(&journal)),
            Arc::clone(&journal) as Arc<dyn JournalSource>,
            DurableBrokerConfig::default(),
            Arc::new(RecordingAudit::default()),
        );

        let err = broker
            .poll(&format!("{DURABLE_SUBSCRIPTION_PREFIX}999"), 10)
            .expect_err("unknown durable subscription id must be rejected");
        assert!(matches!(err, EventError::SubscriptionNotFound(_)));

        let cancel_err = broker
            .cancel(&format!("{DURABLE_SUBSCRIPTION_PREFIX}999"))
            .expect_err("cancelling an unknown durable subscription id must be rejected");
        assert!(matches!(cancel_err, EventError::SubscriptionNotFound(_)));
    }

    #[test]
    fn poll_with_zero_max_events_on_a_durable_subscription_returns_no_events() {
        let root = test_root("fallback-zero-poll");
        let clock = Arc::new(ManualClock::new());
        let journal = open_shared_journal(&root);
        let broker = DurableBroker::new(
            short_retention_inner(clock.clone()),
            SharedJournalSink(Arc::clone(&journal)),
            Arc::clone(&journal) as Arc<dyn JournalSource>,
            DurableBrokerConfig::default(),
            Arc::new(RecordingAudit::default()),
        );

        broker
            .publish(test_event())
            .expect("first publish must succeed");
        broker
            .publish(test_event())
            .expect("second publish must succeed");
        clock.advance(Duration::from_secs(1));
        let subscription = broker
            .subscribe(EVENT_TYPE, "1")
            .expect("subscribe must fall back to the durable journal");

        let poll = broker
            .poll(&subscription.subscription_id, 0)
            .expect("poll with max_events=0 must succeed");
        assert!(poll.events.is_empty());
        assert_eq!(poll.cursor, "1", "an unread cursor must not advance");
    }

    #[test]
    fn cursor_survives_a_full_broker_restart_at_the_same_journal_root() {
        let root = test_root("restart-continuity");
        let cursor_before_restart = {
            let broker = DurableBroker::open(
                &root,
                inner_broker(),
                JournalConfig::default(),
                DurableBrokerConfig::default(),
                Arc::new(RecordingAudit::default()),
                Arc::new(crate::events::broker::SystemClock),
            )
            .expect("durable broker must open");

            let subscription = broker
                .subscribe(EVENT_TYPE, "0")
                .expect("subscribe must succeed");
            broker
                .publish(test_event())
                .expect("first publish must succeed");
            let poll = broker
                .poll(&subscription.subscription_id, 10)
                .expect("poll must succeed");
            assert_eq!(poll.events.len(), 1);
            // A second event that the subscriber never got around to
            // consuming before the (simulated) restart, so resuming its
            // cursor genuinely has a gap only the durable journal can fill.
            broker
                .publish(test_event())
                .expect("second publish must succeed");
            poll.cursor
            // `broker` and its in-process buffers are dropped here,
            // simulating a full process restart: nothing in memory
            // survives, only what was fsynced to `root`.
        };

        // Reopen at the same root with an entirely fresh in-memory broker —
        // the durable journal is the only thing that persisted.
        let restarted = DurableBroker::open(
            &root,
            inner_broker(),
            JournalConfig::default(),
            DurableBrokerConfig::default(),
            Arc::new(RecordingAudit::default()),
            Arc::new(crate::events::broker::SystemClock),
        )
        .expect("durable broker must reopen at the same root");

        // The fresh in-memory broker has never seen this event type, so
        // without a seeded restart floor it could not tell this cursor is
        // behind the durable tip; resuming it must fall back to the
        // durable journal and pick up the event missed before restart.
        let subscription = restarted
            .subscribe(EVENT_TYPE, &cursor_before_restart)
            .expect("resuming the pre-restart cursor must succeed after restart");
        assert!(
            subscription
                .subscription_id
                .starts_with(DURABLE_SUBSCRIPTION_PREFIX)
        );

        let poll = restarted
            .poll(&subscription.subscription_id, 10)
            .expect("poll after restart must succeed");
        assert_eq!(
            poll.events.len(),
            1,
            "the event published before restart but never consumed must replay"
        );

        restarted
            .publish(test_event())
            .expect("publish after restart must succeed");
        let poll = restarted
            .poll(&subscription.subscription_id, 10)
            .expect("second poll after restart must succeed");
        assert_eq!(
            poll.events.len(),
            1,
            "the event published after restart must replay from the resumed cursor"
        );
    }

    #[test]
    fn open_surfaces_an_invalid_journal_config() {
        let root = test_root("open-invalid-config");
        let err = DurableBroker::open(
            &root,
            inner_broker(),
            JournalConfig {
                max_segment_bytes: 0,
                ..JournalConfig::default()
            },
            DurableBrokerConfig::default(),
            Arc::new(RecordingAudit::default()),
            Arc::new(crate::events::broker::SystemClock),
        )
        .err()
        .expect("an invalid journal config must not open");
        assert!(matches!(err, JournalError::InvalidConfig(_)), "{err}");
    }

    #[test]
    fn subscribe_propagates_non_cursor_errors_from_the_inner_broker_unchanged() {
        let root = test_root("subscribe-non-cursor-error");
        let broker = DurableBroker::open(
            &root,
            inner_broker(),
            JournalConfig::default(),
            DurableBrokerConfig::default(),
            Arc::new(RecordingAudit::default()),
            Arc::new(crate::events::broker::SystemClock),
        )
        .expect("durable broker must open");

        let err = broker
            .subscribe("dev.traverse.test.unregistered", "0")
            .expect_err(
                "an unregistered event type must be rejected without consulting the journal",
            );
        assert!(matches!(err, EventError::UnregisteredEventType(_)), "{err}");
    }

    #[test]
    fn durable_poll_returns_no_events_when_already_caught_up_to_the_journal_tip() {
        let root = test_root("fallback-caught-up");
        let clock = Arc::new(ManualClock::new());
        let journal = open_shared_journal(&root);
        let broker = DurableBroker::new(
            short_retention_inner(clock.clone()),
            SharedJournalSink(Arc::clone(&journal)),
            Arc::clone(&journal) as Arc<dyn JournalSource>,
            DurableBrokerConfig::default(),
            Arc::new(RecordingAudit::default()),
        );

        broker
            .publish(test_event())
            .expect("first publish must succeed");
        broker
            .publish(test_event())
            .expect("second publish must succeed");
        clock.advance(Duration::from_secs(1));
        let subscription = broker
            .subscribe(EVENT_TYPE, "1")
            .expect("subscribe must fall back to the durable journal");

        // Immediately caught up: the durable journal has nothing after
        // cursor "2" yet, so `replay_from` returns an empty batch on the
        // very first loop iteration.
        let poll = broker
            .poll(&subscription.subscription_id, 10)
            .expect("first poll must succeed");
        assert_eq!(poll.events.len(), 1, "only the missed event must replay");
        let poll_again = broker
            .poll(&subscription.subscription_id, 10)
            .expect("second poll must succeed");
        assert!(
            poll_again.events.is_empty(),
            "nothing new since the last poll must yield an empty batch"
        );
    }

    #[test]
    fn durable_poll_respects_max_events_and_can_be_paged() {
        let root = test_root("fallback-paging");
        let clock = Arc::new(ManualClock::new());
        let journal = open_shared_journal(&root);
        let broker = DurableBroker::new(
            short_retention_inner(clock.clone()),
            SharedJournalSink(Arc::clone(&journal)),
            Arc::clone(&journal) as Arc<dyn JournalSource>,
            DurableBrokerConfig::default(),
            Arc::new(RecordingAudit::default()),
        );

        for _ in 0..4 {
            broker.publish(test_event()).expect("publish must succeed");
        }
        clock.advance(Duration::from_secs(1));
        let subscription = broker
            .subscribe(EVENT_TYPE, "1")
            .expect("subscribe must fall back to the durable journal");

        let first_page = broker
            .poll(&subscription.subscription_id, 2)
            .expect("first page must succeed");
        assert_eq!(
            first_page.events.len(),
            2,
            "the page must stop at max_events"
        );
        assert_eq!(first_page.cursor, "3");

        let second_page = broker
            .poll(&subscription.subscription_id, 2)
            .expect("second page must succeed");
        assert_eq!(second_page.events.len(), 1, "only one event remains");
        assert_eq!(second_page.cursor, "4");
    }

    #[test]
    fn durable_poll_skips_records_of_other_event_types() {
        const OTHER_EVENT_TYPE: &str = "dev.traverse.test.durable.other";
        let root = test_root("fallback-other-type");
        let clock = Arc::new(ManualClock::new());
        let journal = open_shared_journal(&root);
        let catalog = active_catalog();
        catalog
            .register(EventCatalogEntry {
                event_type: OTHER_EVENT_TYPE.to_string(),
                version: "1.0.0".to_string(),
                owner: "test.capability".to_string(),
                lifecycle_status: LifecycleStatus::Active,
                consumer_count: 0,
            })
            .expect("second catalog entry must register");
        let inner = crate::events::broker::InProcessBroker::with_clock(
            catalog,
            crate::events::broker::BrokerConfig {
                retention_window: Duration::from_millis(10),
                max_queue_len: 16,
            },
            clock.clone() as Arc<dyn crate::events::broker::BrokerClock>,
        )
        .expect("broker must build");
        let broker = DurableBroker::new(
            inner,
            SharedJournalSink(Arc::clone(&journal)),
            Arc::clone(&journal) as Arc<dyn JournalSource>,
            DurableBrokerConfig::default(),
            Arc::new(RecordingAudit::default()),
        );

        broker
            .publish(test_event())
            .expect("first publish must succeed");
        let mut other = test_event();
        other.event_type = OTHER_EVENT_TYPE.to_string();
        broker
            .publish(other)
            .expect("other-type publish must succeed");
        broker
            .publish(test_event())
            .expect("third publish must succeed");

        clock.advance(Duration::from_secs(1));
        let subscription = broker
            .subscribe(EVENT_TYPE, "1")
            .expect("subscribe must fall back to the durable journal");
        let poll = broker
            .poll(&subscription.subscription_id, 10)
            .expect("poll must succeed");
        assert_eq!(
            poll.events.len(),
            1,
            "the interleaved other-type record must be skipped"
        );
        assert_eq!(poll.events[0].event.event_type, EVENT_TYPE);
    }

    #[test]
    fn durable_poll_surfaces_cursor_expired_when_the_journal_prunes_mid_subscription() {
        struct ExpiresAfterFirstCallSource(std::sync::atomic::AtomicUsize);

        impl JournalSource for ExpiresAfterFirstCallSource {
            fn replay_from(
                &self,
                _cursor: &str,
                _max_events: usize,
            ) -> Result<Vec<(String, TraverseEvent)>, JournalError> {
                let call = self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if call == 0 {
                    Ok(Vec::new())
                } else {
                    Err(JournalError::CursorExpired {
                        oldest_available_cursor: "9".to_string(),
                    })
                }
            }
        }

        let clock = Arc::new(ManualClock::new());
        let (gate_tx, gate_rx) = channel();
        let (revoked_tx, _revoked_rx) = channel();
        let sink = ScriptedSink {
            gate: gate_rx,
            revocations: revoked_tx,
            fail_revocation: false,
        };
        let broker = DurableBroker::new(
            short_retention_inner(clock.clone()),
            sink,
            Arc::new(ExpiresAfterFirstCallSource(
                std::sync::atomic::AtomicUsize::new(0),
            )) as Arc<dyn JournalSource>,
            DurableBrokerConfig::default(),
            Arc::new(RecordingAudit::default()),
        );

        gate_tx.send(Ok("1".to_string())).expect("gate must accept");
        broker
            .publish(test_event())
            .expect("first publish must succeed");
        gate_tx.send(Ok("2".to_string())).expect("gate must accept");
        broker
            .publish(test_event())
            .expect("second publish must succeed");

        clock.advance(Duration::from_secs(1));
        let subscription = broker
            .subscribe(EVENT_TYPE, "1")
            .expect("subscribe must fall back to the durable journal (first source call)");

        let err = broker.poll(&subscription.subscription_id, 10).expect_err(
            "the journal pruning this cursor mid-subscription must surface cursor_expired",
        );
        assert_eq!(
            err,
            EventError::CursorExpired {
                event_type: EVENT_TYPE.to_string(),
                oldest_available_cursor: "9".to_string(),
            }
        );
    }

    #[test]
    fn durable_poll_propagates_a_corrupt_journal_read_as_journal_read() {
        struct CorruptSource;
        impl JournalSource for CorruptSource {
            fn replay_from(
                &self,
                _cursor: &str,
                _max_events: usize,
            ) -> Result<Vec<(String, TraverseEvent)>, JournalError> {
                Err(JournalError::Corrupt {
                    path: "segment-1.jsonl".to_string(),
                    line: 3,
                    message: "truncated record".to_string(),
                })
            }
        }

        let clock = Arc::new(ManualClock::new());
        let (gate_tx, gate_rx) = channel();
        let (revoked_tx, _revoked_rx) = channel();
        let sink = ScriptedSink {
            gate: gate_rx,
            revocations: revoked_tx,
            fail_revocation: false,
        };
        let broker = DurableBroker::new(
            short_retention_inner(clock.clone()),
            sink,
            Arc::new(CorruptSource) as Arc<dyn JournalSource>,
            DurableBrokerConfig::default(),
            Arc::new(RecordingAudit::default()),
        );

        gate_tx.send(Ok("1".to_string())).expect("gate must accept");
        broker
            .publish(test_event())
            .expect("first publish must succeed");
        gate_tx.send(Ok("2".to_string())).expect("gate must accept");
        broker
            .publish(test_event())
            .expect("second publish must succeed");

        clock.advance(Duration::from_secs(1));
        let err = broker
            .subscribe(EVENT_TYPE, "1")
            .expect_err("a corrupt journal read must not be silently swallowed");
        assert!(
            matches!(&err, EventError::JournalRead(msg) if msg.contains("truncated record")),
            "{err}"
        );
    }

    #[test]
    fn null_source_fails_loudly_instead_of_silently_serving_a_fallback() {
        let clock = Arc::new(ManualClock::new());
        let broker = DurableBroker::new(
            short_retention_inner(clock.clone()),
            SharedJournalSink(open_shared_journal(&test_root("null-source-fallback"))),
            null_source(),
            DurableBrokerConfig::default(),
            Arc::new(RecordingAudit::default()),
        );

        broker
            .publish(test_event())
            .expect("first publish must succeed");
        broker
            .publish(test_event())
            .expect("second publish must succeed");
        clock.advance(Duration::from_secs(1));

        let err = broker.subscribe(EVENT_TYPE, "1").expect_err(
            "a broker with no real durable source must not silently accept a stale cursor",
        );
        assert!(matches!(err, EventError::CursorExpired { .. }), "{err}");
    }

    #[test]
    fn map_journal_read_error_translates_invalid_cursor() {
        struct InvalidCursorSource;
        impl JournalSource for InvalidCursorSource {
            fn replay_from(
                &self,
                _cursor: &str,
                _max_events: usize,
            ) -> Result<Vec<(String, TraverseEvent)>, JournalError> {
                Err(JournalError::InvalidCursor("not a real cursor".to_string()))
            }
        }

        let clock = Arc::new(ManualClock::new());
        let (gate_tx, gate_rx) = channel();
        let (revoked_tx, _revoked_rx) = channel();
        let sink = ScriptedSink {
            gate: gate_rx,
            revocations: revoked_tx,
            fail_revocation: false,
        };
        let broker = DurableBroker::new(
            short_retention_inner(clock.clone()),
            sink,
            Arc::new(InvalidCursorSource) as Arc<dyn JournalSource>,
            DurableBrokerConfig::default(),
            Arc::new(RecordingAudit::default()),
        );

        gate_tx.send(Ok("1".to_string())).expect("gate must accept");
        broker
            .publish(test_event())
            .expect("first publish must succeed");
        gate_tx.send(Ok("2".to_string())).expect("gate must accept");
        broker
            .publish(test_event())
            .expect("second publish must succeed");

        clock.advance(Duration::from_secs(1));
        let err = broker
            .subscribe(EVENT_TYPE, "1")
            .expect_err("an invalid durable cursor must not be silently accepted");
        assert!(matches!(err, EventError::InvalidCursor(_)), "{err}");
    }

    #[test]
    fn shared_journal_sink_revokes_a_durably_written_but_undeliverable_event() {
        // Uses the real, production `SharedJournalSink` (via `DurableBroker::open`)
        // rather than a scripted test double, so the revocation actually
        // exercises the journal-backed write path (067 FR-004).
        let root = test_root("shared-sink-revocation");
        let broker = DurableBroker::open(
            &root,
            inner_broker(),
            JournalConfig::default(),
            DurableBrokerConfig::default(),
            Arc::new(RecordingAudit::default()),
            Arc::new(crate::events::broker::SystemClock),
        )
        .expect("durable broker must open");

        let mut unregistered = test_event();
        unregistered.event_type = "dev.traverse.test.unknown".to_string();
        let err = broker
            .publish(unregistered)
            .expect_err("an unregistered event type must be rejected by the inner broker");
        assert!(matches!(err, EventError::UnregisteredEventType(_)), "{err}");

        // The event was durably written before delivery failed, then
        // revoked; a fresh reader over the same root must never see it.
        let reader = DurableEventJournal::open(
            &root,
            JournalConfig::default(),
            Arc::new(crate::events::broker::SystemClock),
        )
        .expect("journal must reopen");
        let replayed = reader.replay_from("0", 10).expect("replay must succeed");
        assert!(
            replayed.is_empty(),
            "the revoked, undeliverable event must never replay"
        );
    }

    #[test]
    fn durable_poll_fetches_additional_batches_when_a_full_batch_matches_nothing() {
        const OTHER_EVENT_TYPE: &str = "dev.traverse.test.durable.other-batching";
        let root = test_root("fallback-multi-batch");
        let clock = Arc::new(ManualClock::new());
        let journal = open_shared_journal(&root);
        let catalog = active_catalog();
        catalog
            .register(EventCatalogEntry {
                event_type: OTHER_EVENT_TYPE.to_string(),
                version: "1.0.0".to_string(),
                owner: "test.capability".to_string(),
                lifecycle_status: LifecycleStatus::Active,
                consumer_count: 0,
            })
            .expect("second catalog entry must register");
        let inner = crate::events::broker::InProcessBroker::with_clock(
            catalog,
            crate::events::broker::BrokerConfig {
                retention_window: Duration::from_millis(10),
                max_queue_len: 16,
            },
            clock.clone() as Arc<dyn crate::events::broker::BrokerClock>,
        )
        .expect("broker must build");
        let broker = DurableBroker::new(
            inner,
            SharedJournalSink(Arc::clone(&journal)),
            Arc::clone(&journal) as Arc<dyn JournalSource>,
            DurableBrokerConfig::default(),
            Arc::new(RecordingAudit::default()),
        );

        broker
            .publish(test_event())
            .expect("leading publish must succeed");
        // Two consecutive other-type records: with max_events=1, each
        // `replay_from` batch is entirely filtered out, forcing the poll
        // loop to fetch another batch rather than stopping after one.
        let mut other_one = test_event();
        other_one.event_type = OTHER_EVENT_TYPE.to_string();
        broker
            .publish(other_one)
            .expect("first other-type publish must succeed");
        let mut other_two = test_event();
        other_two.event_type = OTHER_EVENT_TYPE.to_string();
        broker
            .publish(other_two)
            .expect("second other-type publish must succeed");
        broker
            .publish(test_event())
            .expect("trailing publish must succeed");

        clock.advance(Duration::from_secs(1));
        let subscription = broker
            .subscribe(EVENT_TYPE, "1")
            .expect("subscribe must fall back to the durable journal");

        let poll = broker
            .poll(&subscription.subscription_id, 1)
            .expect("poll must succeed across multiple internal batches");
        assert_eq!(poll.events.len(), 1);
        assert_eq!(poll.events[0].event.event_type, EVENT_TYPE);
        assert_eq!(
            poll.cursor, "4",
            "cursor must advance past the skipped batch"
        );
    }
}
