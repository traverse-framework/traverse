use std::{
    sync::Arc,
    time::{Duration, SystemTime},
};

use traverse_runtime::events::{
    BrokerClock, BrokerConfig, EventBroker, EventCatalog, EventCatalogEntry, EventError,
    InProcessBroker, LifecycleStatus, TraverseEvent,
};

fn active_entry(event_type: &str) -> EventCatalogEntry {
    EventCatalogEntry {
        event_type: event_type.to_string(),
        owner: "cap.test".to_string(),
        version: "1.0.0".to_string(),
        lifecycle_status: LifecycleStatus::Active,
        consumer_count: 0,
    }
}

fn sample_event(event_type: &str, id: &str, time: &str) -> TraverseEvent {
    TraverseEvent {
        id: id.to_string(),
        source: "traverse-runtime/cap.test".to_string(),
        event_type: event_type.to_string(),
        datacontenttype: "application/json".to_string(),
        time: time.to_string(),
        data: serde_json::json!({}),
        owner: "cap.test".to_string(),
        version: "1.0.0".to_string(),
        lifecycle_status: LifecycleStatus::Active,
    }
}

fn broker_with_active(event_type: &str) -> Result<InProcessBroker, String> {
    let catalog = Arc::new(EventCatalog::new());
    catalog
        .register(active_entry(event_type))
        .map_err(|e| e.to_string())?;
    InProcessBroker::new(catalog).map_err(|e| e.to_string())
}

#[derive(Debug)]
struct ManualClock(std::sync::Mutex<SystemTime>);

impl ManualClock {
    fn new(now: SystemTime) -> Self {
        Self(std::sync::Mutex::new(now))
    }

    fn advance(&self, by: Duration) {
        if let Ok(mut guard) = self.0.lock()
            && let Some(next) = guard.checked_add(by)
        {
            *guard = next;
        }
    }
}

impl BrokerClock for ManualClock {
    fn now(&self) -> SystemTime {
        self.0
            .lock()
            .ok()
            .map_or(SystemTime::UNIX_EPOCH, |guard| *guard)
    }
}

#[test]
fn publish_to_active_event_type_is_pollable_by_subscription() -> Result<(), String> {
    let broker = broker_with_active("dev.traverse.test.happened")?;
    let sub = broker
        .subscribe("dev.traverse.test.happened", "0")
        .map_err(|e| e.to_string())?;

    broker
        .publish(sample_event(
            "dev.traverse.test.happened",
            "evt-001",
            "2026-04-08T00:00:00Z",
        ))
        .map_err(|e| e.to_string())?;

    let poll = broker
        .poll(&sub.subscription_id, 10)
        .map_err(|e| e.to_string())?;
    assert_eq!(poll.events.len(), 1);
    assert_eq!(poll.events[0].event.id, "evt-001");
    assert_eq!(poll.cursor, "1");
    Ok(())
}

#[test]
fn late_join_subscribe_replays_from_cursor_zero() -> Result<(), String> {
    let broker = broker_with_active("dev.traverse.test.happened")?;

    broker
        .publish(sample_event(
            "dev.traverse.test.happened",
            "evt-001",
            "2026-04-08T00:00:00Z",
        ))
        .map_err(|e| e.to_string())?;

    let sub = broker
        .subscribe("dev.traverse.test.happened", "0")
        .map_err(|e| e.to_string())?;
    let poll = broker
        .poll(&sub.subscription_id, 10)
        .map_err(|e| e.to_string())?;
    assert_eq!(poll.events.len(), 1);
    assert_eq!(poll.events[0].event.id, "evt-001");
    Ok(())
}

#[test]
fn reconnect_replays_events_after_cursor() -> Result<(), String> {
    let broker = broker_with_active("dev.traverse.test.happened")?;

    let sub1 = broker
        .subscribe("dev.traverse.test.happened", "0")
        .map_err(|e| e.to_string())?;

    broker
        .publish(sample_event(
            "dev.traverse.test.happened",
            "evt-001",
            "2026-04-08T00:00:00Z",
        ))
        .map_err(|e| e.to_string())?;
    broker
        .publish(sample_event(
            "dev.traverse.test.happened",
            "evt-002",
            "2026-04-08T00:00:01Z",
        ))
        .map_err(|e| e.to_string())?;

    let poll1 = broker
        .poll(&sub1.subscription_id, 10)
        .map_err(|e| e.to_string())?;
    assert_eq!(poll1.events.len(), 2);
    let cursor = poll1.cursor;

    broker
        .publish(sample_event(
            "dev.traverse.test.happened",
            "evt-003",
            "2026-04-08T00:00:02Z",
        ))
        .map_err(|e| e.to_string())?;
    broker
        .publish(sample_event(
            "dev.traverse.test.happened",
            "evt-004",
            "2026-04-08T00:00:03Z",
        ))
        .map_err(|e| e.to_string())?;
    broker
        .publish(sample_event(
            "dev.traverse.test.happened",
            "evt-005",
            "2026-04-08T00:00:04Z",
        ))
        .map_err(|e| e.to_string())?;

    let sub2 = broker
        .subscribe("dev.traverse.test.happened", &cursor)
        .map_err(|e| e.to_string())?;
    let poll2 = broker
        .poll(&sub2.subscription_id, 10)
        .map_err(|e| e.to_string())?;

    assert_eq!(poll2.events.len(), 3);
    assert_eq!(poll2.events[0].event.id, "evt-003");
    assert_eq!(poll2.events[2].event.id, "evt-005");
    Ok(())
}

#[test]
fn two_subscribers_receive_independent_ordered_streams() -> Result<(), String> {
    let broker = broker_with_active("dev.traverse.test.happened")?;

    let a = broker
        .subscribe("dev.traverse.test.happened", "0")
        .map_err(|e| e.to_string())?;
    let b = broker
        .subscribe("dev.traverse.test.happened", "0")
        .map_err(|e| e.to_string())?;

    for i in 1..=5 {
        broker
            .publish(sample_event(
                "dev.traverse.test.happened",
                &format!("evt-{i:03}"),
                "2026-04-08T00:00:00Z",
            ))
            .map_err(|e| e.to_string())?;
    }

    let poll_a = broker
        .poll(&a.subscription_id, 10)
        .map_err(|e| e.to_string())?;
    let poll_b = broker
        .poll(&b.subscription_id, 10)
        .map_err(|e| e.to_string())?;
    assert_eq!(poll_a.events.len(), 5);
    assert_eq!(poll_b.events.len(), 5);
    assert_eq!(poll_a.events[0].event.id, "evt-001");
    assert_eq!(poll_b.events[4].event.id, "evt-005");
    Ok(())
}

#[test]
fn cursor_expired_is_returned_when_cursor_is_outside_retention_window() -> Result<(), String> {
    let catalog = Arc::new(EventCatalog::new());
    catalog
        .register(active_entry("dev.traverse.retained"))
        .map_err(|e| e.to_string())?;

    let clock = Arc::new(ManualClock::new(SystemTime::UNIX_EPOCH));
    let broker_clock: Arc<dyn BrokerClock> = clock.clone();
    let config = BrokerConfig {
        retention_window: Duration::from_secs(10),
        max_queue_len: 64,
    };

    let broker = InProcessBroker::with_clock(Arc::clone(&catalog), config, broker_clock)
        .map_err(|e| e.to_string())?;

    for i in 1..=5 {
        broker
            .publish(sample_event(
                "dev.traverse.retained",
                &format!("evt-{i:03}"),
                "2026-04-08T00:00:00Z",
            ))
            .map_err(|e| e.to_string())?;
        clock.advance(Duration::from_secs(1));
    }

    // Advance beyond retention window so all buffered events are pruned.
    clock.advance(Duration::from_mins(1));

    let Err(err) = broker.subscribe("dev.traverse.retained", "1") else {
        return Err("expected subscribe to fail with cursor_expired".to_string());
    };
    match err {
        EventError::CursorExpired {
            event_type,
            oldest_available_cursor,
        } => {
            assert_eq!(event_type, "dev.traverse.retained");
            assert_eq!(oldest_available_cursor, "5");
        }
        other => return Err(format!("expected CursorExpired, got {other:?}")),
    }

    Ok(())
}

#[test]
fn bounded_queue_drops_oldest_when_over_capacity() -> Result<(), String> {
    let catalog = Arc::new(EventCatalog::new());
    catalog
        .register(active_entry("dev.traverse.backpressure"))
        .map_err(|e| e.to_string())?;

    let broker = InProcessBroker::with_clock(
        Arc::clone(&catalog),
        BrokerConfig {
            retention_window: Duration::from_mins(1),
            max_queue_len: 2,
        },
        Arc::new(traverse_runtime::events::SystemClock),
    )
    .map_err(|e| e.to_string())?;

    let sub = broker
        .subscribe("dev.traverse.backpressure", "0")
        .map_err(|e| e.to_string())?;

    broker
        .publish(sample_event(
            "dev.traverse.backpressure",
            "evt-001",
            "2026-04-08T00:00:00Z",
        ))
        .map_err(|e| e.to_string())?;
    broker
        .publish(sample_event(
            "dev.traverse.backpressure",
            "evt-002",
            "2026-04-08T00:00:01Z",
        ))
        .map_err(|e| e.to_string())?;
    broker
        .publish(sample_event(
            "dev.traverse.backpressure",
            "evt-003",
            "2026-04-08T00:00:02Z",
        ))
        .map_err(|e| e.to_string())?;

    let poll = broker
        .poll(&sub.subscription_id, 10)
        .map_err(|e| e.to_string())?;
    assert_eq!(poll.events.len(), 2);
    assert_eq!(poll.events[0].event.id, "evt-002");
    assert_eq!(poll.events[1].event.id, "evt-003");
    Ok(())
}

#[test]
fn duplicate_event_ids_are_discarded() -> Result<(), String> {
    let broker = broker_with_active("dev.traverse.dedup")?;
    let sub = broker
        .subscribe("dev.traverse.dedup", "0")
        .map_err(|e| e.to_string())?;

    broker
        .publish(sample_event(
            "dev.traverse.dedup",
            "dup-001",
            "2026-04-08T00:00:00Z",
        ))
        .map_err(|e| e.to_string())?;
    broker
        .publish(sample_event(
            "dev.traverse.dedup",
            "dup-001",
            "2026-04-08T00:00:01Z",
        ))
        .map_err(|e| e.to_string())?;

    let poll = broker
        .poll(&sub.subscription_id, 10)
        .map_err(|e| e.to_string())?;
    assert_eq!(poll.events.len(), 1);
    Ok(())
}

#[test]
fn cancel_removes_subscription_and_prevents_polling() -> Result<(), String> {
    let broker = broker_with_active("dev.traverse.cancel")?;
    let sub = broker
        .subscribe("dev.traverse.cancel", "0")
        .map_err(|e| e.to_string())?;

    broker
        .cancel(&sub.subscription_id)
        .map_err(|e| e.to_string())?;

    let Err(err) = broker.poll(&sub.subscription_id, 1) else {
        return Err("expected poll to fail after cancel".to_string());
    };
    assert!(matches!(err, EventError::SubscriptionNotFound(_)));
    Ok(())
}

#[test]
fn subscribe_to_unregistered_event_type_returns_error() -> Result<(), String> {
    let catalog = Arc::new(EventCatalog::new());
    let broker = InProcessBroker::new(catalog).map_err(|e| e.to_string())?;
    let result = broker.subscribe("dev.traverse.missing", "0");
    assert!(
        matches!(result, Err(EventError::UnregisteredEventType(_))),
        "expected UnregisteredEventType, got {result:?}"
    );
    Ok(())
}

#[test]
fn publishing_unregistered_event_type_returns_error() -> Result<(), String> {
    let catalog = Arc::new(EventCatalog::new());
    let broker = InProcessBroker::new(catalog).map_err(|e| e.to_string())?;
    let result = broker.publish(sample_event(
        "dev.traverse.unknown.event",
        "evt-001",
        "2026-04-08T00:00:00Z",
    ));
    assert!(
        matches!(result, Err(EventError::UnregisteredEventType(_))),
        "expected UnregisteredEventType, got {result:?}"
    );
    Ok(())
}

#[test]
fn catalog_consumer_count_increments_on_subscribe() -> Result<(), String> {
    let catalog = Arc::new(EventCatalog::new());
    catalog
        .register(active_entry("dev.traverse.counted"))
        .map_err(|e| e.to_string())?;
    let broker = InProcessBroker::new(Arc::clone(&catalog)).map_err(|e| e.to_string())?;

    let _ = broker
        .subscribe("dev.traverse.counted", "0")
        .map_err(|e| e.to_string())?;
    let _ = broker
        .subscribe("dev.traverse.counted", "0")
        .map_err(|e| e.to_string())?;

    let entry = catalog
        .get("dev.traverse.counted")
        .ok_or_else(|| "entry not found in catalog".to_string())?;
    assert_eq!(entry.consumer_count, 2);
    Ok(())
}

#[test]
fn invalid_retention_window_is_rejected() -> Result<(), String> {
    let catalog = Arc::new(EventCatalog::new());
    let err = InProcessBroker::with_clock(
        catalog,
        BrokerConfig {
            retention_window: Duration::from_secs(0),
            max_queue_len: 1,
        },
        Arc::new(traverse_runtime::events::SystemClock),
    )
    .err()
    .ok_or_else(|| "expected invalid retention window to be rejected".to_string())?;
    assert!(matches!(err, EventError::InvalidRetentionWindow(_)));
    Ok(())
}
