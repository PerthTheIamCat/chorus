//! Pure tests for the context-assembly layer. No database, no storage
//! modules touched — only tokens/render/context.

use chorus_core::context::{AssembleInput, Assembler, Budget, Tier, TurnItem};
use chorus_core::render::{self, ContextItem, FileChange};
use chorus_core::tokens::{CachedCounter, HeuristicCounter, TokenCounter};

/// Thai text should be estimated at meaningfully more tokens than English
/// text of the same *byte* length — the len/4 bug this counter exists to
/// avoid would instead undercount Thai because Thai packs 3 bytes/char.
#[test]
fn thai_estimated_denser_than_english_same_byte_length() {
    let counter = HeuristicCounter;

    // Thai sentence, no spaces between words (as Thai actually is).
    let thai = "สวัสดีครับวันนี้อากาศดีมากและผมอยากไปเที่ยวทะเลกับเพื่อนสนิทของผม";
    // English filler of comparable *byte* length.
    let english: String = {
        let base = "The weather today is very nice and I want to go to the beach. ";
        let mut s = String::new();
        while s.len() < thai.len() {
            s.push_str(base);
        }
        s.truncate(thai.len());
        s
    };

    assert!(
        (thai.len() as i64 - english.len() as i64).abs() <= 4,
        "test setup: byte lengths should be close ({} vs {})",
        thai.len(),
        english.len()
    );

    let thai_tokens = counter.count(thai);
    let english_tokens = counter.count(&english);

    // len/4 would give roughly the same estimate for both (same byte
    // length); a correct per-character-class estimate must diverge
    // substantially because Thai is 3 bytes/char and English is ~1.
    assert!(
        thai_tokens > english_tokens,
        "expected thai ({thai_tokens}) to cost more tokens than english ({english_tokens}) of the same byte length"
    );

    // Sanity: naive len/4 on bytes would produce roughly len/4 tokens for
    // both. Assert the Thai estimate is clearly above that naive baseline,
    // proving we are not secretly doing len/4.
    let naive = thai.len() as i64 / 4;
    assert!(
        (thai_tokens as f64) > (naive as f64) * 1.2,
        "thai estimate ({thai_tokens}) should be meaningfully above the len/4 baseline ({naive})"
    );
}

#[test]
fn cached_counter_reports_hits_on_repeat() {
    let cached = CachedCounter::new(HeuristicCounter);
    let text = "the quick brown fox jumps over the lazy dog";

    assert_eq!(cached.hits(), 0);
    assert_eq!(cached.misses(), 0);

    let first = cached.count(text);
    assert_eq!(cached.misses(), 1);
    assert_eq!(cached.hits(), 0);

    let second = cached.count(text);
    assert_eq!(first, second);
    assert_eq!(cached.misses(), 1);
    assert_eq!(cached.hits(), 1);

    // Different content is still a miss.
    cached.count("something else entirely");
    assert_eq!(cached.misses(), 2);
    assert_eq!(cached.hits(), 1);
}

#[test]
fn table_has_one_header_and_n_rows_and_beats_json() {
    let header = ["id", "name", "status"];
    let rows: Vec<Vec<String>> = (0..10)
        .map(|i| vec![i.to_string(), format!("item-{i}"), "ok".to_string()])
        .collect();

    let rendered = render::table(&header, &rows);
    let lines: Vec<&str> = rendered.lines().collect();

    // Exactly one header line + one line per row.
    assert_eq!(lines.len(), 1 + rows.len());
    assert!(lines[0].starts_with('('));
    assert!(lines[0].contains("id | name | status"));
    for (i, line) in lines[1..].iter().enumerate() {
        assert_eq!(*line, format!("{i} | item-{i} | ok"));
    }

    // Equivalent JSON array-of-objects for the same 10 rows.
    let json_rows: Vec<serde_json::Value> = (0..10)
        .map(|i| {
            serde_json::json!({
                "id": i.to_string(),
                "name": format!("item-{i}"),
                "status": "ok",
            })
        })
        .collect();
    let json = serde_json::to_string(&json_rows).unwrap();

    // The table form must be materially shorter than JSON: assert it's at
    // most 60% of the JSON's length.
    let ratio = rendered.len() as f64 / json.len() as f64;
    assert!(
        ratio < 0.6,
        "table ({} bytes) should be materially shorter than JSON ({} bytes), ratio={ratio}",
        rendered.len(),
        json.len()
    );
}

#[test]
fn table_sanitises_pipes_and_newlines() {
    let header = ["a", "b"];
    let rows = vec![vec!["x|y".to_string(), "line1\nline2".to_string()]];
    let rendered = render::table(&header, &rows);
    let data_line = rendered.lines().nth(1).unwrap();
    assert!(!data_line.contains('\n'));
    // The cell's own pipe must not be mistaken for a column separator: the
    // row must still split into exactly 2 columns.
    let cols: Vec<&str> = data_line.split(" | ").collect();
    assert_eq!(cols.len(), 2);
}

#[test]
fn file_manifest_builds_on_table() {
    let files = vec![
        FileChange { path: "src/a.rs".into(), status: "M".into(), added: 3, removed: 1 },
        FileChange { path: "src/b.rs".into(), status: "A".into(), added: 10, removed: 0 },
    ];
    let manifest = render::file_manifest(&files);
    assert!(manifest.contains("src/a.rs"));
    assert!(manifest.contains("src/b.rs"));
    assert_eq!(manifest.lines().count(), 3); // header + 2 rows
}

#[test]
fn truncate_middle_keeps_head_and_tail_and_noop_when_short() {
    let lines: Vec<String> = (0..20).map(|i| format!("line{i}")).collect();
    let text = lines.join("\n");

    let truncated = render::truncate_middle(&text, 3, 3);
    let out_lines: Vec<&str> = truncated.lines().collect();

    assert_eq!(out_lines[0], "line0");
    assert_eq!(out_lines[1], "line1");
    assert_eq!(out_lines[2], "line2");
    assert_eq!(out_lines[out_lines.len() - 3], "line17");
    assert_eq!(out_lines[out_lines.len() - 2], "line18");
    assert_eq!(out_lines[out_lines.len() - 1], "line19");
    // A marker line naming how many were dropped (20 - 3 - 3 = 14).
    assert!(truncated.contains("14"));

    // No-op when already short enough.
    let short = "a\nb\nc";
    assert_eq!(render::truncate_middle(short, 5, 5), short);
    // Exactly head+tail lines: still a no-op (nothing in the middle).
    let exact = "a\nb\nc\nd";
    assert_eq!(render::truncate_middle(exact, 2, 2), exact);
}

#[test]
fn dedup_window_keeps_first_collapses_rest() {
    let mut items = vec![
        ContextItem { text: "full body A".into(), key: Some("k1".into()), label: "a.rs".into() },
        ContextItem { text: "full body B".into(), key: Some("k2".into()), label: "b.rs".into() },
        ContextItem { text: "full body A again".into(), key: Some("k1".into()), label: "a.rs".into() },
        ContextItem { text: "unkeyed".into(), key: None, label: "c.rs".into() },
        ContextItem { text: "full body A third".into(), key: Some("k1".into()), label: "a.rs".into() },
    ];

    render::dedup_window(&mut items);

    assert_eq!(items[0].text, "full body A"); // first occurrence kept verbatim
    assert_eq!(items[1].text, "full body B"); // distinct key, kept
    assert_eq!(items[2].text, render::repeat_ref("a.rs")); // collapsed
    assert_eq!(items[3].text, "unkeyed"); // no key, never touched
    assert_eq!(items[4].text, render::repeat_ref("a.rs")); // collapsed
}

fn make_turn(age: usize) -> TurnItem {
    TurnItem {
        text_l0: format!("turn {age} full raw text ").repeat(20),
        text_l1: Some(format!("turn {age} summary")),
        text_l2: Some(format!("turn {age} episode summary, a bit longer than l1")),
        age,
        key: None,
    }
}

#[test]
fn tight_budget_demotes_before_dropping_and_respects_caps() {
    // 50 turns, ages 0..49 (0 = newest). With a generous budget everything
    // fits at L0/L1/L2 per the age ladder; with a tight one, old turns must
    // be demoted down the rungs before any turn is dropped outright.
    let turns: Vec<TurnItem> = (0..50).map(make_turn).collect();

    let counter = HeuristicCounter;
    // Compute what a fully-demoted (best case) conversation would cost, to
    // pick a cap that forces demotion but not total collapse.
    let fully_demoted_tokens: i64 = turns
        .iter()
        .map(|t| {
            counter.count(
                t.text_l2
                    .as_deref()
                    .or(t.text_l1.as_deref())
                    .unwrap_or(&t.text_l0),
            )
        })
        .sum();

    // Pick a total window whose Conversation cap sits comfortably above the
    // fully-demoted cost (so demotion alone can succeed) but whose raw
    // (all-L0) cost overflows it (so trimming is actually exercised).
    let conversation_frac = 0.38; // must track context::CONVERSATION_FRAC
    let total = (((fully_demoted_tokens + 100) as f64 / conversation_frac).ceil()) as i64;
    let budget = Budget::new(total);
    let cap = budget.cap(Tier::Conversation);
    let raw_tokens: i64 = turns.iter().map(|t| counter.count(&t.text_l0)).sum();
    assert!(raw_tokens > cap, "test setup: raw turns should overflow the cap");
    assert!(
        fully_demoted_tokens < cap,
        "test setup: fully demoted turns ({fully_demoted_tokens}) should fit the cap ({cap})"
    );

    let assembler = Assembler::new(Budget::new(total), HeuristicCounter);
    let input = AssembleInput {
        system: String::new(),
        task_state: String::new(),
        memory: vec![],
        turns: turns.clone(),
        code: vec![],
    };
    let assembled = assembler.assemble(&input);

    assert_eq!(assembled.dropped_turns, 0, "demotion alone should have been enough");
    assert!(assembled.demoted_turns > 0, "some old turns should have been demoted");

    for (tier, used) in &assembled.per_tier {
        let cap = budget.cap(*tier);
        assert!(
            *used <= cap,
            "tier {tier:?} used {used} tokens, exceeding its cap of {cap}"
        );
    }
}

#[test]
fn task_state_survives_when_conversation_is_emptied() {
    // A budget tiny enough that Conversation's 38% share can't hold even
    // one fully-demoted turn, but TaskState (never trimmed) must still be
    // present in full.
    let task_state = "goal: ship the release\ndone: 3/5\npending: tests, docs";
    let turns: Vec<TurnItem> = (0..50).map(make_turn).collect();

    let assembler = Assembler::new(Budget::new(20), HeuristicCounter);
    let input = AssembleInput {
        system: "sys".into(),
        task_state: task_state.into(),
        memory: vec![],
        turns,
        code: vec![],
    };
    let assembled = assembler.assemble(&input);

    assert!(assembled.text.contains(task_state));
    assert!(assembled.dropped_turns > 0, "conversation should have been emptied out");

    let conv_used = assembled
        .per_tier
        .iter()
        .find(|(t, _)| *t == Tier::Conversation)
        .unwrap()
        .1;
    assert_eq!(conv_used, 0);

    let task_used = assembled
        .per_tier
        .iter()
        .find(|(t, _)| *t == Tier::TaskState)
        .unwrap()
        .1;
    assert!(task_used > 0);
}
