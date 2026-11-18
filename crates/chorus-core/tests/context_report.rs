//! The inspector's report must agree with what the assembler actually did —
//! a report that describes a different context than the one sent is worse
//! than none.

use chorus_core::context::{AssembleInput, Assembler, Budget, TurnItem};
use chorus_core::render::ContextItem;
use chorus_core::tokens::HeuristicCounter;

fn turn(age: usize) -> TurnItem {
    TurnItem {
        text_l0: format!("turn {age} full raw text ").repeat(20),
        text_l1: Some(format!("turn {age} summary")),
        text_l2: Some(format!("turn {age} episode summary, a bit longer than l1")),
        age,
        key: None,
    }
}

fn item(label: &str, text: &str, key: Option<&str>) -> ContextItem {
    ContextItem {
        text: text.into(),
        key: key.map(str::to_string),
        label: label.into(),
    }
}

fn input(turns: Vec<TurnItem>, memory: Vec<ContextItem>, code: Vec<ContextItem>) -> AssembleInput {
    AssembleInput {
        system: "sys".into(),
        task_state: "goal: ship it".into(),
        memory,
        turns,
        code,
    }
}

#[test]
fn tiers_match_the_assemblers_own_numbers() {
    let a = Assembler::new(Budget::new(120_000), HeuristicCounter)
        .assemble(&input((0..12).map(turn).collect(), vec![], vec![]));
    assert_eq!(a.report.budget, 120_000);
    assert_eq!(a.report.tiers.len(), a.per_tier.len());
    for (t, r) in a.per_tier.iter().zip(&a.report.tiers) {
        assert_eq!(format!("{:?}", t.0), r.tier);
        assert_eq!(t.1, r.tokens);
    }
    assert_eq!(a.report.total_tokens, a.per_tier.iter().map(|(_, n)| n).sum::<i64>());
}

#[test]
fn a_roomy_window_reports_no_budget_losses() {
    let a = Assembler::new(Budget::new(200_000), HeuristicCounter)
        .assemble(&input((0..50).map(turn).collect(), vec![], vec![]));
    let c = &a.report.conversation;
    assert_eq!((c.demoted_by_budget, c.dropped), (0, 0));
    assert!(c.notes.is_empty(), "age-based summaries are not losses");
    // Rungs follow age alone: ten at L0, thirty at L1, ten at L2.
    assert_eq!((c.at_l0, c.at_l1, c.at_l2), (10, 30, 10));
    assert_eq!(c.turns_total, 50);
}

#[test]
fn a_tight_window_lists_what_was_demoted_then_dropped() {
    let a = Assembler::new(Budget::new(1_500), HeuristicCounter)
        .assemble(&input((0..50).map(turn).collect(), vec![], vec![]));
    let c = &a.report.conversation;
    assert_eq!(c.demoted_by_budget, a.demoted_turns);
    assert_eq!(c.dropped, a.dropped_turns);
    assert!(c.dropped > 0, "setup: this window should force drops");
    assert_eq!(c.at_l0 + c.at_l1 + c.at_l2 + c.dropped, c.turns_total);
    // Invariant 3 as the report shows it: nothing is dropped before every
    // surviving-or-dropped turn has been demoted as far as it goes.
    for n in c.notes.iter().filter(|n| n.dropped) {
        assert_eq!(n.to_rung, 2, "a dropped turn was already fully demoted");
        assert_eq!(n.tokens_after, 0);
    }
    // Newest first.
    assert!(c.notes.windows(2).all(|w| w[0].age <= w[1].age));
    assert!(c.notes.iter().filter(|n| n.dropped).all(|n| n.tokens_before > 0));
    assert!(c.notes.len() + c.notes_omitted >= c.dropped, "every dropped turn is listed");
    let conv = a.report.tiers.iter().find(|t| t.tier == "Conversation").unwrap();
    assert!(conv.note.contains("ย่อก่อนทิ้ง"));
}

#[test]
fn memory_cut_from_the_tail_is_marked_not_included() {
    let facts: Vec<ContextItem> = (0..40)
        .map(|i| item("fact", &format!("- convention number {i}: always run the focused tests first"), None))
        .collect();
    let a = Assembler::new(Budget::new(2_000), HeuristicCounter).assemble(&input(vec![], facts, vec![]));
    let m = &a.report.memory;
    assert_eq!(m.len(), 40);
    let in_n = m.iter().filter(|n| n.included).count();
    assert!(in_n > 0 && in_n < 40, "setup: some but not all fit ({in_n})");
    // Included ones are a prefix: lowest-ranked go first.
    assert!(m.iter().take(in_n).all(|n| n.included));
    assert!(m.iter().skip(in_n).all(|n| !n.included));
    let mem = a.report.tiers.iter().find(|t| t.tier == "Memory").unwrap();
    assert!(mem.note.contains("ตัด"));
    // Counters do not exist yet: the report must not pretend to know.
    assert!(m.iter().all(|n| n.referenced.is_none()));
}

#[test]
fn code_items_say_whether_they_were_repeated_or_replaced() {
    let big = "line of code\n".repeat(2_000);
    let code = vec![
        item("a.rs", "fn a() {}", Some("k1")),
        item("a.rs", "fn a() {}", Some("k1")),
        item("big.rs", &big, Some("k2")),
    ];
    let a = Assembler::new(Budget::new(4_000), HeuristicCounter).assemble(&input(vec![], vec![], code));
    let s: Vec<&str> = a.report.code.iter().map(|c| c.status.as_str()).collect();
    assert_eq!(s, ["kept", "repeat", "omitted"]);
}

#[test]
fn the_report_serialises_for_the_shell() {
    let a = Assembler::new(Budget::new(10_000), HeuristicCounter)
        .assemble(&input(vec![turn(0)], vec![item("fact", "- x", None)], vec![]));
    let v = serde_json::to_value(&a.report).unwrap();
    assert!(v["tiers"].is_array() && v["conversation"]["notes"].is_array());
    assert_eq!(v["memory"][0]["referenced"], serde_json::Value::Null);
}
