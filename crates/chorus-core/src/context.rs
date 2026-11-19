//! The budget allocator: turns system/task-state/memory/conversation/code
//! inputs into one assembled prompt string, trimming lower-priority tiers
//! first when the whole thing doesn't fit the window.
//!
//! Nothing here reads or writes storage — see the crate-level doc comment.
//! `Assembler::assemble` is meant to be called fresh on every turn; its
//! output is never persisted.
use crate::context_report::{self as rep, CodeNote, ContextReport, ConversationReport, MemoryNote, TierReport, TurnNote};
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
    /// What the assembler decided and why — see [`ContextReport`]. Built from
    /// bookkeeping the assembler already does; costs no extra model call.
    pub report: ContextReport,
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
        let (memory_text, memory_tokens, memory_notes) = self.assemble_memory(&input.memory, memory_cap);
        per_tier.push((Tier::Memory, memory_tokens));
        sections.push((Tier::Memory, memory_text));

        // Conversation — step turns down the rungs, oldest first, before
        // dropping any.
        let conv_cap = self.budget.cap(Tier::Conversation);
        let (conv_text, conv_tokens, dropped_turns, demoted_turns, conv_report) =
            self.assemble_conversation(&input.turns, conv_cap);
        per_tier.push((Tier::Conversation, conv_tokens));
        sections.push((Tier::Conversation, conv_text));

        // Code — dedup identical content first, then replace bodies with a
        // manifest line on overflow, largest first.
        let code_cap = self.budget.cap(Tier::Code);
        let mut code_items = input.code.clone();
        let (code_text, code_tokens, code_notes) = self.assemble_code(&mut code_items, code_cap);
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

        let report = self.build_report(
            &per_tier,
            memory_notes,
            conv_report,
            code_notes,
            memory_cap,
            conv_cap,
            code_cap,
        );
        Assembled {
            text,
            per_tier,
            dropped_turns,
            demoted_turns,
            report,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn build_report(
        &self,
        per_tier: &[(Tier, i64)],
        memory: Vec<MemoryNote>,
        conversation: ConversationReport,
        code: Vec<CodeNote>,
        memory_cap: i64,
        conv_cap: i64,
        code_cap: i64,
    ) -> ContextReport {
        let tokens_of = |t: Tier| per_tier.iter().find(|(x, _)| *x == t).map_or(0, |(_, n)| *n);
        let cut_memory = memory.iter().filter(|m| !m.included).count();
        let omitted_code = code.iter().filter(|c| c.status == "omitted").count();
        let repeat_code = code.iter().filter(|c| c.status == "repeat").count();
        let mut tiers = Vec::with_capacity(6);
        for (tier, cap) in [
            (Tier::System, self.budget.total),
            (Tier::TaskState, self.budget.cap(Tier::TaskState)),
            (Tier::Memory, memory_cap),
            (Tier::Conversation, conv_cap),
            (Tier::Code, code_cap),
            (Tier::Reserve, self.budget.cap(Tier::Reserve)),
        ] {
            let note = match tier {
                Tier::TaskState => "ไม่เคยถูกตัด — เข้าไปครบเสมอ".to_string(),
                Tier::Memory if cut_memory > 0 => {
                    format!("ตัด {cut_memory} ข้อที่อันดับต่ำสุด เพราะเกินเพดาน {memory_cap} token")
                }
                Tier::Conversation if conversation.dropped + conversation.demoted_by_budget > 0 => format!(
                    "เกินเพดาน {conv_cap} token: ย่อ {} เทิร์น ทิ้ง {} เทิร์น (ย่อก่อนทิ้งเสมอ)",
                    conversation.demoted_by_budget, conversation.dropped
                ),
                Tier::Code if omitted_code + repeat_code > 0 => {
                    format!("ไฟล์ซ้ำ {repeat_code} · ถูกแทนด้วยรายการ {omitted_code} เพราะเกินเพดาน {code_cap} token")
                }
                Tier::Reserve => "กันไว้ให้ agent ตอบ ไม่มีอะไรถูกใส่".to_string(),
                _ => String::new(),
            };
            tiers.push(TierReport::new(&format!("{tier:?}"), tokens_of(tier), cap, note));
        }
        let total_tokens = per_tier.iter().map(|(_, n)| *n).sum();
        ContextReport {
            budget: self.budget.total,
            total_tokens,
            tiers,
            conversation,
            memory,
            code,
        }
    }

    fn assemble_memory(&self, items: &[ContextItem], cap: i64) -> (String, i64, Vec<MemoryNote>) {
        let mut kept: Vec<&ContextItem> = items.iter().collect();
        let mut sizes: Vec<i64> = kept.iter().map(|i| self.counter.count(&i.text)).collect();
        let all_sizes = sizes.clone();
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
        // Items are cut from the tail, so the first `kept.len()` are in.
        let notes = items
            .iter()
            .enumerate()
            .map(|(i, it)| MemoryNote {
                text: it.text.clone(),
                included: i < kept.len(),
                tokens: all_sizes[i],
                ..Default::default()
            })
            .collect();
        (text, total, notes)
    }

    fn assemble_conversation(
        &self,
        turns: &[TurnItem],
        cap: i64,
    ) -> (String, i64, usize, usize, ConversationReport) {
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
        ordered.sort_by_key(|a| std::cmp::Reverse(a.turn.age));

        let mut text = String::new();
        for s in &ordered {
            text.push_str(&rung_text(&s.turn, s.rung));
            text.push('\n');
        }
        if text.ends_with('\n') {
            text.pop();
        }

        let used = total_of(&states, &self.counter);

        // The report: only turns that did not arrive at their by-age size.
        let mut report = ConversationReport {
            turns_total: states.len(),
            demoted_by_budget: demoted_turns,
            dropped: dropped_turns,
            ..Default::default()
        };
        let mut notes: Vec<TurnNote> = Vec::new();
        for s in &states {
            if !s.dropped {
                match s.rung {
                    0 => report.at_l0 += 1,
                    1 => report.at_l1 += 1,
                    _ => report.at_l2 += 1,
                }
            }
            let by_age = base_rung(s.turn.age);
            if s.dropped || s.rung > by_age {
                notes.push(TurnNote {
                    age: s.turn.age,
                    from_rung: by_age,
                    to_rung: s.rung,
                    dropped: s.dropped,
                    tokens_before: self.counter.count(&rung_text(&s.turn, by_age)),
                    tokens_after: if s.dropped {
                        0
                    } else {
                        self.counter.count(&rung_text(&s.turn, s.rung))
                    },
                    preview: rep::preview(&s.turn.text_l0, 60),
                });
            }
        }
        notes.sort_by_key(|n| n.age);
        if notes.len() > rep::NOTES_MAX {
            report.notes_omitted = notes.len() - rep::NOTES_MAX;
            notes.truncate(rep::NOTES_MAX);
        }
        report.notes = notes;
        (text, used, dropped_turns, demoted_turns, report)
    }

    fn assemble_code(&self, items: &mut [ContextItem], cap: i64) -> (String, i64, Vec<CodeNote>) {
        render::dedup_window(items);
        let repeats: Vec<bool> = items
            .iter()
            .map(|i| i.text == render::repeat_ref(&i.label))
            .collect();

        let mut sizes: Vec<i64> = items.iter().map(|i| self.counter.count(&i.text)).collect();
        let mut total: i64 = sizes.iter().sum();
        let mut omitted = vec![false; items.len()];

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
                omitted[i] = true;
            }
        }
        let notes = items
            .iter()
            .enumerate()
            .map(|(i, it)| CodeNote {
                label: it.label.clone(),
                tokens: sizes[i],
                status: if omitted[i] {
                    "omitted"
                } else if repeats[i] {
                    "repeat"
                } else {
                    "kept"
                }
                .to_string(),
            })
            .collect();

        let text = items
            .iter()
            .map(|i| i.text.clone())
            .collect::<Vec<_>>()
            .join("\n\n");
        (text, total, notes)
    }
}
