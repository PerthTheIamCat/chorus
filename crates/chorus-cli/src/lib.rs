//! Chorus CLI internals, exposed as a library so integration tests (and a
//! future UniFFI shell) can drive the Session Engine directly.

pub mod agents;
pub mod session;
pub use chorus_acp::cancel_active;
