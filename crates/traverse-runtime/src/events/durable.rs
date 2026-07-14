//! Durable write path for event publication.
//!
//! Governed by spec 067-durable-journal-retention-and-write-limits
//! (FR-003..FR-005): `publish()` waits for the durable journal write only up
//! to a configured timeout, a timed-out event is rejected — never silently
//! downgraded to in-memory-only delivery — and every timeout produces a
//! structured audit record (spec 036 NFR-006).

use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

use serde::Serialize;

use super::journal::{DurableEventJournal, JournalError};
use super::types::{EventBroker, EventError, Subscription, SubscriptionPoll, TraverseEvent};

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
    },
}

/// [`EventBroker`] decorator that makes every published event durable before
/// it is delivered: the event is journaled with fsync-before-acknowledgement
/// (066 FR-006) and only then forwarded to the inner broker for live
/// delivery. A write that exceeds the configured timeout rejects the event
/// with `journal_write_timeout` (067 FR-003/FR-004); if the abandoned write
/// completes later, the writer durably revokes it so it can never surface
/// through replay.
pub struct DurableBroker<B: EventBroker> {
    inner: B,
    jobs: mpsc::Sender<WriterJob>,
    write_timeout: Duration,
    audit: Arc<dyn JournalWriteAuditSink>,
}

impl<B: EventBroker> DurableBroker<B> {
    /// Wrap `inner` with a durable write path backed by `sink`.
    pub fn new(
        inner: B,
        sink: impl JournalSink + 'static,
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
        }
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
            AckState::Done(cursor) => match self.inner.publish(event) {
                Ok(()) => Ok(()),
                Err(error) => {
                    // Durably written but undeliverable: revoke so replay
                    // never surfaces an event that was never delivered live.
                    let revoke = WriterJob::Revoke {
                        cursor,
                        event_id: String::new(),
                        event_type: String::new(),
                    };
                    let _ = self.jobs.send(revoke);
                    Err(error)
                }
            },
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
        self.inner.subscribe(event_type, from_cursor)
    }

    fn subscribe_for_subject(
        &self,
        event_type: &str,
        from_cursor: &str,
        subject_id: Option<&str>,
    ) -> Result<Subscription, EventError> {
        self.inner
            .subscribe_for_subject(event_type, from_cursor, subject_id)
    }

    fn poll(
        &self,
        subscription_id: &str,
        max_events: usize,
    ) -> Result<SubscriptionPoll, EventError> {
        self.inner.poll(subscription_id, max_events)
    }

    fn cancel(&self, subscription_id: &str) -> Result<(), EventError> {
        self.inner.cancel(subscription_id)
    }
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
                        revoke(&mut sink, audit, &cursor, &event.id, &event.event_type);
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
            } => revoke(&mut sink, audit, &cursor, &event_id, &event_type),
        }
    }
}

fn revoke(
    sink: &mut impl JournalSink,
    audit: &dyn JournalWriteAuditSink,
    cursor: &str,
    event_id: &str,
    event_type: &str,
) {
    if let Err(error) = sink.append_revocation(cursor) {
        audit.record(&JournalWriteAuditRecord {
            kind: "journal_revocation_failed".to_string(),
            event_id: event_id.to_string(),
            event_type: event_type.to_string(),
            detail: error.to_string(),
        });
    }
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
        let journal = DurableEventJournal::open(
            &root,
            JournalConfig::default(),
            Arc::new(crate::events::broker::SystemClock),
        )
        .expect("journal must open");
        let audit = Arc::new(RecordingAudit::default());
        let broker = DurableBroker::new(
            inner_broker(),
            journal,
            DurableBrokerConfig::default(),
            audit.clone(),
        );

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
        let journal = DurableEventJournal::open(
            &root,
            JournalConfig::default(),
            Arc::new(crate::events::broker::SystemClock),
        )
        .expect("journal must open");
        let broker = DurableBroker::new(
            inner_broker(),
            journal,
            DurableBrokerConfig::default(),
            Arc::new(RecordingAudit::default()),
        );
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
        let broker =
            DurableBroker::new(inner_broker(), sink, DurableBrokerConfig::default(), audit);

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
        let broker =
            DurableBroker::new(inner_broker(), sink, DurableBrokerConfig::default(), audit);

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
}
