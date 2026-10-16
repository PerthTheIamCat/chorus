//! Headless core daemon for Chorus (`chorusd`).
//! Serves Unix Domain Socket (`~/.chorus/engine.sock`) and SSE streaming
//! for Swift UI, CLI, and future mobile control.

pub mod client;
pub mod config;
pub mod error;
pub mod events;
pub mod http;
pub mod interaction;
pub mod ipc;
pub mod lifecycle;
pub mod pairing;
pub mod server;

pub use client::{DaemonClient, EventStreamReader};
pub use config::DaemonConfig;
pub use error::{DaemonError, Result};
pub use events::{format_sse, format_sse_comment, DaemonEvent, EventBroadcaster};
pub use interaction::{
    CreateInteractionRequest, Interaction, InteractionDecision, InteractionManager,
    InteractionStatus, ResolveInteractionRequest, RiskLevel,
};
pub use lifecycle::{LifecycleManager, LifecycleMode};
pub use pairing::{PairedDevice, PairingManager, PairingSession};
pub use server::DaemonServer;
