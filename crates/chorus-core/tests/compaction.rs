use chorus_core::compaction::{rungs_for, Extractive, Summarizer};

#[test]
fn short_text_is_returned_unchanged() {
    let s = Extractive;
    assert_eq!(s.summarize("all good", 100), "all good");
}

#[test]
fn a_long_single_line_is_cut_on_a_word_boundary() {
    let s = Extractive;
    let text =
        "the quick brown fox jumps over the lazy dog and then keeps running for a very long time";
    let out = s.summarize(text, 30);
    assert!(out.chars().count() <= 31, "over budget: {out:?}");
    assert!(out.ends_with('…'));
    assert!(!out.contains("runnin g"), "cut mid-word: {out:?}");
}

#[test]
fn load_bearing_lines_survive_compaction() {
    let s = Extractive;
    let text = "\
I looked at a bunch of files and thought about the architecture for a while.
The bug is in src/order/repo.ts line 88 — findAll does N+1.
Then I wrote some notes that don't really matter much at all here.
Decided: add an index in migration 0042.
More filler that is not important and just takes up space in the log.";
    let out = s.summarize(text, 140);
    assert!(out.contains("src/order/repo.ts"), "dropped the path: {out}");
    assert!(
        out.contains("0042") || out.to_lowercase().contains("decided"),
        "dropped the decision: {out}"
    );
    assert!(
        out.contains("more") && out.contains("+"),
        "should note how many lines were dropped: {out}"
    );
}

#[test]
fn l2_is_shorter_than_l1() {
    let text = "Fixed the N+1 in findAll by adding eager loading. Tests in the order \
                suite now pass. Still need to check the report page which has a similar pattern.";
    let r = rungs_for(&Extractive, text);
    assert!(r.l2.chars().count() <= r.l1.chars().count());
    assert!(r.l1.chars().count() <= 251);
    assert!(r.l2.chars().count() <= 92);
}

#[test]
fn ollama_summariser_falls_back_when_the_server_is_unreachable() {
    use chorus_core::compaction::{Ollama, Summarizer};
    // Point at a port nothing is listening on.
    let o = Ollama {
        model: "nope".into(),
        host: "127.0.0.1:1".into(),
    };
    let text = "The bug is in src/order/repo.ts line 88. \
                Decided: add an index in migration 0042. Lots of other filler text here \
                that does not matter and just takes up space in the log for a while.";
    let out = o.summarize(text, 90);
    assert!(out.chars().count() <= 92, "over budget: {out}");
    assert!(
        out.contains("repo.ts") || out.contains("0042"),
        "lost the signal: {out}"
    );
}

#[test]
fn best_is_extractive_without_the_env_var() {
    std::env::remove_var("CHORUS_SUMMARY_MODEL");
    let s = chorus_core::compaction::best();
    // Extractive returns short text unchanged.
    assert_eq!(s.summarize("all good", 100), "all good");
}
