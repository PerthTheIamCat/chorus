//! Chorus CLI internals, exposed as a library so integration tests (and a
//! future UniFFI shell) can drive the Session Engine directly.

pub mod accounts;
pub mod agents;
pub mod child_verify;
pub mod drafts;
pub mod inspector;
pub mod limits;
pub mod metrics;
pub mod orchestra;
pub mod session;
pub mod taint_gate;
pub use chorus_acp::{cancel_active, sandbox, set_push_sink, shutdown, Push};
