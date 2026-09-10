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
