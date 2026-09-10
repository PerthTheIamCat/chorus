//! The budget allocator: turns system/task-state/memory/conversation/code
//! inputs into one assembled prompt string, trimming lower-priority tiers
//! first when the whole thing doesn't fit the window.
//!
//! Nothing here reads or writes storage — see the crate-level doc comment.
//! `Assembler::assemble` is meant to be called fresh on every turn; its
//! output is never persisted.
use crate::render::{self, ContextItem};
use crate::tokens::TokenCounter;

/// A context tier, in priority order (earlier tiers are trimmed last).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    System,
    TaskState,
    Memory,
    Conversation,
    Code,
    Reserve,
}

/// Fraction of the total window each tier is capped at. `System` is not a
/// fraction: it's a fixed, caller-supplied amount that's never trimmed, so
/// its "cap" is simply the whole budget (the cap is never enforced against
/// it in practice).
const TASK_STATE_FRAC: f64 = 0.05;
const MEMORY_FRAC: f64 = 0.09;
const CONVERSATION_FRAC: f64 = 0.38;
const CODE_FRAC: f64 = 0.26;
const RESERVE_FRAC: f64 = 0.16;

fn frac_of(total: i64, frac: f64) -> i64 {
    ((total as f64) * frac).floor() as i64
}

/// The total token window and the per-tier caps carved out of it.
pub struct Budget {
    pub total: i64,
}

impl Budget {
    pub fn new(total: i64) -> Self {
        Self { total }
    }

    /// The token cap for one tier. `System` is fixed/caller-supplied and
    /// never trimmed, so its cap is the whole window (not a real
    /// constraint — see the module doc comment).
    pub fn cap(&self, tier: Tier) -> i64 {
        match tier {
            Tier::System => self.total,
            Tier::TaskState => frac_of(self.total, TASK_STATE_FRAC),
            Tier::Memory => frac_of(self.total, MEMORY_FRAC),
            Tier::Conversation => frac_of(self.total, CONVERSATION_FRAC),
            Tier::Code => frac_of(self.total, CODE_FRAC),
            Tier::Reserve => frac_of(self.total, RESERVE_FRAC),
        }
    }
}

/// Rung ages: turns younger than this many turns-ago render at L0 (raw).
const L0_MAX_AGE: usize = 10;
/// Turns at or beyond this age render at L2 (episode summary) by default;
/// turns in between render at L1 (per-turn summary).
const L2_MIN_AGE: usize = 40;

/// One conversational turn, carried at up to three compaction rungs.
/// `age` 0 is the newest turn; larger is older.
#[derive(Debug, Clone)]
pub struct TurnItem {
    pub text_l0: String,
    pub text_l1: Option<String>,
    pub text_l2: Option<String>,
    pub age: usize,
    pub key: Option<String>,
}

fn base_rung(age: usize) -> u8 {
    if age < L0_MAX_AGE {
        0
    } else if age < L2_MIN_AGE {
        1
    } else {
        2
    }
}

/// The text a turn renders as at a given rung, falling back toward L0 (the
/// only rung guaranteed to be supplied) when the caller didn't provide the
/// requested one.
fn rung_text(turn: &TurnItem, rung: u8) -> String {
    match rung {
        0 => turn.text_l0.clone(),
        1 => turn
            .text_l1
            .clone()
            .unwrap_or_else(|| turn.text_l0.clone()),
        _ => turn
            .text_l2
            .clone()
            .or_else(|| turn.text_l1.clone())
            .unwrap_or_else(|| turn.text_l0.clone()),
    }
}

/// Everything the assembler needs for one call. Nothing here is stored —
/// it's assembled fresh and discarded.
pub struct AssembleInput {
    pub system: String,
    pub task_state: String,
    /// Pre-ranked, highest-priority first. On overflow the lowest-ranked
    /// (tail) items are dropped first.
    pub memory: Vec<ContextItem>,
    pub turns: Vec<TurnItem>,
    pub code: Vec<ContextItem>,
}

/// The result of one `assemble` call.
pub struct Assembled {
    pub text: String,
    pub per_tier: Vec<(Tier, i64)>,
    pub dropped_turns: usize,
    pub demoted_turns: usize,
}

pub struct Assembler<C: TokenCounter> {
    budget: Budget,
    counter: C,
}

impl<C: TokenCounter> Assembler<C> {
    pub fn new(budget: Budget, counter: C) -> Self {
        Self { budget, counter }
    }

    pub fn assemble(&self, input: &AssembleInput) -> Assembled {
        let mut per_tier = Vec::with_capacity(6);
        let mut sections: Vec<(Tier, String)> = Vec::with_capacity(6);

        // System — fixed, caller-supplied, never trimmed.
        let system_tokens = self.counter.count(&input.system);
        per_tier.push((Tier::System, system_tokens));
        sections.push((Tier::System, input.system.clone()));

        // TaskState — never trimmed: goal/done/pending outranks everything
        // below it, so it goes in whole regardless of its 5% cap.
        let task_tokens = self.counter.count(&input.task_state);
        per_tier.push((Tier::TaskState, task_tokens));
        sections.push((Tier::TaskState, input.task_state.clone()));

        // Memory — drop lowest-ranked items on overflow.
        let memory_cap = self.budget.cap(Tier::Memory);
        let (memory_text, memory_tokens) = self.assemble_memory(&input.memory, memory_cap);
        per_tier.push((Tier::Memory, memory_tokens));
        sections.push((Tier::Memory, memory_text));

        // Conversation — step turns down the rungs, oldest first, before
        // dropping any.
        let conv_cap = self.budget.cap(Tier::Conversation);
        let (conv_text, conv_tokens, dropped_turns, demoted_turns) =
            self.assemble_conversation(&input.turns, conv_cap);
        per_tier.push((Tier::Conversation, conv_tokens));
        sections.push((Tier::Conversation, conv_text));

        // Code — dedup identical content first, then replace bodies with a
        // manifest line on overflow, largest first.
        let code_cap = self.budget.cap(Tier::Code);
        let mut code_items = input.code.clone();
        let (code_text, code_tokens) = self.assemble_code(&mut code_items, code_cap);
        per_tier.push((Tier::Code, code_tokens));
        sections.push((Tier::Code, code_text));

        // Reserve — untouched headroom for the model's reply. Nothing is
        // ever placed here.
        per_tier.push((Tier::Reserve, 0));

        let mut text = String::new();
        for (tier, body) in &sections {
            if body.trim().is_empty() {
                continue;
            }
            text.push_str(&format!("# {tier:?}\n"));
            text.push_str(body);
            text.push_str("\n\n");
        }
        let text = text.trim_end().to_string();

        Assembled {
            text,
            per_tier,
            dropped_turns,
            demoted_turns,
        }
    }

    fn assemble_memory(&self, items: &[ContextItem], cap: i64) -> (String, i64) {
        let mut kept: Vec<&ContextItem> = items.iter().collect();
        let mut sizes: Vec<i64> = kept.iter().map(|i| self.counter.count(&i.text)).collect();
        let mut total: i64 = sizes.iter().sum();
        while total > cap && !kept.is_empty() {
            // Lowest-ranked item is the last one (caller supplies items
            // pre-ranked, highest priority first).
            kept.pop();
            let removed = sizes.pop().unwrap();
            total -= removed;
        }
        let text = kept
            .iter()
            .map(|i| i.text.clone())
            .collect::<Vec<_>>()
            .join("\n");
        (text, total)
    }

    fn assemble_conversation(
        &self,
        turns: &[TurnItem],
        cap: i64,
    ) -> (String, i64, usize, usize) {
        struct State {
            turn: TurnItem,
            rung: u8,
            dropped: bool,
            demoted: bool,
        }

        let mut states: Vec<State> = turns
            .iter()
            .map(|t| State {
                turn: t.clone(),
                rung: base_rung(t.age),
                dropped: false,
                demoted: false,
            })
            .collect();

        let total_of = |states: &[State], counter: &C| -> i64 {
            states
                .iter()
                .filter(|s| !s.dropped)
                .map(|s| counter.count(&rung_text(&s.turn, s.rung)))
                .sum()
        };

        loop {
            let total = total_of(&states, &self.counter);
            if total <= cap {
                break;
            }
            // Step the oldest not-fully-demoted turn down one rung.
            let demote_idx = states
                .iter()
                .enumerate()
                .filter(|(_, s)| !s.dropped && s.rung < 2)
                .max_by_key(|(_, s)| s.turn.age)
                .map(|(i, _)| i);

            if let Some(i) = demote_idx {
                states[i].rung += 1;
                states[i].demoted = true;
                continue;
            }

            // Everything left is already fully demoted (or nothing is
            // left to demote): drop the oldest remaining turn.
            let drop_idx = states
                .iter()
                .enumerate()
                .filter(|(_, s)| !s.dropped)
                .max_by_key(|(_, s)| s.turn.age)
                .map(|(i, _)| i);

            match drop_idx {
                Some(i) => states[i].dropped = true,
                None => break, // nothing left at all
            }
        }

        let demoted_turns = states.iter().filter(|s| s.demoted).count();
        let dropped_turns = states.iter().filter(|s| s.dropped).count();

        // Render chronologically: oldest first, newest last.
        let mut ordered: Vec<&State> = states.iter().filter(|s| !s.dropped).collect();
        ordered.sort_by(|a, b| b.turn.age.cmp(&a.turn.age));

        let mut text = String::new();
        for s in &ordered {
            text.push_str(&rung_text(&s.turn, s.rung));
            text.push('\n');
        }
        if text.ends_with('\n') {
            text.pop();
        }

        let used = total_of(&states, &self.counter);
        (text, used, dropped_turns, demoted_turns)
    }

    fn assemble_code(&self, items: &mut Vec<ContextItem>, cap: i64) -> (String, i64) {
        render::dedup_window(items);

        let mut sizes: Vec<i64> = items.iter().map(|i| self.counter.count(&i.text)).collect();
        let mut total: i64 = sizes.iter().sum();

        if total > cap {
            let mut order: Vec<usize> = (0..items.len()).collect();
            order.sort_by_key(|&i| std::cmp::Reverse(sizes[i]));
            for i in order {
                if total <= cap {
                    break;
                }
                let replacement =
                    format!("[{}: content omitted — see manifest, over budget]", items[i].label);
                let new_size = self.counter.count(&replacement);
                total = total - sizes[i] + new_size;
                items[i].text = replacement;
                sizes[i] = new_size;
            }
        }

        let text = items
            .iter()
            .map(|i| i.text.clone())
            .collect::<Vec<_>>()
            .join("\n\n");
        (text, total)
    }
}
