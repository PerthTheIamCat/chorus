//! Shared value types. Owned by the workspace contract: keep stable.
use serde::{Deserialize, Serialize};

/// Event kinds are stored as small integers, never strings.
/// A 5,000-event session would otherwise pay for `"tool_result"` 5,000 times.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(i64)]
pub enum EventKind {
    Msg = 0,
    ToolCall = 1,
    ToolResult = 2,
    Diff = 3,
    Decision = 4,
    Handoff = 5,
    Summary = 6,
}

impl EventKind {
    pub fn from_i64(v: i64) -> Option<Self> {
        Some(match v {
            0 => Self::Msg,
            1 => Self::ToolCall,
            2 => Self::ToolResult,
            3 => Self::Diff,
            4 => Self::Decision,
            5 => Self::Handoff,
            6 => Self::Summary,
            _ => return None,
        })
    }
    pub fn as_i64(self) -> i64 {
        self as i64
    }
}

/// Which compaction rung a rendered event came from (see Storage Spec §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Rung {
    /// Raw event, verbatim.
    L0 = 0,
    /// Per-turn summary, ~60 tokens.
    L1 = 1,
    /// Episode summary, ~250 tokens.
    L2 = 2,
    /// Durable fact, ~15 tokens.
    L3 = 3,
}

/// Content address of a blob: BLAKE3, hex, prefixed `b3:`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BlobId(pub String);

impl BlobId {
    pub fn of(bytes: &[u8]) -> Self {
        BlobId(format!("b3:{}", blake3::hash(bytes).to_hex()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for BlobId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One row of the ledger, as read back.
#[derive(Debug, Clone)]
pub struct Event {
    pub seq: i64,
    /// ms offset from the session's start, not an ISO string.
    pub t: i64,
    pub sess: i64,
    pub parent: Option<i64>,
    pub kind: EventKind,
    pub actor: i64,
    /// Inline body when small; `None` when the payload lives in the blob store.
    pub body: Option<String>,
    pub blob: Option<BlobId>,
    pub tok_in: Option<i64>,
    pub tok_out: Option<i64>,
    pub cost_usd: Option<f64>,
    /// Model reported by the agent for this message, when the bridge provides it.
    pub model_id: Option<String>,
}

/// What to append. `body` is routed to inline storage or the blob store by size.
#[derive(Debug, Clone)]
pub struct NewEvent {
    pub sess: i64,
    pub parent: Option<i64>,
    pub kind: EventKind,
    pub actor: i64,
    pub body: String,
    pub tok_in: Option<i64>,
    pub tok_out: Option<i64>,
    pub cost_usd: Option<f64>,
    pub model_id: Option<String>,
}

impl NewEvent {
    pub fn new(sess: i64, kind: EventKind, actor: i64, body: impl Into<String>) -> Self {
        Self { sess, parent: None, kind, actor, body: body.into(), tok_in: None, tok_out: None, cost_usd: None, model_id: None }
    }
    pub fn parent(mut self, p: i64) -> Self {
        self.parent = Some(p);
        self
    }
    pub fn model(mut self, model_id: Option<String>) -> Self {
        self.model_id = model_id;
        self
    }
}

/// Payloads at or above this many bytes go to the blob store instead of the row.
pub const INLINE_MAX_BYTES: usize = 2 * 1024;
