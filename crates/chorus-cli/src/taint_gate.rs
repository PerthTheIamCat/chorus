//! E2 at the permission callback: a turn that has read something from outside
//! the project does not get "ให้ทำเลย" for actions that leave the machine.
//!
//! The classification is `chorus_core::taint`; this is only the glue that
//! needs a session — who to ask, which mode the turn runs in.
//!
//! - Where the person already answers every ask (ถามก่อน), nothing changes
//!   except that an earlier "always allow" no longer covers such an ask, and
//!   the dialog is told why.
//! - Where the turn runs free ("ให้ทำเลย"), an ask that is outbound and
//!   tainted is put to the person through the same question channel as
//!   `chorus_ask_user`; with nobody to ask it is refused, not allowed.
//!
//! Limit, stated once: this sees what the agent *asks*. Claude asks for shell
//! commands and MCP tools even in "ให้ทำเลย" (we start it in `acceptEdits`);
//! Codex, started without its own sandbox because ours is the wall, asks for
//! nothing, so for Codex only the computer-use tool is gated.

use crate::session::{Permission, PermissionAsk};
use chorus_core::ask_user::{Asker, Question};
use chorus_core::computer;
use chorus_core::taint::{self, Judge};
use std::sync::Arc;

pub const ALLOW: &str = "อนุญาต";
pub const DENY: &str = "ไม่อนุญาต";

pub struct Gate {
    pub judge: Judge,
    /// The turn runs on "ให้ทำเลย": nobody answers its asks.
    pub free_hand: bool,
    pub asker: Option<Asker>,
    pub agent: String,
    pub log: bool,
}

impl Gate {
    /// Run `inner` (the turn's policy or dialog) on `ask`, then tighten.
    pub fn decide(
        &self,
        ask: &PermissionAsk,
        inner: &mut dyn FnMut(&PermissionAsk) -> Permission,
    ) -> Permission {
        let taint = self
            .judge
            .taint_of(ask.prior_tools.iter().map(|(t, k)| (t.as_str(), k.as_str())));
        let hit = taint.and_then(|t| {
            taint::outbound(&ask.tool, &ask.kind, &ask.detail).map(|what| (t.label(), what))
        });
        let Some((label, what)) = hit else {
            return inner(ask);
        };
        let mut shown = ask.clone();
        shown.tainted_by = Some(format!("{label} → {what}"));
        let decision = inner(&shown);
        if decision != Permission::Allow || !self.free_hand {
            return decision;
        }
        let allowed = self.confirm(&format!(
            "{} {} — {label}\nเทิร์นนี้อ่านของจากข้างนอกมาแล้ว ข้อความในนั้นอาจแอบสั่ง agent ให้ทำสิ่งนี้ · ให้ทำ `{what}` ไหม?\n{}",
            self.agent, what, ask.tool
        ));
        if self.log {
            eprintln!(
                "  taint: {} ({what}) — {}",
                ask.tool,
                if allowed { "ALLOW (ผู้ใช้ยืนยัน)" } else { "DENY" }
            );
        }
        if allowed {
            Permission::Allow
        } else {
            Permission::Deny
        }
    }

    fn confirm(&self, text: &str) -> bool {
        let Some(asker) = &self.asker else {
            return false;
        };
        asker(&question(&self.agent, text)).is_some_and(|a| a.trim() == ALLOW)
    }

    /// The fallback for the computer tool when the turn has no dialog of its
    /// own: a tainted turn that was set to act asks through the question
    /// channel instead of failing with "no one to ask".
    pub fn computer_asker(&self) -> Option<computer::Asker> {
        let asker = self.asker.clone()?;
        let agent = self.agent.clone();
        Some(Arc::new(move |what: &str| {
            let text = format!(
                "เทิร์นนี้อ่านของจากข้างนอกมาแล้ว — ให้ {agent} ควบคุมเครื่องต่อไหม?\n{what}"
            );
            asker(&question(&agent, &text)).is_some_and(|a| a.trim() == ALLOW)
        }))
    }
}

fn question(agent: &str, text: &str) -> Question {
    Question {
        agent: agent.to_string(),
        question: text.to_string(),
        reason: String::new(),
        options: vec![ALLOW.to_string(), DENY.to_string()],
        recommended: Some(1),
        allow_other: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Mutex;

    fn ask(tool: &str, prior: &[(&str, &str)]) -> PermissionAsk {
        PermissionAsk {
            tool: tool.into(),
            kind: "execute".into(),
            detail: String::new(),
            prior_tools: prior.iter().map(|(t, k)| (t.to_string(), k.to_string())).collect(),
            tainted_by: None,
        }
    }

    fn gate(free_hand: bool, asker: Option<Asker>) -> Gate {
        Gate {
            judge: Judge::new([PathBuf::from("/work/app")], false),
            free_hand,
            asker,
            agent: "claude".into(),
            log: false,
        }
    }

    fn answering(reply: &'static str, seen: Arc<Mutex<Vec<Question>>>) -> Asker {
        Arc::new(move |q: &Question| {
            seen.lock().unwrap().push(q.clone());
            Some(reply.to_string())
        })
    }

    const WEB: (&str, &str) = ("Fetch https://e.com", "fetch");

    #[test]
    fn a_clean_turn_is_not_touched() {
        let g = gate(true, None);
        let mut inner = |_: &PermissionAsk| Permission::Allow;
        assert_eq!(g.decide(&ask("git push origin main", &[]), &mut inner), Permission::Allow);
        assert_eq!(
            g.decide(&ask("git push origin main", &[("Read src/a.rs", "read")]), &mut inner),
            Permission::Allow
        );
    }

    #[test]
    fn a_tainted_turn_may_still_do_local_things() {
        let g = gate(true, None);
        let mut inner = |_: &PermissionAsk| Permission::Allow;
        assert_eq!(g.decide(&ask("cargo test", &[WEB]), &mut inner), Permission::Allow);
        assert_eq!(g.decide(&ask("git commit -m x", &[WEB]), &mut inner), Permission::Allow);
    }

    #[test]
    fn free_hand_push_after_reading_the_web_asks_and_obeys_the_answer() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut inner = |_: &PermissionAsk| Permission::Allow;

        let yes = gate(true, Some(answering(ALLOW, seen.clone())));
        assert_eq!(yes.decide(&ask("git push origin main", &[WEB]), &mut inner), Permission::Allow);
        let no = gate(true, Some(answering(DENY, seen.clone())));
        assert_eq!(no.decide(&ask("git push origin main", &[WEB]), &mut inner), Permission::Deny);

        let q = seen.lock().unwrap();
        assert_eq!(q.len(), 2);
        assert_eq!(q[0].options, vec![ALLOW, DENY]);
        assert!(!q[0].allow_other, "a typed answer must not count as consent");
        assert!(q[0].question.contains("git push") && q[0].question.contains("เว็บ"), "{}", q[0].question);
    }

    #[test]
    fn nobody_to_ask_means_refused_not_allowed() {
        let g = gate(true, None);
        let mut inner = |_: &PermissionAsk| Permission::Allow;
        assert_eq!(g.decide(&ask("git push origin main", &[WEB]), &mut inner), Permission::Deny);
    }

    #[test]
    fn a_dismissed_question_is_a_no() {
        let g = gate(true, Some(Arc::new(|_: &Question| None)));
        let mut inner = |_: &PermissionAsk| Permission::Allow;
        assert_eq!(g.decide(&ask("gh pr create", &[WEB]), &mut inner), Permission::Deny);
    }

    #[test]
    fn where_the_person_already_answers_the_dialog_is_not_doubled() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let g = gate(false, Some(answering(DENY, seen.clone())));
        let mut told = Vec::new();
        let mut inner = |a: &PermissionAsk| {
            told.push(a.tainted_by.clone());
            Permission::Allow
        };
        assert_eq!(g.decide(&ask("git push origin main", &[WEB]), &mut inner), Permission::Allow);
        assert!(seen.lock().unwrap().is_empty(), "no second question");
        assert!(told[0].as_deref().unwrap().contains("git push"), "the dialog is told why: {told:?}");
    }

    #[test]
    fn a_denial_by_the_policy_stands_and_asks_nothing() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let g = gate(true, Some(answering(ALLOW, seen.clone())));
        let mut inner = |_: &PermissionAsk| Permission::Deny;
        assert_eq!(g.decide(&ask("git push origin main", &[WEB]), &mut inner), Permission::Deny);
        assert!(seen.lock().unwrap().is_empty());
    }

    #[test]
    fn the_computer_fallback_asks_through_the_same_channel() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let g = gate(true, Some(answering(ALLOW, seen.clone())));
        let a = g.computer_asker().unwrap();
        assert!(a("คลิกที่ (10, 10)"));
        assert!(seen.lock().unwrap()[0].question.contains("คลิกที่"));
        assert!(gate(true, None).computer_asker().is_none());
    }
}
