//! Chorus harness evaluation framework.
//! Provides Tier 0 (invariants), Tier 1 (router/scaffolding gate), and metrics telemetry.

pub mod classifier;
pub mod loop_detector;
pub mod metrics;
pub mod models;
pub mod suite;
pub mod tier0;
pub mod tier1;
pub mod tier2;
pub mod verification;

pub use suite::*;
pub use tier2::{run_tier2, Tier2Report};
