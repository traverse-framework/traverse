//! In-process event system for Traverse.
//!
//! Governed by spec 026-event-broker and spec 036-event-subscription-replay.

pub mod broker;
pub mod catalog;
pub mod durable;
pub mod journal;
pub mod types;

pub use broker::{BrokerClock, BrokerConfig, InProcessBroker, SystemClock};
pub use catalog::{EventCatalog, EventCatalogEntry};
pub use durable::{
    DurableBroker, DurableBrokerConfig, JournalSink, JournalWriteAuditRecord, JournalWriteAuditSink,
};
pub use journal::{DurableEventJournal, JournalConfig, JournalError};
pub use types::{
    BrokerEvent, BrokerEventSink, EventBroker, EventCursor, EventError, LifecycleStatus,
    NoopRuntimeEventSink, RuntimeEventSink, Subscription, SubscriptionId, SubscriptionPoll,
    TraverseEvent,
};
