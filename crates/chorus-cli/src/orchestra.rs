//! Who runs what in orchestra mode.
//!
//! Two decisions live here, both pure so they can be tested without a bridge:
//!
//! - **A child's agent, model and effort.** The orchestrator only names
//!   `role`, `task` and `weight`; the router picks from the agents that are
//!   ready right now, under the session's route preset. A role file's
//!   `model:` pins the model instead, on whichever agent actually offers it.
//! - **The orchestrator's own chain.** Its Settings model (default Claude
//!   Opus at medium effort) first, then a backup from another provider, so a
//!   limit relays with the usual handoff brief.
//!
//! Neither touches Swift: the shell passes the raw Settings strings and gets
//! a turn back.

use crate::agents::AgentInfo;
use crate::session::AgentSpec;
use chorus_core::router::{effort_rank, pick_for_weight, Candidate, RoutePreset, TaskKind};

/// The ACP provider label for an agent name (accounts allowed: `claude@work`).
pub fn provider_for(agent: &str) -> &'static str {
    let a = crate::accounts::split(agent).0;
    if a.contains("claude") {
        "anthropic"
    } else if a.contains("gemini") || a.contains("antigravity") || a.contains("agy") {
        "google"
    } else {
        "openai"
    }
}

/// `gpt-5.5[low]` → (`gpt-5.5`, `Some("low")`). Brackets that are not an
/// effort (a context size, `[1m]`) stay part of the model.
pub fn split_effort(id: &str) -> (&str, Option<&str>) {
    const EFFORTS: &[&str] = &["minimal", "low", "medium", "high", "xhigh", "max", "none"];
    if let (Some(open), true) = (id.rfind('['), id.ends_with(']')) {
        let e = &id[open + 1..id.len() - 1];
        if EFFORTS.contains(&e) {
            return (&id[..open], Some(e));
        }
    }
    (id, None)
}

/// One candidate per model of every ready agent, in the router's shape.
/// The id is `agent:model-id`, and the model id keeps its effort suffix,
/// because that is what `session/set_model` takes.
pub fn candidates_from(agents: &[AgentInfo]) -> Vec<Candidate> {
    let mut out = Vec::new();
    for a in agents
        .iter()
        .filter(|a| matches!(a.auth, crate::agents::AuthState::Ok))
    {
        for m in &a.models {
            let (base, effort) = split_effort(&m.id);
            out.push(Candidate {
                id: format!("{}:{}", a.id, m.id),
                agent: a.id.clone(),
                model: base.to_string(),
                effort: effort.unwrap_or("default").to_string(),
                media_types: vec!["text/*".into()],
                integrations: Vec::new(),
                ready: true,
                tier: crate::agents::model_tier(&m.id),
            });
        }
    }
    out
}

/// The full model id a candidate stands for (`gpt-5.5[low]`).
fn model_id(c: &Candidate) -> &str {
    c.id.split_once(':').map(|(_, m)| m).unwrap_or(&c.model)
}

/// What kind of work a role does, for the router's `Want` table.
pub fn kind_for_role(role: &str) -> TaskKind {
    let r = role.to_ascii_lowercase();
    if r.contains("review") || r.contains("audit") || r.contains("verif") {
        TaskKind::Review
    } else if r.contains("investig")
        || r.contains("plan")
        || r.contains("architect")
        || r.contains("research")
    {
        TaskKind::Think
    } else {
        TaskKind::Do
    }
}

/// Where one child runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildRoute {
    pub agent: String,
    /// Full model id to pin on `agent` (`gpt-5.5[low]`, `opus`).
    pub model: String,
    pub pinned: bool,
}

/// Pick a child's agent and model.
///
/// `pinned` is a role file's `model:`. It matches a ready candidate by full
/// id, or by base id preferring the agent's default effort, and runs on the
/// agent that offers it. No ready agent offering it is an error, not a
/// silent swap: a role that asked for a model must not quietly get another.
///
/// Otherwise `weight` (`light` / `normal` / `heavy`, default `normal`) maps
/// to easy / moderate / hard and the router picks under `preset`.
pub fn route_child(
    candidates: &[Candidate],
    role: &str,
    weight: Option<&str>,
    preset: RoutePreset,
    pinned: Option<&str>,
) -> anyhow::Result<ChildRoute> {
    if let Some(pin) = pinned.map(str::trim).filter(|p| !p.is_empty()) {
        let exact = candidates.iter().find(|c| model_id(c) == pin);
        let by_base = || {
            let same: Vec<&Candidate> = candidates.iter().filter(|c| c.model == pin).collect();
            same.iter()
                .copied()
                .find(|c| c.effort == "default" || c.effort == "medium")
                .or_else(|| same.first().copied())
        };
        let c = exact.or_else(by_base).ok_or_else(|| {
            anyhow::anyhow!("role {role:?} pins model {pin:?}, but no ready agent offers it")
        })?;
        return Ok(ChildRoute {
            agent: c.agent.clone(),
            model: model_id(c).to_string(),
            pinned: true,
        });
    }
    let weight = weight
        .map(str::trim)
        .filter(|w| !w.is_empty())
        .unwrap_or("normal");
    let c = pick_for_weight(candidates, kind_for_role(role), weight, preset).ok_or_else(|| {
        if candidates.is_empty() {
            anyhow::anyhow!("no agent is ready to run a child (login in Settings › Accounts)")
        } else {
            anyhow::anyhow!("unknown weight {weight:?}: use light, normal or heavy")
        }
    })?;
    Ok(ChildRoute {
        agent: c.agent.clone(),
        model: model_id(c).to_string(),
        pinned: false,
    })
}

/// The relay chain for a child: its routed agent, then one link per other
/// ready agent so a limit fails over instead of failing the child.
pub fn child_chain(route: &ChildRoute, candidates: &[Candidate]) -> Vec<AgentSpec> {
    let mut names = vec![route.agent.clone()];
    for c in candidates {
        if !names.contains(&c.agent) {
            names.push(c.agent.clone());
        }
    }
    names
        .into_iter()
        .map(|n| {
            let p = provider_for(&n);
            AgentSpec::new(n, p)
        })
        .collect()
}

/// One link of the orchestrator's chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lead {
    pub agent: String,
    pub model: String,
}

/// Default orchestrator: Claude's largest model. claude-agent-acp reports
/// aliases (`default` / `opus` / `haiku`, CLAUDE.md 7.19), never a dated id.
const DEFAULT_LEAD_AGENT: &str = "claude";
const DEFAULT_LEAD_MODEL: &str = "opus";
/// The orchestrator plans and delegates; medium effort is the decided level.
const LEAD_EFFORT: &str = "medium";

/// A model at medium effort when the agent offers that variant, else as is.
fn at_lead_effort(candidates: &[Candidate], agent: &str, base: &str) -> Option<String> {
    let of = |c: &&Candidate| c.agent == agent && c.model == base;
    candidates
        .iter()
        .filter(of)
        .find(|c| c.effort == LEAD_EFFORT)
        .or_else(|| candidates.iter().filter(of).find(|c| c.effort == "default"))
        .or_else(|| candidates.iter().find(of))
        .map(|c| model_id(c).to_string())
}

/// Resolve a Settings model string (`""`, a base id like `opus`, or a full
/// id like `gpt-5.5[high]`, optionally `agent:`-prefixed) to a ready link.
fn resolve_lead(candidates: &[Candidate], setting: &str) -> Option<Lead> {
    let s = setting.trim();
    let (agent_hint, id) = match s.split_once(':') {
        Some((a, m)) => (Some(a), m),
        None => (None, s),
    };
    let fits = |c: &&Candidate| agent_hint.is_none_or(|a| c.agent == a);
    if let Some(c) = candidates.iter().filter(fits).find(|c| model_id(c) == id) {
        return Some(Lead {
            agent: c.agent.clone(),
            model: model_id(c).to_string(),
        });
    }
    let (base, _) = split_effort(id);
    let c = candidates.iter().filter(fits).find(|c| c.model == base)?;
    Some(Lead {
        agent: c.agent.clone(),
        model: at_lead_effort(candidates, &c.agent, base)?,
    })
}

/// The strongest ready model whose agent is not `except`: highest tier,
/// then the effort closest to medium.
fn strongest_elsewhere(candidates: &[Candidate], except: &str) -> Option<Lead> {
    let tier = |c: &Candidate| c.tier.unwrap_or(1);
    let medium = effort_rank(LEAD_EFFORT);
    candidates
        .iter()
        .filter(|c| c.agent != except)
        .max_by_key(|c| {
            (
                tier(c),
                std::cmp::Reverse(effort_rank(&c.effort).abs_diff(medium)),
            )
        })
        .map(|c| Lead {
            agent: c.agent.clone(),
            model: model_id(c).to_string(),
        })
}

/// The orchestrator's relay chain: `primary` (Settings, `""` = Claude Opus
/// at medium), then `backup` (`""` = the strongest ready model from another
/// provider). A primary that is not ready is skipped, so the backup leads.
pub fn lead_chain(candidates: &[Candidate], primary: &str, backup: &str) -> Vec<Lead> {
    let first = if primary.trim().is_empty() {
        at_lead_effort(candidates, DEFAULT_LEAD_AGENT, DEFAULT_LEAD_MODEL).map(|model| Lead {
            agent: DEFAULT_LEAD_AGENT.into(),
            model,
        })
    } else {
        resolve_lead(candidates, primary)
    };
    let lead_agent = first
        .as_ref()
        .map(|l| l.agent.clone())
        .unwrap_or_else(|| DEFAULT_LEAD_AGENT.into());
    let second = if backup.trim().is_empty() {
        strongest_elsewhere(candidates, &lead_agent)
    } else {
        resolve_lead(candidates, backup)
    };
    let mut out: Vec<Lead> = first.into_iter().collect();
    if let Some(b) = second.filter(|b| out.iter().all(|l| l.agent != b.agent)) {
        out.push(b);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(agent: &str, id: &str, tier: u8) -> Candidate {
        let (base, effort) = split_effort(id);
        Candidate {
            id: format!("{agent}:{id}"),
            agent: agent.into(),
            model: base.into(),
            effort: effort.unwrap_or("default").into(),
            media_types: vec![],
            integrations: vec![],
            ready: true,
            tier: Some(tier),
        }
    }

    fn pool() -> Vec<Candidate> {
        vec![
            cand("claude", "opus", 2),
            cand("claude", "haiku", 0),
            cand("codex", "gpt-6-astra[high]", 2),
            cand("codex", "gpt-6-astra[medium]", 2),
            cand("codex", "gpt-5.6-luna[low]", 0),
            cand("codex", "gpt-5.6-luna[medium]", 0),
            cand("codex", "gpt-5.6-terra[medium]", 1),
            cand("codex", "gpt-5.6-terra[high]", 1),
        ]
    }

    #[test]
    fn weight_moves_the_pick_up_the_tiers() {
        let p = pool();
        let light = route_child(&p, "builder", Some("light"), RoutePreset::Balanced, None).unwrap();
        let heavy = route_child(&p, "builder", Some("heavy"), RoutePreset::Balanced, None).unwrap();
        assert_eq!(light.model, "gpt-5.6-luna[low]");
        assert_eq!(heavy.model, "gpt-5.6-terra[high]");
        assert!(!light.pinned);
    }

    #[test]
    fn preset_shifts_the_child_pick() {
        let p = pool();
        let eco = route_child(&p, "builder", Some("heavy"), RoutePreset::Economy, None).unwrap();
        let max = route_child(&p, "builder", Some("heavy"), RoutePreset::Max, None).unwrap();
        assert_ne!(eco.model, max.model);
        assert_eq!(crate::agents::model_tier(&max.model), Some(2), "{max:?}");
    }

    #[test]
    fn reviewer_routes_as_review_not_do() {
        let p = pool();
        let r = route_child(&p, "reviewer", Some("normal"), RoutePreset::Balanced, None).unwrap();
        let b = route_child(&p, "builder", Some("normal"), RoutePreset::Balanced, None).unwrap();
        assert_ne!(r.model, b.model, "review wants a larger model than doing");
    }

    #[test]
    fn missing_weight_is_normal_and_unknown_weight_errors() {
        let p = pool();
        let none = route_child(&p, "builder", None, RoutePreset::Balanced, None).unwrap();
        let normal =
            route_child(&p, "builder", Some("normal"), RoutePreset::Balanced, None).unwrap();
        assert_eq!(none, normal);
        assert!(route_child(&p, "builder", Some("huge"), RoutePreset::Balanced, None).is_err());
    }

    #[test]
    fn pinned_model_runs_on_the_agent_that_offers_it() {
        let p = pool();
        let r = route_child(
            &p,
            "builder",
            Some("light"),
            RoutePreset::Economy,
            Some("opus"),
        )
        .unwrap();
        assert_eq!(
            (r.agent.as_str(), r.model.as_str(), r.pinned),
            ("claude", "opus", true)
        );
        let r = route_child(&p, "x", None, RoutePreset::Balanced, Some("gpt-6-astra")).unwrap();
        assert_eq!(
            (r.agent.as_str(), r.model.as_str()),
            ("codex", "gpt-6-astra[medium]")
        );
        let r = route_child(
            &p,
            "x",
            None,
            RoutePreset::Balanced,
            Some("gpt-6-astra[high]"),
        )
        .unwrap();
        assert_eq!(r.model, "gpt-6-astra[high]");
        assert!(route_child(&p, "x", None, RoutePreset::Balanced, Some("gemini-9")).is_err());
    }

    #[test]
    fn no_ready_agent_is_an_error() {
        assert!(route_child(&[], "builder", None, RoutePreset::Balanced, None).is_err());
    }

    #[test]
    fn child_chain_leads_with_the_routed_agent() {
        let p = pool();
        let route = ChildRoute {
            agent: "codex".into(),
            model: "gpt-5.6-luna[low]".into(),
            pinned: false,
        };
        let names: Vec<String> = child_chain(&route, &p)
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(names, vec!["codex", "claude"]);
    }

    #[test]
    fn default_lead_is_claude_opus_with_a_backup_elsewhere() {
        let chain = lead_chain(&pool(), "", "");
        assert_eq!(
            chain,
            vec![
                Lead {
                    agent: "claude".into(),
                    model: "opus".into()
                },
                Lead {
                    agent: "codex".into(),
                    model: "gpt-6-astra[medium]".into()
                },
            ]
        );
    }

    #[test]
    fn lead_settings_resolve_to_real_ids() {
        let chain = lead_chain(&pool(), "gpt-6-astra", "haiku");
        assert_eq!(chain[0].model, "gpt-6-astra[medium]");
        assert_eq!(
            chain[1],
            Lead {
                agent: "claude".into(),
                model: "haiku".into()
            }
        );
    }

    #[test]
    fn lead_that_is_not_ready_lets_the_backup_lead() {
        let codex_only: Vec<Candidate> =
            pool().into_iter().filter(|c| c.agent == "codex").collect();
        let chain = lead_chain(&codex_only, "", "");
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].agent, "codex");
    }

    #[test]
    fn effort_suffix_is_split_only_for_efforts() {
        assert_eq!(split_effort("gpt-5.5[low]"), ("gpt-5.5", Some("low")));
        assert_eq!(split_effort("opus[1m]"), ("opus[1m]", None));
        assert_eq!(split_effort("opus"), ("opus", None));
    }
}
