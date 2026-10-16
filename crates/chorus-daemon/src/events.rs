//! Push-based Server-Sent Events (SSE) streaming and SQLite catch-up replay.

use crate::error::Result;
use crate::interaction::Interaction;
use chorus_core::types::EventKind;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{mpsc, Arc, Mutex};

/// A normalized daemon event sent over SSE streams.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonEvent {
    pub seq: i64,
    pub t: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    pub kind: String,
    pub actor: String,
    pub body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interaction: Option<Interaction>,
}

impl DaemonEvent {
    /// Create a new generic daemon event.
    pub fn new(
        seq: i64,
        t: i64,
        session_id: Option<i64>,
        kind: impl Into<String>,
        actor: impl Into<String>,
        body: impl Into<String>,
    ) -> Self {
        Self {
            seq,
            t,
            session_id,
            kind: kind.into(),
            actor: actor.into(),
            body: body.into(),
            model_id: None,
            interaction: None,
        }
    }

    /// Attach an interaction payload to the event.
    pub fn with_interaction(mut self, interaction: Interaction) -> Self {
        self.interaction = Some(interaction);
        self
    }

    /// Attach model ID to the event.
    pub fn with_model(mut self, model_id: Option<String>) -> Self {
        self.model_id = model_id;
        self
    }
}

/// Format a `DaemonEvent` according to the SSE standard.
pub fn format_sse(event: &DaemonEvent) -> String {
    let data_json = serde_json::to_string(event).unwrap_or_default();
    format!(
        "id: {}\nevent: {}\ndata: {}\n\n",
        event.seq, event.kind, data_json
    )
}

/// Format an SSE comment line (for heartbeats / keep-alive).
pub fn format_sse_comment(comment: &str) -> String {
    format!(": {}\n\n", comment)
}

/// Broadcaster hub distributing live events to active SSE connections.
#[derive(Clone)]
pub struct EventBroadcaster {
    subscribers: Arc<Mutex<Vec<mpsc::SyncSender<DaemonEvent>>>>,
    counter: Arc<AtomicI64>,
}

impl Default for EventBroadcaster {
    fn default() -> Self {
        Self::new(1000)
    }
}

impl EventBroadcaster {
    /// Initialize broadcaster with starting synthetic sequence ID.
    pub fn new(initial_seq: i64) -> Self {
        Self {
            subscribers: Arc::new(Mutex::new(Vec::new())),
            counter: Arc::new(AtomicI64::new(initial_seq)),
        }
    }

    /// Generate the next monotonic sequence number.
    pub fn next_seq(&self) -> i64 {
        self.counter.fetch_add(1, Ordering::SeqCst)
    }

    /// Subscribe to live events. Returns an `mpsc::Receiver`.
    pub fn subscribe(&self) -> mpsc::Receiver<DaemonEvent> {
        // Disconnect lagging consumers rather than accumulating events without a bound.
        let (tx, rx) = mpsc::sync_channel(256);
        let mut subs = self.subscribers.lock().unwrap();
        subs.push(tx);
        rx
    }

    /// Broadcast an event to all connected subscribers.
    pub fn broadcast(&self, event: DaemonEvent) {
        let mut subs = self.subscribers.lock().unwrap();
        subs.retain(|tx| tx.try_send(event.clone()).is_ok());
    }

    /// Active subscriber count.
    pub fn subscriber_count(&self) -> usize {
        let subs = self.subscribers.lock().unwrap();
        subs.len()
    }
}

/// Map integer `EventKind` to string name.
pub fn event_kind_name(kind: i64) -> &'static str {
    match EventKind::from_i64(kind) {
        Some(EventKind::Msg) => "Msg",
        Some(EventKind::ToolCall) => "ToolCall",
        Some(EventKind::ToolResult) => "ToolResult",
        Some(EventKind::Diff) => "Diff",
        Some(EventKind::Decision) => "Decision",
        Some(EventKind::Handoff) => "Handoff",
        Some(EventKind::Summary) => "Summary",
        None => "Unknown",
    }
}

/// Replay ledger events from SQLite with `seq > after_seq`, optionally filtered by `session_id`.
pub fn replay_ledger_events(
    conn: &Connection,
    after_seq: i64,
    session_id: Option<i64>,
) -> Result<Vec<DaemonEvent>> {
    let mut query = String::from(
        "SELECT e.seq, e.t, e.sess, e.kind, e.actor, e.body, e.blob, e.model_id,
                COALESCE(a.name, '?') as actor_name
         FROM event e
         LEFT JOIN actor a ON a.id = e.actor
         WHERE e.seq > ?1",
    );

    let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(after_seq)];

    if let Some(sess) = session_id {
        query.push_str(" AND e.sess = ?2");
        params.push(Box::new(sess));
    }

    query.push_str(" ORDER BY e.seq ASC");

    let mut stmt = conn.prepare(&query)?;
    let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();

    let rows = stmt.query_map(param_refs.as_slice(), |row| {
        let seq: i64 = row.get(0)?;
        let t: i64 = row.get(1)?;
        let sess: i64 = row.get(2)?;
        let kind_raw: i64 = row.get(3)?;
        let _actor_id: i64 = row.get(4)?;
        let inline_body: Option<String> = row.get(5)?;
        let blob_id: Option<String> = row.get(6)?;
        let model_id: Option<String> = row.get(7)?;
        let actor_name: String = row.get(8)?;

        Ok((
            seq,
            t,
            sess,
            kind_raw,
            inline_body,
            blob_id,
            model_id,
            actor_name,
        ))
    })?;

    let mut events = Vec::new();
    for row in rows {
        let (seq, t, sess, kind_raw, inline_body, blob_id, model_id, actor_name) = row?;

        // Resolve body text if stored in blob
        let body = if let Some(b) = inline_body {
            b
        } else if let Some(blob_str) = blob_id {
            chorus_core::blob::get_text(conn, &chorus_core::BlobId(blob_str)).unwrap_or_default()
        } else {
            String::new()
        };

        events.push(DaemonEvent {
            seq,
            t,
            session_id: Some(sess),
            kind: event_kind_name(kind_raw).to_string(),
            actor: actor_name,
            body,
            model_id,
            interaction: None,
        });
    }

    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_formatting_conforms_to_spec() {
        let event = DaemonEvent::new(
            42,
            100,
            Some(1),
            "ToolCall",
            "codex",
            "{\"command\":\"ls\"}",
        );
        let sse = format_sse(&event);
        assert!(sse.starts_with("id: 42\n"));
        assert!(sse.contains("event: ToolCall\n"));
        assert!(sse.contains("data: {"));
        assert!(sse.ends_with("\n\n"));
    }

    #[test]
    fn broadcaster_delivers_to_subscribers() {
        let broadcaster = EventBroadcaster::new(10);
        let rx1 = broadcaster.subscribe();
        let rx2 = broadcaster.subscribe();

        let event = DaemonEvent::new(broadcaster.next_seq(), 0, None, "Msg", "user", "hello");
        broadcaster.broadcast(event.clone());

        let ev1 = rx1
            .recv_timeout(std::time::Duration::from_millis(500))
            .unwrap();
        let ev2 = rx2
            .recv_timeout(std::time::Duration::from_millis(500))
            .unwrap();

        assert_eq!(ev1.seq, 10);
        assert_eq!(ev1.body, "hello");
        assert_eq!(ev2.seq, 10);
        assert_eq!(ev2.body, "hello");
    }

    #[test]
    fn slow_subscriber_is_disconnected_without_blocking_healthy_subscriber() {
        let broadcaster = EventBroadcaster::new(1);
        let slow = broadcaster.subscribe();
        let healthy = broadcaster.subscribe();
        for seq in 1..=257 {
            broadcaster.broadcast(DaemonEvent::new(seq, 0, None, "Msg", "fixture", "body"));
            assert_eq!(healthy.try_recv().unwrap().seq, seq);
        }
        assert_eq!(broadcaster.subscriber_count(), 1);
        for seq in 1..=256 {
            assert_eq!(slow.try_recv().unwrap().seq, seq);
        }
        assert!(matches!(
            slow.try_recv(),
            Err(mpsc::TryRecvError::Disconnected)
        ));
    }
}
