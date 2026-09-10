//! The Session Engine spine: one app-owned session, one ledger, agents
//! called into it one turn at a time.
//!
//! This is the smallest real version of the orchestrator from the design
//! doc — enough to prove the invariant that matters: **the session is the
//! app's, not the provider's.** Every turn, whoever answers, lands in the
//! same ledger with an `actor`. Kill the agent mid-turn and the ledger is
//! still whole.

use anyhow::{Context, Result};
use chorus_acp::codex::{AgentEvent, CodexAgent, CodexConfig, Permission, PermissionAsk, TurnOutcome};
use chorus_core::handoff::{Brief, BriefInput, HandoffMeta, HandoffReason};
use chorus_core::{actor, ledger, EventKind, NewEvent, Store};
use std::path::{Path, PathBuf};

/// How to answer the agent's permission requests for a whole run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// Deny everything the sandbox escalates (the safe default).
    DenyAll,
    /// Allow everything — only for throwaway directories.
    AllowAll,
}

impl Policy {
    fn decide(self, ask: &PermissionAsk) -> Permission {
        let verb = match self {
            Policy::DenyAll => "DENY",
            Policy::AllowAll => "ALLOW",
        };
        eprintln!("  permission: {} ({}) — {verb}", ask.tool, ask.kind);
        match self {
            Policy::DenyAll => Permission::Deny,
            Policy::AllowAll => Permission::Allow,
        }
    }
}

/// What one turn wrote to the ledger.
pub struct TurnReport {
    pub user_seq: i64,
    pub agent_seqs: Vec<i64>,
    pub message: String,
    pub context_used: Option<i64>,
    pub context_window: Option<i64>,
    pub stop_reason: String,
}

/// The outcome of a `handoff`: the brief that was handed over and the
/// incoming agent's first response (its Verify echo-back).
pub struct HandoffResult {
    pub handoff_seq: i64,
    pub from: String,
    pub to: String,
    pub brief: Brief,
    pub echo_back: String,
    pub agent_seqs: Vec<i64>,
    pub stop_reason: String,
}

pub struct Session {
    store: Store,
    sess: i64,
    cwd: PathBuf,
    user: i64,
    /// The ACP agent command. Defaults to the published Codex bridge;
    /// tests point it at the in-repo mock.
    bridge: Vec<String>,
}

impl Session {
    /// Open (or create) the store and start a new session in it.
    pub fn create(db: &Path, cwd: &Path, title: &str) -> Result<Self> {
        let cwd = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
        let store = Store::open(db).context("open store")?;
        let user = actor::intern(store.conn(), "user", "human")?;
        let sess = ledger::new_session(store.conn(), title, &cwd.to_string_lossy())?;
        Ok(Self {
            store,
            sess,
            cwd,
            user,
            bridge: CodexConfig::default().command,
        })
    }

    /// Reattach to an existing session in an existing store.
    pub fn open(db: &Path, session_id: i64) -> Result<Self> {
        let store = Store::open(db).context("open store")?;
        let (cwd, exists): (String, bool) = store
            .conn()
            .query_row(
                "SELECT cwd, 1 FROM session WHERE id = ?1",
                [session_id],
                |r| Ok((r.get(0)?, true)),
            )
            .unwrap_or_default();
        anyhow::ensure!(exists, "no session {session_id} in {}", db.display());
        let user = actor::intern(store.conn(), "user", "human")?;
        Ok(Self {
            store,
            sess: session_id,
            cwd: PathBuf::from(cwd),
            user,
            bridge: CodexConfig::default().command,
        })
    }

    /// Override the agent command (for tests, or a pinned bridge install).
    pub fn with_bridge(mut self, cmd: Vec<String>) -> Self {
        self.bridge = cmd;
        self
    }

    pub fn session_id(&self) -> i64 {
        self.sess
    }

    /// Run one user turn against `agent_name`, persisting the user message
    /// and every structured thing the agent does back into the ledger.
    pub fn run_turn(
        &mut self,
        agent_name: &str,
        provider: &str,
        user_text: &str,
        policy: Policy,
    ) -> Result<TurnReport> {
        let c = self.store.conn();
        let agent_actor = actor::intern(c, agent_name, provider)?;

        let user_seq =
            ledger::append(c, &NewEvent::new(self.sess, EventKind::Msg, self.user, user_text))?;

        let outcome = self.one_turn(agent_name, user_text, policy)?;
        let agent_seqs = self.persist_outcome(&outcome, agent_actor, user_seq)?;

        Ok(TurnReport {
            user_seq,
            agent_seqs,
            message: outcome.message,
            context_used: outcome.context_used,
            context_window: outcome.context_window,
            stop_reason: outcome.stop_reason,
        })
    }

    /// Hand the live session to another agent. Runs the five-step pipeline:
    /// Freeze (the caller has already stopped the outgoing agent), Repack /
    /// Brief (here), Verify (the incoming agent echoes back its
    /// understanding), Resume (it answers `first_task`, or just the brief).
    ///
    /// A `Handoff` event is written first, so the ledger records the switch
    /// even if the incoming agent never answers.
    #[allow(clippy::too_many_arguments)]
    pub fn handoff(
        &mut self,
        to: &str,
        to_provider: &str,
        reason: HandoffReason,
        goal: &str,
        done: Vec<String>,
        pending: Vec<String>,
        constraints: Vec<String>,
        diff: String,
        policy: Policy,
    ) -> Result<HandoffResult> {
        let from = self.last_agent().unwrap_or_else(|| "unknown".into());
        let events = self.transcript()?;
        let at = events.last().map(|(e, _)| e.t).unwrap_or(0);

        let brief = Brief::build(BriefInput {
            goal: goal.to_string(),
            done,
            pending,
            constraints,
            events: &events,
            diff,
            handoff: HandoffMeta { from: from.clone(), to: to.to_string(), reason, at },
        });

        let c = self.store.conn();
        let to_actor = actor::intern(c, to, to_provider)?;
        let handoff_seq = ledger::append(
            c,
            &NewEvent::new(self.sess, EventKind::Handoff, to_actor, brief.to_json()),
        )?;

        // Verify + Resume: the incoming agent's first turn is the brief.
        let outcome = self.one_turn(to, &brief.render_markdown(), policy)?;
        let agent_seqs = self.persist_outcome(&outcome, to_actor, handoff_seq)?;

        Ok(HandoffResult {
            handoff_seq,
            from,
            to: to.to_string(),
            brief,
            echo_back: outcome.message,
            agent_seqs,
            stop_reason: outcome.stop_reason,
        })
    }

    // ---- internals --------------------------------------------------

    fn one_turn(&self, agent_name: &str, text: &str, policy: Policy) -> Result<TurnOutcome> {
        let cfg = CodexConfig { command: self.bridge.clone(), ..Default::default() };
        let mut agent =
            CodexAgent::start(&self.cwd, cfg).with_context(|| format!("start {agent_name}"))?;
        let mut decide = |ask: &PermissionAsk| policy.decide(ask);
        agent.prompt(text, &mut decide).context("agent turn")
    }

    fn persist_outcome(
        &self,
        outcome: &TurnOutcome,
        agent_actor: i64,
        parent: i64,
    ) -> Result<Vec<i64>> {
        let c = self.store.conn();
        let mut seqs = Vec::new();
        for ev in &outcome.events {
            let (kind, body) = match ev {
                AgentEvent::Message(m) => (EventKind::Msg, m.clone()),
                AgentEvent::ToolCall { title, kind } => {
                    (EventKind::ToolCall, format!("{kind}: {title}"))
                }
                AgentEvent::ToolResult { title, status } => {
                    (EventKind::ToolResult, format!("{title} → {status}"))
                }
                AgentEvent::Diff { path, old, new } => {
                    (EventKind::Diff, render_diff(path, old, new))
                }
            };
            let mut ne = NewEvent::new(self.sess, kind, agent_actor, body).parent(parent);
            if matches!(ev, AgentEvent::Message(_)) {
                ne.tok_out = outcome.context_used;
            }
            seqs.push(ledger::append(c, &ne)?);
        }
        Ok(seqs)
    }

    /// The actor name of the most recent non-user event, if any.
    fn last_agent(&self) -> Option<String> {
        let c = self.store.conn();
        let events = ledger::tail(c, self.sess, 200).ok()?;
        events
            .iter()
            .rev()
            .filter(|e| e.actor != self.user)
            .find_map(|e| actor::name(c, e.actor).ok())
    }

    /// The whole session so far, oldest first, with bodies resolved.
    pub fn transcript(&self) -> Result<Vec<(chorus_core::Event, String)>> {
        let c = self.store.conn();
        let events = ledger::tail(c, self.sess, 10_000)?;
        events
            .into_iter()
            .map(|e| {
                let body = ledger::body(c, &e)?;
                Ok((e, body))
            })
            .collect()
    }
}

/// A minimal unified diff for the ledger. Not a patch tool — just enough
/// that the change is legible and `handoff::files_touched` can find the path.
fn render_diff(path: &str, old: &str, new: &str) -> String {
    let mut s = format!("--- {path}\n+++ {path}\n");
    for line in old.lines() {
        s.push_str("- ");
        s.push_str(line);
        s.push('\n');
    }
    for line in new.lines() {
        s.push_str("+ ");
        s.push_str(line);
        s.push('\n');
    }
    s.truncate(s.trim_end().len());
    s
}
