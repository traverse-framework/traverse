//! In-process event system for Traverse.
//!
//! Governed by spec 026-event-broker and spec 036-event-subscription-replay.

pub mod broker;
pub mod catalog;
pub mod journal;
pub mod types;

pub use broker::{BrokerClock, BrokerConfig, InProcessBroker, SystemClock};
pub use catalog::{EventCatalog, EventCatalogEntry};
pub use journal::{DurableEventJournal, JournalConfig, JournalError};
pub use types::{
    BrokerEvent, EventBroker, EventCursor, EventError, LifecycleStatus, Subscription,
    SubscriptionId, SubscriptionPoll, TraverseEvent,
};
