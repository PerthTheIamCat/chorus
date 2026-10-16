//! Async `Interaction` state machine and persistence in SQLite.
//! Replaces blocking FFI callbacks with non-blocking, queryable interaction records.

use crate::error::{DaemonError, Result};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Lifecycle status of an interaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionStatus {
    Pending,
    Allowed,
    Denied,
    Canceled,
}

impl InteractionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Allowed => "allowed",
            Self::Denied => "denied",
            Self::Canceled => "canceled",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "pending" => Some(Self::Pending),
            "allowed" => Some(Self::Allowed),
            "denied" => Some(Self::Denied),
            "canceled" => Some(Self::Canceled),
            _ => None,
        }
    }
}

/// The decision submitted to resolve an interaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionDecision {
    Allow,
    Deny,
    Reply,
}

impl InteractionDecision {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
            Self::Reply => "reply",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "allow" => Some(Self::Allow),
            "deny" => Some(Self::Deny),
            "reply" => Some(Self::Reply),
            _ => None,
        }
    }
}

/// Estimated risk level of the requested action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

impl RiskLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "critical" => Some(Self::Critical),
            _ => None,
        }
    }
}

/// An interaction resource stored in the database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Interaction {
    pub id: String,
    pub session_id: Option<i64>,
    pub tool: String,
    pub kind: String,
    pub detail: String,
    pub risk_level: RiskLevel,
    pub status: InteractionStatus,
    pub decision: Option<InteractionDecision>,
    pub feedback: Option<String>,
    pub created_ms: i64,
    pub resolved_ms: Option<i64>,
}

/// Request payload to create a new pending interaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateInteractionRequest {
    pub session_id: Option<i64>,
    pub tool: String,
    pub kind: String,
    pub detail: String,
    #[serde(default)]
    pub risk_level: Option<RiskLevel>,
}

/// Request payload to resolve an interaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolveInteractionRequest {
    pub decision: InteractionDecision,
    pub feedback: Option<String>,
}

fn row_to_interaction(row: &Row) -> rusqlite::Result<Interaction> {
    let risk_str: String = row.get("risk_level")?;
    let status_str: String = row.get("status")?;
    let decision_str: Option<String> = row.get("decision")?;

    Ok(Interaction {
        id: row.get("id")?,
        session_id: row.get("session_id")?,
        tool: row.get("tool")?,
        kind: row.get("kind")?,
        detail: row.get("detail")?,
        risk_level: RiskLevel::from_str(&risk_str).unwrap_or(RiskLevel::Medium),
        status: InteractionStatus::from_str(&status_str).unwrap_or(InteractionStatus::Pending),
        decision: decision_str.as_deref().and_then(InteractionDecision::from_str),
        feedback: row.get("feedback")?,
        created_ms: row.get("created_ms")?,
        resolved_ms: row.get("resolved_ms")?,
    })
}

/// Coordinates interaction creation, listing, resolution, and non-blocking notification.
#[derive(Clone)]
pub struct InteractionManager {
    conn: Arc<Mutex<Connection>>,
    waiters: Arc<(Mutex<HashMap<String, Interaction>>, Condvar)>,
}

impl InteractionManager {
    /// Initialize with an SQLite connection, ensuring tables exist.
    pub fn new(conn: Arc<Mutex<Connection>>) -> Result<Self> {
        {
            let c = conn.lock().unwrap();
            c.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS interaction (
                    id          TEXT PRIMARY KEY,
                    session_id  INTEGER,
                    tool        TEXT NOT NULL,
                    kind        TEXT NOT NULL,
                    detail      TEXT NOT NULL,
                    risk_level  TEXT NOT NULL,
                    status      TEXT NOT NULL,
                    decision    TEXT,
                    feedback    TEXT,
                    created_ms  INTEGER NOT NULL,
                    resolved_ms INTEGER
                );
                CREATE INDEX IF NOT EXISTS idx_interaction_status ON interaction(status);
                CREATE INDEX IF NOT EXISTS idx_interaction_session ON interaction(session_id);
                "#,
            )?;
        }
        Ok(Self {
            conn,
            waiters: Arc::new((Mutex::new(HashMap::new()), Condvar::new())),
        })
    }

    /// Create a new interaction in `pending` state.
    pub fn create(&self, req: CreateInteractionRequest) -> Result<Interaction> {
        let created = now_ms();
        let random_suffix = fastrand::u32(100000..999999);
        let id = format!("act_{created}_{random_suffix}");
        let risk = req.risk_level.unwrap_or(RiskLevel::Medium);
        let status = InteractionStatus::Pending;

        let interaction = Interaction {
            id: id.clone(),
            session_id: req.session_id,
            tool: req.tool,
            kind: req.kind,
            detail: req.detail,
            risk_level: risk,
            status,
            decision: None,
            feedback: None,
            created_ms: created,
            resolved_ms: None,
        };

        {
            let c = self.conn.lock().unwrap();
            c.execute(
                "INSERT INTO interaction (id, session_id, tool, kind, detail, risk_level, status, decision, feedback, created_ms, resolved_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    interaction.id,
                    interaction.session_id,
                    interaction.tool,
                    interaction.kind,
                    interaction.detail,
                    interaction.risk_level.as_str(),
                    interaction.status.as_str(),
                    Option::<&str>::None,
                    Option::<&str>::None,
                    interaction.created_ms,
                    Option::<i64>::None,
                ],
            )?;
        }

        Ok(interaction)
    }

    /// Retrieve an interaction by ID.
    pub fn get(&self, id: &str) -> Result<Option<Interaction>> {
        let c = self.conn.lock().unwrap();
        let mut stmt = c.prepare(
            "SELECT id, session_id, tool, kind, detail, risk_level, status, decision, feedback, created_ms, resolved_ms
             FROM interaction WHERE id = ?1",
        )?;
        let row = stmt.query_row([id], row_to_interaction).optional()?;
        Ok(row)
    }

    /// List interactions, optionally filtered by status or session ID.
    pub fn list(
        &self,
        status: Option<InteractionStatus>,
        session_id: Option<i64>,
    ) -> Result<Vec<Interaction>> {
        let c = self.conn.lock().unwrap();
        let mut query = String::from(
            "SELECT id, session_id, tool, kind, detail, risk_level, status, decision, feedback, created_ms, resolved_ms
             FROM interaction WHERE 1=1",
        );
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(st) = status {
            query.push_str(" AND status = ?");
            params.push(Box::new(st.as_str().to_string()));
        }

        if let Some(sess) = session_id {
            query.push_str(" AND session_id = ?");
            params.push(Box::new(sess));
        }

        query.push_str(" ORDER BY created_ms DESC");

        let mut stmt = c.prepare(&query)?;
        let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|b| b.as_ref()).collect();
        let rows = stmt
            .query_map(param_refs.as_slice(), row_to_interaction)?
            .collect::<std::result::Result<Vec<Interaction>, _>>()?;
        Ok(rows)
    }

    /// Resolve an interaction with a decision (allow / deny / reply).
    pub fn resolve(&self, id: &str, req: ResolveInteractionRequest) -> Result<Interaction> {
        let existing = self
            .get(id)?
            .ok_or_else(|| DaemonError::InteractionNotFound(id.to_string()))?;

        if existing.status != InteractionStatus::Pending {
            return Err(DaemonError::InteractionAlreadyResolved(id.to_string()));
        }

        let new_status = match req.decision {
            InteractionDecision::Allow => InteractionStatus::Allowed,
            InteractionDecision::Deny => InteractionStatus::Denied,
            InteractionDecision::Reply => InteractionStatus::Allowed,
        };
        let resolved_at = now_ms();

        {
            let c = self.conn.lock().unwrap();
            c.execute(
                "UPDATE interaction SET status = ?1, decision = ?2, feedback = ?3, resolved_ms = ?4
                 WHERE id = ?5 AND status = 'pending'",
                params![
                    new_status.as_str(),
                    req.decision.as_str(),
                    req.feedback,
                    resolved_at,
                    id,
                ],
            )?;
        }

        let resolved = self
            .get(id)?
            .ok_or_else(|| DaemonError::InteractionNotFound(id.to_string()))?;

        // Wake in-process wait handles
        let (lock, cvar) = &*self.waiters;
        {
            let mut resolved_map = lock.lock().unwrap();
            resolved_map.insert(id.to_string(), resolved.clone());
        }
        cvar.notify_all();

        Ok(resolved)
    }

    /// Wait non-blockingly (or with timeout) for an interaction to be resolved.
    /// Useful for in-process turn execution that suspends waiting on human approval.
    pub fn wait_for_resolution(
        &self,
        id: &str,
        timeout: Option<Duration>,
    ) -> Result<Interaction> {
        // First check if already resolved in DB
        if let Some(cur) = self.get(id)? {
            if cur.status != InteractionStatus::Pending {
                return Ok(cur);
            }
        } else {
            return Err(DaemonError::InteractionNotFound(id.to_string()));
        }

        let (lock, cvar) = &*self.waiters;
        let mut resolved_map = lock.lock().unwrap();

        let start = Instant::now();
        loop {
            if let Some(resolved) = resolved_map.get(id) {
                return Ok(resolved.clone());
            }

            match timeout {
                Some(dur) => {
                    let elapsed = start.elapsed();
                    if elapsed >= dur {
                        return Err(DaemonError::Timeout(format!(
                            "timed out waiting for resolution of interaction {id}"
                        )));
                    }
                    let remaining = dur - elapsed;
                    let result = cvar.wait_timeout(resolved_map, remaining).unwrap();
                    resolved_map = result.0;
                    if result.1.timed_out() && !resolved_map.contains_key(id) {
                        return Err(DaemonError::Timeout(format!(
                            "timed out waiting for resolution of interaction {id}"
                        )));
                    }
                }
                None => {
                    resolved_map = cvar.wait(resolved_map).unwrap();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interaction_state_machine_workflow() {
        let conn = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let manager = InteractionManager::new(conn).unwrap();

        // 1. Create interaction
        let req = CreateInteractionRequest {
            session_id: Some(10),
            tool: "bash".to_string(),
            kind: "execute".to_string(),
            detail: "git push".to_string(),
            risk_level: Some(RiskLevel::High),
        };
        let created = manager.create(req).unwrap();
        assert_eq!(created.status, InteractionStatus::Pending);
        assert_eq!(created.risk_level, RiskLevel::High);

        // 2. List pending
        let pending = manager.list(Some(InteractionStatus::Pending), None).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, created.id);

        // 3. Resolve interaction
        let resolve_req = ResolveInteractionRequest {
            decision: InteractionDecision::Allow,
            feedback: Some("Approved by user".to_string()),
        };
        let resolved = manager.resolve(&created.id, resolve_req.clone()).unwrap();
        assert_eq!(resolved.status, InteractionStatus::Allowed);
        assert_eq!(resolved.decision, Some(InteractionDecision::Allow));
        assert_eq!(resolved.feedback.as_deref(), Some("Approved by user"));

        // 4. Double resolve fails
        let err = manager.resolve(&created.id, resolve_req);
        assert!(matches!(err, Err(DaemonError::InteractionAlreadyResolved(_))));
    }

    #[test]
    fn in_process_waiter_wakes_on_resolution() {
        let conn = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let manager = InteractionManager::new(conn).unwrap();

        let created = manager
            .create(CreateInteractionRequest {
                session_id: None,
                tool: "file_write".to_string(),
                kind: "write".to_string(),
                detail: "overwrite config".to_string(),
                risk_level: Some(RiskLevel::Medium),
            })
            .unwrap();

        let m_clone = manager.clone();
        let id = created.id.clone();
        let waiter = std::thread::spawn(move || {
            m_clone.wait_for_resolution(&id, Some(Duration::from_secs(3))).unwrap()
        });

        std::thread::sleep(Duration::from_millis(100));

        manager
            .resolve(
                &created.id,
                ResolveInteractionRequest {
                    decision: InteractionDecision::Deny,
                    feedback: Some("Denied".to_string()),
                },
            )
            .unwrap();

        let result = waiter.join().unwrap();
        assert_eq!(result.status, InteractionStatus::Denied);
        assert_eq!(result.decision, Some(InteractionDecision::Deny));
    }
}
