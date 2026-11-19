//! Renders assembled context items into compact, model-facing text.
//!
//! Everything here produces plain `String`s meant to go straight into a
//! prompt. None of it is written back to storage — see the crate-level
//! doc comment for the hot/cold split this module exists to keep.

/// One item destined for the "code" / working-set tier of a context window.
///
/// `key` is an optional content key (e.g. a blob hash or a path+hash pair):
/// when two items in the same window share a `key`, `dedup_window` collapses
/// the later ones into a `repeat_ref` line instead of repeating the full
/// text. `label` is what the repeat-ref line names (typically a path).
#[derive(Debug, Clone)]
pub struct ContextItem {
    pub text: String,
    pub key: Option<String>,
    pub label: String,
}

/// A single file's change, for `file_manifest`.
#[derive(Debug, Clone)]
pub struct FileChange {
    pub path: String,
    pub status: String,
    pub added: i64,
    pub removed: i64,
}

/// Replace `|`, which is the column separator, and newlines, which would
/// break the one-line-per-row invariant, with visibly-safe stand-ins.
fn sanitise_cell(cell: &str) -> String {
    cell.replace('|', "/").replace('\r', " ").replace('\n', " ⏎ ")
}

/// Render `rows` as a single labeled table: the header appears once, as
/// `label (col | col | col):`, followed by one `|`-separated line per row.
///
/// This is the token-saving alternative to a JSON array of objects: instead
/// of repeating each field name once per row, the shape is stated once up
/// front and every row is data. It has to read clearly to a model with no
/// extra explanation, so the header names the columns in the same order
/// they appear on every row below it.
pub fn table(header: &[&str], rows: &[Vec<String>]) -> String {
    let mut out = String::new();
    out.push('(');
    out.push_str(&header.join(" | "));
    out.push_str("):\n");
    for row in rows {
        let cells: Vec<String> = row.iter().map(|c| sanitise_cell(c)).collect();
        out.push_str(&cells.join(" | "));
        out.push('\n');
    }
    if out.ends_with('\n') {
        out.pop();
    }
    out
}

/// Keep the first `head` and last `tail` lines of `text`, replacing
/// whatever's in between with a marker naming how many lines were dropped.
/// If `text` already has `head + tail` lines or fewer, it is returned
/// unchanged.
pub fn truncate_middle(text: &str, head_lines: usize, tail_lines: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len();
    if total <= head_lines + tail_lines {
        return text.to_string();
    }
    let dropped = total - head_lines - tail_lines;
    let mut out = String::new();
    for line in &lines[..head_lines] {
        out.push_str(line);
        out.push('\n');
    }
    out.push_str(&format!("… [{dropped} lines omitted] …\n"));
    for line in &lines[total - tail_lines..] {
        out.push_str(line);
        out.push('\n');
    }
    if out.ends_with('\n') {
        out.pop();
    }
    out
}

/// Render a file-change manifest as a `table`: path, status, lines added,
/// lines removed. This is what a `Code` tier falls back to when full file
/// bodies don't fit the budget.
pub fn file_manifest(files: &[FileChange]) -> String {
    let rows: Vec<Vec<String>> = files
        .iter()
        .map(|f| {
            vec![
                f.path.clone(),
                f.status.clone(),
                f.added.to_string(),
                f.removed.to_string(),
            ]
        })
        .collect();
    table(&["path", "status", "+", "-"], &rows)
}

/// The one-line stand-in emitted when the same blob content appears again
/// later in the same window: instead of repeating the full body, we tell
/// the model it was already shown this content, unchanged.
pub fn repeat_ref(path: &str) -> String {
    format!("[{path}: read again, unchanged]")
}

/// Deduplicate a window of `ContextItem`s in place: the first occurrence of
/// each distinct `key` is kept verbatim, and every later item with the same
/// key has its `text` replaced with a `repeat_ref` line naming its label.
/// Items with `key: None` are never considered duplicates of anything.
pub fn dedup_window(items: &mut [ContextItem]) {
    use std::collections::HashSet;
    let mut seen: HashSet<String> = HashSet::new();
    for item in items.iter_mut() {
        if let Some(key) = &item.key {
            if seen.contains(key) {
                item.text = repeat_ref(&item.label);
            } else {
                seen.insert(key.clone());
            }
        }
    }
}

// ---- tool results (R1) --------------------------------------------------
//
// A tool result is stored as `title → status`, then optionally `[exit N]`,
// then the tool's own output. The first line is the same shape it always had,
// so a result with no output is byte-for-byte what older sessions hold.

/// A parsed tool-result body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResultView<'a> {
    pub title: &'a str,
    pub status: &'a str,
    pub exit: Option<i32>,
    pub output: &'a str,
}

/// Build the stored body of a tool result. Newlines in the title would
/// swallow the output boundary, so they become spaces.
pub fn encode_tool_result(title: &str, status: &str, exit: Option<i32>, output: &str) -> String {
    let title = title.replace(['\r', '\n'], " ");
    let mut s = format!("{title} → {status}");
    if let Some(code) = exit {
        s.push_str(&format!("\n[exit {code}]"));
    }
    let out = output.trim_end();
    if !out.is_empty() {
        s.push('\n');
        s.push_str(out);
    }
    s
}

/// Parse a body written by [`encode_tool_result`] (or an older `title → status`).
/// `None` for anything else, e.g. the JSON bodies subagent events use.
pub fn parse_tool_result(body: &str) -> Option<ToolResultView<'_>> {
    let (head, rest) = body.split_once('\n').unwrap_or((body, ""));
    if head.starts_with('{') {
        return None;
    }
    let (title, status) = head.rsplit_once(" → ")?;
    let status = status.trim();
    if status.is_empty() || status.contains(' ') {
        return None;
    }
    let (exit, output) = match rest.split_once('\n').unwrap_or((rest, "")) {
        (first, tail) if first.starts_with("[exit ") && first.ends_with(']') => {
            match first[6..first.len() - 1].parse::<i32>() {
                Ok(code) => (Some(code), tail),
                Err(_) => (None, rest),
            }
        }
        _ => (None, rest),
    };
    Some(ToolResultView { title, status, exit, output })
}

impl ToolResultView<'_> {
    /// Failed by status or by a non-zero exit code.
    pub fn failed(&self) -> bool {
        self.status == "failed" || self.exit.is_some_and(|c| c != 0)
    }

    /// Says nothing the tool call above it did not already say.
    pub fn is_bare_success(&self) -> bool {
        self.output.trim().is_empty()
            && self.exit.unwrap_or(0) == 0
            && matches!(self.status, "completed" | "success" | "ok")
    }
}

/// A tool result no longer than this is kept whole at every rung.
const TOOL_RESULT_SHORT_BYTES: usize = 240;
/// What a recent (L0) result shows of its output: head and tail lines.
const TOOL_RESULT_HEAD: usize = 8;
const TOOL_RESULT_TAIL: usize = 8;
const TOOL_RESULT_L0_MAX_CHARS: usize = 1500;

/// The rungs of one tool result: `(l0, older)`. `None` means "render the body
/// as it is" — no output to speak of, or short enough that a reference line
/// would not be shorter.
///
/// `older` is what a result past the L0 window becomes: one line naming the
/// call, its outcome, how many lines it printed and the blob that holds them
/// (`read src/x.ts → completed, 212 lines (blob:ab12cd34)`), so the agent
/// knows what was there and can run it again. A failure keeps the last lines of
/// its output, where the error is. Pure render; no model call.
pub fn tool_result_rungs(body: &str, blob: Option<&str>) -> Option<(String, String)> {
    let v = parse_tool_result(body)?;
    if v.output.trim().is_empty() || body.len() <= TOOL_RESULT_SHORT_BYTES {
        return None;
    }
    let lines = v.output.lines().count();
    let mut head = format!("{} → {}", v.title, v.status);
    if let Some(code) = v.exit {
        head.push_str(&format!(" (exit {code})"));
    }

    let mut l0 = format!("{head}\n{}", truncate_middle(v.output, TOOL_RESULT_HEAD, TOOL_RESULT_TAIL));
    if l0.chars().count() > TOOL_RESULT_L0_MAX_CHARS {
        l0 = l0.chars().take(TOOL_RESULT_L0_MAX_CHARS).collect::<String>() + " …";
    }

    let mut older = format!("{head}, {lines} lines");
    if let Some(id) = blob {
        older.push_str(&format!(" (blob:{})", &id[..id.len().min(8)]));
    }
    if v.failed() {
        let tail: Vec<&str> = v.output.lines().rev().take(3).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        let tail = tail.join("\n");
        let tail: String = tail.chars().take(300).collect();
        older.push('\n');
        older.push_str(&tail);
    }
    Some((l0, older))
}

#[cfg(test)]
mod tool_result_tests {
    use super::*;

    #[test]
    fn encode_parse_roundtrip_and_old_shape() {
        assert_eq!(encode_tool_result("cargo test", "completed", None, ""), "cargo test → completed");
        let b = encode_tool_result("cargo test\n-q", "failed", Some(101), "boom\nline2\n");
        let v = parse_tool_result(&b).unwrap();
        assert_eq!(v.title, "cargo test -q");
        assert_eq!(v.status, "failed");
        assert_eq!(v.exit, Some(101));
        assert_eq!(v.output, "boom\nline2");
        assert!(v.failed());
        assert_eq!(parse_tool_result("{\"event\":\"x → y\"}"), None);
    }

    #[test]
    fn output_that_looks_like_an_arrow_does_not_break_the_header() {
        let b = encode_tool_result("read a.ts", "completed", None, "a → b\nc");
        let v = parse_tool_result(&b).unwrap();
        assert_eq!((v.title, v.status), ("read a.ts", "completed"));
        assert!(!v.is_bare_success());
    }

    #[test]
    fn old_results_become_one_line_with_blob_ref() {
        let out: String = (0..212).map(|i| format!("line {i} of the file\n")).collect();
        let b = encode_tool_result("read src/x.ts", "completed", None, &out);
        let (l0, old) = tool_result_rungs(&b, Some("ab12cd34ef56")).unwrap();
        assert_eq!(old, "read src/x.ts → completed, 212 lines (blob:ab12cd34)");
        assert!(l0.contains("lines omitted") && l0.len() < b.len() / 4);
    }

    #[test]
    fn failures_keep_the_tail_and_short_results_stay_whole() {
        let out: String = (0..50).map(|i| format!("noise {i}\n")).collect::<String>() + "error: it broke\n";
        let b = encode_tool_result("cargo build", "failed", Some(101), &out);
        let (_, old) = tool_result_rungs(&b, None).unwrap();
        assert!(old.starts_with("cargo build → failed (exit 101), 51 lines"));
        assert!(old.ends_with("error: it broke"));
        let short = encode_tool_result("ls", "completed", None, "a\nb");
        assert!(tool_result_rungs(&short, None).is_none());
    }
}
