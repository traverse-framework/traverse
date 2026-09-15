//! Placement constraint evaluator for Traverse.
//!
//! Governs spec: 024-placement-constraint-evaluator
//!
//! The algorithm lives in [`traverse_contracts::placement`] so native
//! `PlacementRouter` and nested `runtime.wasm` share one implementation
//! (spec `1402` FR-003/FR-004). This module re-exports that shared core.

pub use traverse_contracts::placement::{
    PlacementConfidence, PlacementConstraintEvaluator, PlacementDecision, PlacementError,
    PlacementReason, PlacementRequest, RuntimeSnapshot,
};
