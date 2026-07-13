//! Core types for the in-process event system.
//!
//! Governed by spec 026-event-broker and spec 036-event-subscription-replay.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Lifecycle status of an event type in the catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleStatus {
    Draft,
    Active,
    Deprecated,
}

/// A CloudEvents-formatted event with Traverse governance metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraverseEvent {
    /// UUID for this event instance.
    pub id: String,
    /// Originating capability: `"traverse-runtime/<capability_id>"`.
    pub source: String,
    /// Reverse-DNS event type, e.g. `"dev.traverse.expedition.planned"`.
    pub event_type: String,
    /// Always `"application/json"`.
    pub datacontenttype: String,
    /// RFC 3339 timestamp.
    pub time: String,
    /// Event payload.
    pub data: Value,
    // --- governance metadata ---
    /// Capability ID that emits this event.
    pub owner: String,
    /// Event contract version.
    pub version: String,
    /// Lifecycle status at the time the event was created.
    pub lifecycle_status: LifecycleStatus,
}

/// Errors that can occur during event broker operations.
#[derive(Debug, PartialEq, Eq)]
pub enum EventError {
    /// Attempted to publish an event whose catalog entry is `Deprecated` or `Draft`.
    LifecycleViolation(String),
    /// Attempted to publish an event type not registered in the catalog.
    UnregisteredEventType(String),
    /// Cursor string could not be parsed.
    InvalidCursor(String),
    /// The requested cursor is outside the active retention window.
    CursorExpired {
        event_type: String,
        oldest_available_cursor: String,
    },
    /// Subscription id is unknown or was cancelled.
    SubscriptionNotFound(String),
    /// Broker was configured with an invalid retention window.
    InvalidRetentionWindow(String),
    /// Durable journal write failed; the event was not acknowledged (066 FR-006).
    JournalWrite(String),
    /// Durable journal write exceeded the configured timeout; the event was
    /// rejected, not delivered (067 FR-003/FR-004).
    JournalWriteTimeout(String),
}

impl std::fmt::Display for EventError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LifecycleViolation(msg) => write!(f, "lifecycle violation: {msg}"),
            Self::UnregisteredEventType(t) => write!(f, "unregistered event type: {t}"),
            Self::InvalidCursor(msg) => write!(f, "invalid cursor: {msg}"),
            Self::CursorExpired {
                event_type,
                oldest_available_cursor,
            } => write!(
                f,
                "cursor expired for event type '{event_type}': oldest available cursor is {oldest_available_cursor}"
            ),
            Self::SubscriptionNotFound(id) => write!(f, "subscription not found: {id}"),
            Self::InvalidRetentionWindow(msg) => write!(f, "invalid retention window: {msg}"),
            Self::JournalWrite(msg) => write!(f, "journal write failed: {msg}"),
            Self::JournalWriteTimeout(msg) => write!(f, "journal_write_timeout: {msg}"),
        }
    }
}

impl std::error::Error for EventError {}

/// Pub/sub interface for in-process event delivery.
pub trait EventBroker: Send + Sync {
    /// Publish an event. Fails if the event type is not `Active` in the catalog.
    ///
    /// # Errors
    ///
    /// Returns [`EventError::UnregisteredEventType`] if the event type is not in the catalog,
    /// or [`EventError::LifecycleViolation`] if the catalog entry is not `Active`.
    fn publish(&self, event: TraverseEvent) -> Result<(), EventError>;

    /// Create a subscription for the given `event_type` starting from `from_cursor`.
    ///
    /// `from_cursor` is an opaque cursor string previously returned by [`poll`](Self::poll).
    /// The special value `"0"` requests replay from the start of the active retention window.
    ///
    /// # Errors
    ///
    /// Returns [`EventError::UnregisteredEventType`] if the event type is not in the catalog,
    /// [`EventError::InvalidCursor`] if the cursor string is malformed, or
    /// [`EventError::CursorExpired`] if the cursor is outside the retention window.
    fn subscribe(&self, event_type: &str, from_cursor: &str) -> Result<Subscription, EventError>;

    /// Poll a subscription for up to `max_events`.
    ///
    /// # Errors
    ///
    /// Returns [`EventError::SubscriptionNotFound`] if the subscription id is unknown or cancelled.
    fn poll(
        &self,
        subscription_id: &str,
        max_events: usize,
    ) -> Result<SubscriptionPoll, EventError>;

    /// Cancel a subscription and free all associated queues.
    ///
    /// # Errors
    ///
    /// Returns [`EventError::SubscriptionNotFound`] if the subscription id is unknown.
    fn cancel(&self, subscription_id: &str) -> Result<(), EventError>;
}

/// A broker-issued event cursor string.
pub type EventCursor = String;

/// A broker-assigned subscription identifier.
pub type SubscriptionId = String;

/// Event delivered by the broker, carrying a cursor for replay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrokerEvent {
    pub cursor: EventCursor,
    pub event: TraverseEvent,
}

/// A broker subscription handle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub subscription_id: SubscriptionId,
    pub event_type: String,
    pub cursor: EventCursor,
}

/// Result of polling a subscription.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionPoll {
    pub subscription_id: SubscriptionId,
    pub event_type: String,
    pub cursor: EventCursor,
    pub events: Vec<BrokerEvent>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_error_display_covers_all_variants() {
        let cases: Vec<EventError> = vec![
            EventError::LifecycleViolation("x".to_string()),
            EventError::UnregisteredEventType("t".to_string()),
            EventError::InvalidCursor("c".to_string()),
            EventError::CursorExpired {
                event_type: "evt".to_string(),
                oldest_available_cursor: "7".to_string(),
            },
            EventError::SubscriptionNotFound("sub-1".to_string()),
            EventError::InvalidRetentionWindow("bad".to_string()),
            EventError::JournalWrite("disk gone".to_string()),
            EventError::JournalWriteTimeout("exceeded 2000ms".to_string()),
        ];

        for err in cases {
            let rendered = err.to_string();
            assert!(!rendered.is_empty());
        }
    }
}
