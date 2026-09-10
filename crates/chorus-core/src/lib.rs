//! Chorus core — ledger, blob store, memory and context assembly.
//!
//! Design rule that governs every module here (see the Storage Spec):
//! **what we keep on disk and what we send to a model are different formats.**
//! Cold storage optimises for bytes and query speed; the hot context is
//! assembled fresh on every call and never written back.

pub mod actor;
pub mod blob;
pub mod checkpoint;
pub mod context;
pub mod error;
pub mod handoff;
pub mod ledger;
pub mod memory;
pub mod projector;
pub mod render;
pub mod schema;
pub mod store;
pub mod tokens;
pub mod types;

pub use error::{ChorusError, Result};
pub use store::Store;
pub use types::{BlobId, Event, EventKind, NewEvent, Rung};
