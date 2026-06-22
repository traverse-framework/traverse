//! Synchronous in-process event broker.
//!
//! Governed by spec 026-event-broker and spec 036-event-subscription-replay.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex},
    time::Duration,
};

use super::{
    catalog::EventCatalog,
    types::{
        BrokerEvent, EventBroker, EventCursor, EventError, LifecycleStatus, Subscription,
        SubscriptionId, SubscriptionPoll, TraverseEvent,
    },
};

/// Clock abstraction used by the broker for retention pruning.
pub trait BrokerClock: Send + Sync {
    fn now(&self) -> std::time::SystemTime;
}

#[derive(Debug)]
pub struct SystemClock;

impl BrokerClock for SystemClock {
    fn now(&self) -> std::time::SystemTime {
        std::time::SystemTime::now()
    }
}

/// Broker runtime configuration.
#[derive(Debug, Clone)]
pub struct BrokerConfig {
    pub retention_window: Duration,
    pub max_queue_len: usize,
}

impl Default for BrokerConfig {
    fn default() -> Self {
        Self {
            retention_window: Duration::from_mins(5),
            max_queue_len: 1024,
        }
    }
}

#[derive(Debug, Clone)]
struct BufferedEvent {
    cursor: u64,
    published_at: std::time::SystemTime,
    event: TraverseEvent,
}

#[derive(Debug)]
struct SubscriptionState {
    subscription_id: SubscriptionId,
    event_type: String,
    cursor: u64,
    queue: VecDeque<BufferedEvent>,
}

#[derive(Debug, Default)]
struct BrokerState {
    next_subscription: u64,
    next_cursor: HashMap<String, u64>,
    buffers: HashMap<String, VecDeque<BufferedEvent>>,
    seen_event_ids: HashMap<String, HashSet<String>>,
    subscriptions: HashMap<SubscriptionId, SubscriptionState>,
}

/// Synchronous, in-memory implementation of [`EventBroker`].
///
/// The broker stores a bounded retention buffer per event type and maintains a
/// bounded delivery queue per subscription. Subscribers poll for events using a
/// broker-issued subscription id and a cursor.
pub struct InProcessBroker {
    catalog: Arc<EventCatalog>,
    config: BrokerConfig,
    clock: Arc<dyn BrokerClock>,
    state: Mutex<BrokerState>,
}

impl std::fmt::Debug for InProcessBroker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InProcessBroker").finish_non_exhaustive()
    }
}

impl InProcessBroker {
    /// Create a new broker backed by the given catalog.
    ///
    /// # Errors
    ///
    /// Returns [`EventError::InvalidRetentionWindow`] when the provided configuration is invalid.
    pub fn new(catalog: Arc<EventCatalog>) -> Result<Self, EventError> {
        Self::with_clock(catalog, BrokerConfig::default(), Arc::new(SystemClock))
    }

    /// Create a broker with explicit configuration and clock.
    ///
    /// # Errors
    ///
    /// Returns [`EventError::InvalidRetentionWindow`] when the provided configuration is invalid.
    pub fn with_clock(
        catalog: Arc<EventCatalog>,
        config: BrokerConfig,
        clock: Arc<dyn BrokerClock>,
    ) -> Result<Self, EventError> {
        if config.retention_window == Duration::from_secs(0) {
            return Err(EventError::InvalidRetentionWindow(
                "retention_window must be > 0".to_string(),
            ));
        }
        if config.max_queue_len == 0 {
            return Err(EventError::InvalidRetentionWindow(
                "max_queue_len must be > 0".to_string(),
            ));
        }

        Ok(Self {
            catalog,
            config,
            clock,
            state: Mutex::new(BrokerState::default()),
        })
    }
}

fn parse_cursor(raw: &str) -> Result<u64, EventError> {
    let trimmed = raw.trim();
    if trimmed == "0" {
        return Ok(0);
    }
    trimmed.parse::<u64>().map_err(|_| {
        EventError::InvalidCursor("cursor must be \"0\" or a base-10 unsigned integer".to_string())
    })
}

fn cursor_to_string(cursor: u64) -> EventCursor {
    cursor.to_string()
}

fn enqueue_with_drop_oldest(
    queue: &mut VecDeque<BufferedEvent>,
    max_len: usize,
    item: BufferedEvent,
) {
    while queue.len() >= max_len {
        let _ = queue.pop_front();
    }
    queue.push_back(item);
}

fn prune_expired(
    state: &mut BrokerState,
    event_type: &str,
    retention_window: Duration,
    now: std::time::SystemTime,
) {
    let buffer = state.buffers.entry(event_type.to_string()).or_default();
    let mut oldest_retained_cursor = None;
    while let Some(front) = buffer.pop_front() {
        let age = now
            .duration_since(front.published_at)
            .unwrap_or(Duration::from_secs(0));
        if age <= retention_window {
            oldest_retained_cursor = Some(front.cursor);
            buffer.push_front(front);
            break;
        }

        if let Some(ids) = state.seen_event_ids.get_mut(event_type) {
            let _ = ids.remove(&front.event.id);
        }
    }

    let Some(oldest_cursor) = oldest_retained_cursor else {
        // Buffer is empty after pruning; nothing to sync.
        return;
    };

    // Sync per-subscription queues so they don't deliver events that are no longer retained.
    for sub in state.subscriptions.values_mut() {
        if sub.event_type != event_type {
            continue;
        }
        while let Some(front) = sub.queue.front() {
            if front.cursor >= oldest_cursor {
                break;
            }
            let _ = sub.queue.pop_front();
        }
        if sub.cursor != 0 && sub.cursor < oldest_cursor.saturating_sub(1) {
            // Cursor is now outside the retention window; keep it as-is so poll can surface cursor_expired.
        }
    }
}

fn validate_from_cursor(
    state: &BrokerState,
    event_type: &str,
    from_cursor: u64,
) -> Result<(), EventError> {
    if from_cursor == 0 {
        return Ok(());
    }

    let last_cursor = state.next_cursor.get(event_type).copied().unwrap_or(0);
    if let Some(buffer) = state.buffers.get(event_type)
        && let Some(front) = buffer.front()
    {
        let oldest_ok = front.cursor.saturating_sub(1);
        if from_cursor < oldest_ok {
            return Err(EventError::CursorExpired {
                event_type: event_type.to_string(),
                oldest_available_cursor: cursor_to_string(oldest_ok),
            });
        }
        return Ok(());
    }

    // If the buffer is empty but we have published events before, treat cursors behind the last
    // observed cursor as expired to avoid silent gaps.
    if last_cursor > 0 && from_cursor < last_cursor {
        return Err(EventError::CursorExpired {
            event_type: event_type.to_string(),
            oldest_available_cursor: cursor_to_string(last_cursor),
        });
    }

    Ok(())
}

impl EventBroker for InProcessBroker {
    /// Publish `event` to all registered subscribers.
    ///
    /// # Errors
    ///
    /// - [`EventError::UnregisteredEventType`] if the event type is not in the catalog.
    /// - [`EventError::LifecycleViolation`] if the catalog entry is `Draft` or `Deprecated`.
    fn publish(&self, event: TraverseEvent) -> Result<(), EventError> {
        let entry = self
            .catalog
            .get(&event.event_type)
            .ok_or_else(|| EventError::UnregisteredEventType(event.event_type.clone()))?;

        match entry.lifecycle_status {
            LifecycleStatus::Active => {}
            LifecycleStatus::Deprecated => {
                return Err(EventError::LifecycleViolation(format!(
                    "event type '{}' is Deprecated and cannot be published",
                    event.event_type
                )));
            }
            LifecycleStatus::Draft => {
                return Err(EventError::LifecycleViolation(format!(
                    "event type '{}' is Draft and cannot be published",
                    event.event_type
                )));
            }
        }

        let now = self.clock.now();

        let mut state = self
            .state
            .lock()
            .map_err(|_| EventError::LifecycleViolation("broker lock poisoned".to_owned()))?;

        prune_expired(
            &mut state,
            &event.event_type,
            self.config.retention_window,
            now,
        );

        let seen = state
            .seen_event_ids
            .entry(event.event_type.clone())
            .or_default();
        if seen.contains(&event.id) {
            // Duplicate emissions are silently discarded.
            return Ok(());
        }
        seen.insert(event.id.clone());

        let next = state
            .next_cursor
            .entry(event.event_type.clone())
            .or_insert(0);
        *next = next.saturating_add(1);
        let cursor = *next;

        let buffered = BufferedEvent {
            cursor,
            published_at: now,
            event: event.clone(),
        };

        state
            .buffers
            .entry(event.event_type.clone())
            .or_default()
            .push_back(buffered.clone());

        for sub in state.subscriptions.values_mut() {
            if sub.event_type != event.event_type {
                continue;
            }
            enqueue_with_drop_oldest(&mut sub.queue, self.config.max_queue_len, buffered.clone());
        }

        Ok(())
    }

    /// Create a subscription for `event_type` starting from `from_cursor`.
    ///
    /// The event type must already be registered in the catalog.
    ///
    /// # Errors
    ///
    /// Returns [`EventError::UnregisteredEventType`] if the event type is not catalogued.
    fn subscribe(&self, event_type: &str, from_cursor: &str) -> Result<Subscription, EventError> {
        if self.catalog.get(event_type).is_none() {
            return Err(EventError::UnregisteredEventType(event_type.to_owned()));
        }

        let from_cursor = parse_cursor(from_cursor)?;

        let now = self.clock.now();
        let mut state = self
            .state
            .lock()
            .map_err(|_| EventError::LifecycleViolation("broker lock poisoned".to_owned()))?;

        prune_expired(&mut state, event_type, self.config.retention_window, now);

        validate_from_cursor(&state, event_type, from_cursor)?;

        self.catalog.increment_consumer_count(event_type);

        state.next_subscription = state.next_subscription.saturating_add(1);
        let subscription_id = format!("sub-{}", state.next_subscription);

        let mut queue = VecDeque::new();
        for item in state
            .buffers
            .get(event_type)
            .into_iter()
            .flat_map(|buffer| buffer.iter())
        {
            if from_cursor == 0 || item.cursor > from_cursor {
                enqueue_with_drop_oldest(&mut queue, self.config.max_queue_len, item.clone());
            }
        }

        state.subscriptions.insert(
            subscription_id.clone(),
            SubscriptionState {
                subscription_id: subscription_id.clone(),
                event_type: event_type.to_string(),
                cursor: from_cursor,
                queue,
            },
        );

        Ok(Subscription {
            subscription_id,
            event_type: event_type.to_string(),
            cursor: cursor_to_string(from_cursor),
        })
    }

    /// Poll a subscription for up to `max_events`.
    ///
    /// # Errors
    ///
    fn poll(
        &self,
        subscription_id: &str,
        max_events: usize,
    ) -> Result<SubscriptionPoll, EventError> {
        let now = self.clock.now();
        let mut state = self
            .state
            .lock()
            .map_err(|_| EventError::LifecycleViolation("broker lock poisoned".to_owned()))?;

        let mut subscription = state
            .subscriptions
            .remove(subscription_id)
            .ok_or_else(|| EventError::SubscriptionNotFound(subscription_id.to_string()))?;
        let event_type = subscription.event_type.clone();
        let cursor = subscription.cursor;

        prune_expired(&mut state, &event_type, self.config.retention_window, now);

        validate_from_cursor(&state, &event_type, cursor)?;

        if let Some(buffer) = state.buffers.get(&event_type)
            && let Some(oldest_cursor) = buffer.front().map(|e| e.cursor)
        {
            while let Some(front) = subscription.queue.front() {
                if front.cursor >= oldest_cursor {
                    break;
                }
                let _ = subscription.queue.pop_front();
            }
        }

        if max_events == 0 {
            let cursor_str = cursor_to_string(subscription.cursor);
            state
                .subscriptions
                .insert(subscription.subscription_id.clone(), subscription);
            return Ok(SubscriptionPoll {
                subscription_id: subscription_id.to_string(),
                event_type,
                cursor: cursor_str,
                events: Vec::new(),
            });
        }

        let mut out = Vec::new();
        let mut delivered_cursor = subscription.cursor;
        for _ in 0..max_events {
            let Some(item) = subscription.queue.pop_front() else {
                break;
            };
            delivered_cursor = item.cursor;
            out.push(BrokerEvent {
                cursor: cursor_to_string(item.cursor),
                event: item.event,
            });
        }
        subscription.cursor = delivered_cursor;

        let subscription_id_value = subscription.subscription_id.clone();
        let event_type_value = subscription.event_type.clone();
        let cursor_value = cursor_to_string(subscription.cursor);
        state
            .subscriptions
            .insert(subscription.subscription_id.clone(), subscription);

        Ok(SubscriptionPoll {
            subscription_id: subscription_id_value,
            event_type: event_type_value,
            cursor: cursor_value,
            events: out,
        })
    }

    /// Cancel a subscription.
    ///
    /// # Errors
    ///
    /// Returns [`EventError::SubscriptionNotFound`] if the subscription id is unknown.
    fn cancel(&self, subscription_id: &str) -> Result<(), EventError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| EventError::LifecycleViolation("broker lock poisoned".to_owned()))?;

        if state.subscriptions.remove(subscription_id).is_none() {
            return Err(EventError::SubscriptionNotFound(
                subscription_id.to_string(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::events::catalog::EventCatalogEntry;

    fn cursor_expired_oldest(err: &EventError) -> Option<String> {
        if let EventError::CursorExpired {
            oldest_available_cursor,
            ..
        } = err
        {
            Some(oldest_available_cursor.clone())
        } else {
            None
        }
    }

    fn make_catalog(event_type: &str, status: LifecycleStatus) -> Arc<EventCatalog> {
        let catalog = Arc::new(EventCatalog::new());
        catalog
            .register(EventCatalogEntry {
                event_type: event_type.to_string(),
                owner: "cap.test".to_string(),
                version: "1.0.0".to_string(),
                lifecycle_status: status,
                consumer_count: 0,
            })
            .expect("catalog register must succeed");
        catalog
    }

    fn sample_event(event_type: &str, id: &str) -> TraverseEvent {
        TraverseEvent {
            id: id.to_string(),
            source: "traverse-runtime/cap.test".to_string(),
            event_type: event_type.to_string(),
            datacontenttype: "application/json".to_string(),
            time: "2026-04-08T00:00:00Z".to_string(),
            data: serde_json::json!({}),
            owner: "cap.test".to_string(),
            version: "1.0.0".to_string(),
            lifecycle_status: LifecycleStatus::Active,
        }
    }

    #[test]
    fn broker_debug_impl_is_accessible() {
        let catalog = make_catalog("dev.traverse.debug", LifecycleStatus::Active);
        let broker = InProcessBroker::new(catalog).expect("broker must be created");
        let rendered = format!("{broker:?}");
        assert!(rendered.contains("InProcessBroker"));
    }

    #[test]
    fn invalid_max_queue_len_is_rejected() {
        let catalog = make_catalog("dev.traverse.invalid", LifecycleStatus::Active);
        let err = InProcessBroker::with_clock(
            catalog,
            BrokerConfig {
                retention_window: Duration::from_secs(1),
                max_queue_len: 0,
            },
            Arc::new(SystemClock),
        )
        .expect_err("max_queue_len=0 must be rejected");
        assert!(matches!(err, EventError::InvalidRetentionWindow(_)));
    }

    #[test]
    fn invalid_cursor_is_rejected() {
        let catalog = make_catalog("dev.traverse.cursor", LifecycleStatus::Active);
        let broker = InProcessBroker::new(catalog).expect("broker must be created");
        let err = broker
            .subscribe("dev.traverse.cursor", "not-a-cursor")
            .expect_err("invalid cursor must fail");
        assert!(matches!(err, EventError::InvalidCursor(_)));
    }

    #[test]
    fn publish_rejects_deprecated_and_draft_event_types() {
        let deprecated = InProcessBroker::new(make_catalog(
            "dev.traverse.deprecated",
            LifecycleStatus::Deprecated,
        ))
        .expect("broker must be created");
        let err = deprecated
            .publish(sample_event("dev.traverse.deprecated", "evt-001"))
            .expect_err("deprecated publish must fail");
        assert!(matches!(err, EventError::LifecycleViolation(_)));

        let draft =
            InProcessBroker::new(make_catalog("dev.traverse.draft", LifecycleStatus::Draft))
                .expect("broker must be created");
        let err = draft
            .publish(sample_event("dev.traverse.draft", "evt-001"))
            .expect_err("draft publish must fail");
        assert!(matches!(err, EventError::LifecycleViolation(_)));
    }

    #[test]
    fn broker_lock_poisoning_surfaces_lifecycle_violation() {
        let broker =
            InProcessBroker::new(make_catalog("dev.traverse.poison", LifecycleStatus::Active))
                .expect("broker must be created");

        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = broker.state.lock().unwrap();
            panic!("poison lock");
        }));

        let err = broker
            .publish(sample_event("dev.traverse.poison", "evt-001"))
            .expect_err("poisoned publish must fail");
        assert!(matches!(err, EventError::LifecycleViolation(_)));

        let err = broker
            .subscribe("dev.traverse.poison", "0")
            .expect_err("poisoned subscribe must fail");
        assert!(matches!(err, EventError::LifecycleViolation(_)));

        let err = broker
            .poll("sub-1", 1)
            .expect_err("poisoned poll must fail");
        assert!(matches!(err, EventError::LifecycleViolation(_)));

        let err = broker
            .cancel("sub-1")
            .expect_err("poisoned cancel must fail");
        assert!(matches!(err, EventError::LifecycleViolation(_)));
    }

    #[derive(Debug)]
    struct ManualClock(std::sync::Mutex<std::time::SystemTime>);

    impl ManualClock {
        fn new(now: std::time::SystemTime) -> Self {
            Self(std::sync::Mutex::new(now))
        }

        fn advance(&self, by: Duration) {
            if let Ok(mut guard) = self.0.lock()
                && let Some(next) = guard.checked_add(by)
            {
                *guard = next;
            }
        }

        fn set(&self, now: std::time::SystemTime) {
            if let Ok(mut guard) = self.0.lock() {
                *guard = now;
            }
        }
    }

    impl BrokerClock for ManualClock {
        fn now(&self) -> std::time::SystemTime {
            self.0
                .lock()
                .ok()
                .map_or(std::time::SystemTime::UNIX_EPOCH, |guard| *guard)
        }
    }

    #[test]
    fn clock_regression_does_not_break_retention_pruning() {
        let clock = Arc::new(ManualClock::new(std::time::SystemTime::UNIX_EPOCH));
        let broker = InProcessBroker::with_clock(
            make_catalog("dev.traverse.clock", LifecycleStatus::Active),
            BrokerConfig {
                retention_window: Duration::from_mins(1),
                max_queue_len: 16,
            },
            clock.clone(),
        )
        .expect("broker must be created");

        clock.set(std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(10));
        broker
            .publish(sample_event("dev.traverse.clock", "evt-001"))
            .expect("publish must succeed");

        // Move time backwards to force duration_since() to hit the error path.
        clock.set(std::time::SystemTime::UNIX_EPOCH);
        broker
            .publish(sample_event("dev.traverse.clock", "evt-002"))
            .expect("publish must succeed");
    }

    #[test]
    fn publish_pruning_syncs_subscription_queues_and_skips_other_event_types() {
        let catalog = Arc::new(EventCatalog::new());
        catalog
            .register(EventCatalogEntry {
                event_type: "dev.traverse.a".to_string(),
                owner: "cap.test".to_string(),
                version: "1.0.0".to_string(),
                lifecycle_status: LifecycleStatus::Active,
                consumer_count: 0,
            })
            .expect("register must succeed");
        catalog
            .register(EventCatalogEntry {
                event_type: "dev.traverse.b".to_string(),
                owner: "cap.test".to_string(),
                version: "1.0.0".to_string(),
                lifecycle_status: LifecycleStatus::Active,
                consumer_count: 0,
            })
            .expect("register must succeed");

        let clock = Arc::new(ManualClock::new(std::time::SystemTime::UNIX_EPOCH));
        let broker = InProcessBroker::with_clock(
            catalog,
            BrokerConfig {
                retention_window: Duration::from_secs(5),
                max_queue_len: 64,
            },
            clock.clone(),
        )
        .expect("broker must be created");

        let sub_a = broker
            .subscribe("dev.traverse.a", "1")
            .expect("subscribe must succeed");
        let sub_b = broker
            .subscribe("dev.traverse.b", "0")
            .expect("subscribe must succeed");

        broker
            .publish(sample_event("dev.traverse.a", "evt-001"))
            .expect("publish must succeed");
        clock.advance(Duration::from_secs(1));
        broker
            .publish(sample_event("dev.traverse.a", "evt-002"))
            .expect("publish must succeed");
        clock.advance(Duration::from_secs(1));
        broker
            .publish(sample_event("dev.traverse.a", "evt-003"))
            .expect("publish must succeed");

        // Jump forward so evt-001 and evt-002 are outside retention; evt-003 is retained.
        clock.advance(Duration::from_secs(5));
        broker
            .publish(sample_event("dev.traverse.a", "evt-004"))
            .expect("publish must succeed");

        let err = broker
            .poll(&sub_a.subscription_id, 10)
            .expect_err("poll must surface cursor_expired after retention pruning");
        let oldest_available_cursor = cursor_expired_oldest(&err).expect("must be cursor_expired");

        let sub_a_resumed = broker
            .subscribe("dev.traverse.a", &oldest_available_cursor)
            .expect("subscribe must succeed");
        let poll_a = broker
            .poll(&sub_a_resumed.subscription_id, 10)
            .expect("poll must succeed");
        assert!(
            poll_a
                .events
                .first()
                .is_some_and(|e| e.event.id == "evt-003"),
            "queue must resume from oldest retained event"
        );

        let poll_b = broker
            .poll(&sub_b.subscription_id, 10)
            .expect("poll must succeed");
        assert!(
            poll_b.events.is_empty(),
            "event_type mismatch must not enqueue"
        );

        // Also cover the non-cursor_expired branch in the extraction logic above.
        let other_err = broker
            .poll("sub-missing", 10)
            .expect_err("poll must fail when subscription is missing");
        assert!(cursor_expired_oldest(&other_err).is_none());
    }

    #[test]
    fn subscribe_replays_events_from_existing_buffer() {
        let clock = Arc::new(ManualClock::new(std::time::SystemTime::UNIX_EPOCH));
        let broker = InProcessBroker::with_clock(
            make_catalog("dev.traverse.replay", LifecycleStatus::Active),
            BrokerConfig {
                retention_window: Duration::from_secs(5),
                max_queue_len: 64,
            },
            clock,
        )
        .expect("broker must be created");

        broker
            .publish(sample_event("dev.traverse.replay", "evt-001"))
            .expect("publish must succeed");

        let sub = broker
            .subscribe("dev.traverse.replay", "0")
            .expect("subscribe must succeed");
        let poll = broker
            .poll(&sub.subscription_id, 10)
            .expect("poll must succeed");
        assert_eq!(poll.events.len(), 1);
        assert_eq!(poll.events[0].event.id, "evt-001");
    }

    #[test]
    fn subscribe_rejects_cursor_expired_when_buffer_non_empty() {
        let clock = Arc::new(ManualClock::new(std::time::SystemTime::UNIX_EPOCH));
        let broker = InProcessBroker::with_clock(
            make_catalog("dev.traverse.expire", LifecycleStatus::Active),
            BrokerConfig {
                retention_window: Duration::from_secs(5),
                max_queue_len: 64,
            },
            clock.clone(),
        )
        .expect("broker must be created");

        for i in 1..=5 {
            broker
                .publish(sample_event("dev.traverse.expire", &format!("evt-{i:03}")))
                .expect("publish must succeed");
            clock.advance(Duration::from_secs(1));
        }

        // Advance so only the last event remains within retention.
        clock.advance(Duration::from_secs(5));

        let err = broker
            .subscribe("dev.traverse.expire", "1")
            .expect_err("subscribe must fail with cursor_expired");
        assert!(matches!(err, EventError::CursorExpired { .. }));
    }

    #[test]
    fn poll_with_zero_max_events_returns_empty() {
        let broker =
            InProcessBroker::new(make_catalog("dev.traverse.poll0", LifecycleStatus::Active))
                .expect("broker must be created");
        let sub = broker
            .subscribe("dev.traverse.poll0", "0")
            .expect("subscribe must succeed");
        let poll = broker
            .poll(&sub.subscription_id, 0)
            .expect("poll must succeed");
        assert!(poll.events.is_empty());
    }

    #[test]
    fn poll_prunes_subscription_queue_based_on_retention() {
        let clock = Arc::new(ManualClock::new(std::time::SystemTime::UNIX_EPOCH));
        let broker = InProcessBroker::with_clock(
            make_catalog("dev.traverse.pollprune", LifecycleStatus::Active),
            BrokerConfig {
                retention_window: Duration::from_secs(5),
                max_queue_len: 64,
            },
            clock.clone(),
        )
        .expect("broker must be created");

        let sub = broker
            .subscribe("dev.traverse.pollprune", "0")
            .expect("subscribe must succeed");

        broker
            .publish(sample_event("dev.traverse.pollprune", "evt-001"))
            .expect("publish must succeed");
        clock.advance(Duration::from_secs(4));
        broker
            .publish(sample_event("dev.traverse.pollprune", "evt-002"))
            .expect("publish must succeed");

        // Advance so evt-001 is outside retention but evt-002 is retained.
        clock.advance(Duration::from_secs(3));

        let poll = broker
            .poll(&sub.subscription_id, 10)
            .expect("poll must succeed");
        assert_eq!(poll.events.len(), 1);
        assert_eq!(poll.events[0].event.id, "evt-002");
    }

    #[test]
    fn cancel_unknown_subscription_returns_not_found() {
        let broker = InProcessBroker::new(make_catalog(
            "dev.traverse.cancel-miss",
            LifecycleStatus::Active,
        ))
        .expect("broker must be created");
        let err = broker.cancel("sub-missing").expect_err("cancel must fail");
        assert!(matches!(err, EventError::SubscriptionNotFound(_)));
    }
}
