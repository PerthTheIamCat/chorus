//! Antigravity (`agy`) model ids, shared by the bridge binary and the agent
//! inventory so both read an id the same way.

const EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

/// `(base model, effort)` from either id shape agy users meet: the picker's
/// `gemini-4-argon[high]` or `agy models`' own `gemini-4-argon-high`.
pub fn split_model(id: &str) -> (&str, Option<&str>) {
    if let Some((base, rest)) = id.split_once('[') {
        let effort = rest.strip_suffix(']').unwrap_or(rest);
        return (base, (!effort.is_empty()).then_some(effort));
    }
    for e in EFFORTS {
        if let Some(base) = id.strip_suffix(e).and_then(|b| b.strip_suffix('-')) {
            if !base.is_empty() {
                return (base, Some(e));
            }
        }
    }
    (id, None)
}

/// The id Chorus shows and stores: `base[effort]`, so the picker splits it
/// into a model chip and an effort chip like Codex's.
pub fn canonical_model_id(id: &str) -> String {
    match split_model(id.trim()) {
        (base, Some(e)) => format!("{base}[{e}]"),
        (base, None) => base.to_string(),
    }
}

/// Context window for an agy model id, or `None` when unknown.
///
/// agy's stream-json has no window field. The Gemini figure is what agy
/// itself hands its statusline (`context_window.context_window_size`,
/// 1048576 for Gemini 4 Argon, seen 18 Oct 2026). Claude and GPT-OSS through
/// agy have no such reading, so they stay unknown rather than guessed.
/// `CHORUS_AGY_CONTEXT_WINDOW` overrides.
pub fn context_window(model: &str) -> Option<u64> {
    if let Some(n) = std::env::var("CHORUS_AGY_CONTEXT_WINDOW")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|n| *n > 0)
    {
        return Some(n);
    }
    let (base, _) = split_model(model);
    base.starts_with("gemini-").then_some(1_048_576)
}
