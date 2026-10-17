//! Syntax spans for the shell's read-only code viewer.
//!
//! A small, dependency-free lexer: enough to colour keywords, strings,
//! comments, numbers, types, calls and attributes the way an editor does,
//! not a parser. State (block comment, multi-line string) carries across
//! lines so a `/* … */` or `"""…"""` spanning lines stays coloured.
//!
//! Output is per line, offsets in UTF-8 bytes *within that line*, lines split
//! on `\n` exactly like the shell splits them. The shell maps a [`Kind`] to a
//! colour; no colour decisions live here.

use serde::Serialize;

/// What a span is. The numeric value is the wire format (FFI JSON).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[repr(u8)]
pub enum Kind {
    Keyword = 0,
    Type = 1,
    String = 2,
    Comment = 3,
    Number = 4,
    Function = 5,
    /// Attributes, decorators, macros, preprocessor, map keys.
    Attribute = 6,
    /// `true`, `nil`, `None`, `self`…
    Constant = 7,
}

/// `(start, len, kind)` in bytes within one line.
pub type Span = (u32, u32, u8);

/// Files above this are shown plain — highlighting is a nicety, not worth
/// a multi-megabyte JSON hop for a generated file.
pub const MAX_BYTES: usize = 2 * 1024 * 1024;

struct Lang {
    line_comments: &'static [&'static str],
    block: Option<(&'static str, &'static str)>,
    /// Quote strings, longest first (`"""` before `"`).
    quotes: &'static [&'static str],
    keywords: &'static [&'static str],
    constants: &'static [&'static str],
    /// `#[..]` / `#if` lines and `name!` macros (Rust, Swift `#available`).
    hash_attr: bool,
    /// `@decorator` / `@MainActor`.
    at_attr: bool,
    macro_bang: bool,
    /// A string followed by `:` is a key (JSON).
    string_keys: bool,
    /// `key =` / `key:` at line start is a key, `[section]` a header.
    config: bool,
    /// Rust `'a` lifetimes are not char literals.
    lifetimes: bool,
}

const C_LIKE_COMMENTS: &[&str] = &["//"];
const HASH_COMMENTS: &[&str] = &["#"];

const RUST_KW: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
    "return", "static", "struct", "super", "trait", "type", "unsafe", "use", "where", "while",
];
const RUST_CONST: &[&str] = &["true", "false", "None", "Some", "Ok", "Err", "self", "Self"];

const SWIFT_KW: &[&str] = &[
    "actor",
    "associatedtype",
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "continue",
    "default",
    "defer",
    "deinit",
    "do",
    "else",
    "enum",
    "extension",
    "fallthrough",
    "fileprivate",
    "final",
    "for",
    "func",
    "guard",
    "if",
    "import",
    "in",
    "init",
    "inout",
    "internal",
    "is",
    "lazy",
    "let",
    "mutating",
    "nonisolated",
    "open",
    "operator",
    "override",
    "private",
    "protocol",
    "public",
    "repeat",
    "rethrows",
    "return",
    "some",
    "any",
    "static",
    "struct",
    "subscript",
    "switch",
    "throw",
    "throws",
    "try",
    "typealias",
    "var",
    "weak",
    "where",
    "while",
    "as",
];
const SWIFT_CONST: &[&str] = &["true", "false", "nil", "self", "Self", "super"];

const JS_KW: &[&str] = &[
    "as",
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "finally",
    "for",
    "from",
    "function",
    "if",
    "implements",
    "import",
    "in",
    "instanceof",
    "interface",
    "let",
    "new",
    "of",
    "private",
    "protected",
    "public",
    "readonly",
    "return",
    "static",
    "switch",
    "throw",
    "try",
    "type",
    "typeof",
    "var",
    "void",
    "while",
    "yield",
    "declare",
    "namespace",
    "keyof",
];
const JS_CONST: &[&str] = &["true", "false", "null", "undefined", "this", "super", "NaN"];

const PY_KW: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif",
    "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda",
    "nonlocal", "not", "or", "pass", "raise", "return", "try", "while", "with", "yield", "match",
    "case",
];
const PY_CONST: &[&str] = &["True", "False", "None", "self", "cls"];

const C_KW: &[&str] = &[
    "auto",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "default",
    "defer",
    "delete",
    "do",
    "else",
    "enum",
    "extends",
    "extern",
    "final",
    "for",
    "func",
    "go",
    "goto",
    "if",
    "implements",
    "import",
    "interface",
    "inline",
    "namespace",
    "new",
    "override",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "select",
    "sizeof",
    "static",
    "struct",
    "switch",
    "template",
    "throw",
    "throws",
    "try",
    "typedef",
    "typename",
    "union",
    "using",
    "var",
    "virtual",
    "void",
    "volatile",
    "while",
    "chan",
    "map",
    "range",
    "type",
    "fun",
    "val",
    "when",
    "object",
    "is",
    "in",
    "as",
];
const C_CONST: &[&str] = &[
    "true", "false", "null", "nullptr", "NULL", "nil", "this", "self", "iota",
];

const SH_KW: &[&str] = &[
    "if", "then", "else", "elif", "fi", "for", "in", "do", "done", "while", "until", "case",
    "esac", "function", "return", "export", "local", "readonly", "set", "unset", "source", "exit",
    "shift",
];
const RB_KW: &[&str] = &[
    "begin", "class", "def", "do", "else", "elsif", "end", "ensure", "if", "module", "next",
    "rescue", "return", "unless", "until", "when", "while", "yield", "require", "case", "then",
];
const SQL_KW: &[&str] = &[
    "select",
    "from",
    "where",
    "insert",
    "into",
    "values",
    "update",
    "set",
    "delete",
    "create",
    "table",
    "index",
    "on",
    "and",
    "or",
    "not",
    "null",
    "primary",
    "key",
    "references",
    "join",
    "left",
    "inner",
    "group",
    "by",
    "order",
    "limit",
    "as",
    "if",
    "exists",
    "integer",
    "text",
    "blob",
    "real",
    "default",
    "unique",
    "drop",
    "alter",
    "pragma",
    "begin",
    "commit",
    "with",
];

const fn lang(
    line_comments: &'static [&'static str],
    block: Option<(&'static str, &'static str)>,
    quotes: &'static [&'static str],
    keywords: &'static [&'static str],
    constants: &'static [&'static str],
) -> Lang {
    Lang {
        line_comments,
        block,
        quotes,
        keywords,
        constants,
        hash_attr: false,
        at_attr: false,
        macro_bang: false,
        string_keys: false,
        config: false,
        lifetimes: false,
    }
}

const C_BLOCK: Option<(&str, &str)> = Some(("/*", "*/"));

fn lang_for(ext: &str) -> Option<Lang> {
    Some(match ext {
        "rs" => Lang {
            hash_attr: true,
            macro_bang: true,
            lifetimes: true,
            ..lang(C_LIKE_COMMENTS, C_BLOCK, &["\"", "'"], RUST_KW, RUST_CONST)
        },
        "swift" => Lang {
            hash_attr: true,
            at_attr: true,
            ..lang(
                C_LIKE_COMMENTS,
                C_BLOCK,
                &["\"\"\"", "\""],
                SWIFT_KW,
                SWIFT_CONST,
            )
        },
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => Lang {
            at_attr: true,
            ..lang(C_LIKE_COMMENTS, C_BLOCK, &["\"", "'", "`"], JS_KW, JS_CONST)
        },
        "py" => Lang {
            at_attr: true,
            ..lang(
                HASH_COMMENTS,
                None,
                &["\"\"\"", "'''", "\"", "'"],
                PY_KW,
                PY_CONST,
            )
        },
        "c" | "h" | "cc" | "cpp" | "hpp" | "m" | "mm" | "go" | "java" | "kt" | "kts" | "cs"
        | "dart" | "scala" => Lang {
            hash_attr: true,
            at_attr: true,
            ..lang(C_LIKE_COMMENTS, C_BLOCK, &["\"", "'", "`"], C_KW, C_CONST)
        },
        "sh" | "bash" | "zsh" | "fish" => {
            lang(HASH_COMMENTS, None, &["\"", "'"], SH_KW, &["true", "false"])
        }
        "rb" => lang(
            HASH_COMMENTS,
            None,
            &["\"", "'"],
            RB_KW,
            &["true", "false", "nil", "self"],
        ),
        "sql" => lang(&["--"], C_BLOCK, &["'", "\""], SQL_KW, &["true", "false"]),
        "css" | "scss" => lang(C_LIKE_COMMENTS, C_BLOCK, &["\"", "'"], &[], &[]),
        "json" | "jsonc" => Lang {
            string_keys: true,
            ..lang(
                C_LIKE_COMMENTS,
                C_BLOCK,
                &["\""],
                &[],
                &["true", "false", "null"],
            )
        },
        "toml" | "ini" | "cfg" | "conf" | "env" => Lang {
            config: true,
            ..lang(
                HASH_COMMENTS,
                None,
                &["\"\"\"", "\"", "'"],
                &[],
                &["true", "false"],
            )
        },
        "yaml" | "yml" => Lang {
            config: true,
            ..lang(
                HASH_COMMENTS,
                None,
                &["\"", "'"],
                &[],
                &["true", "false", "null", "yes", "no"],
            )
        },
        _ => return None,
    })
}

/// Lexer state that survives a line break.
#[derive(Clone, PartialEq)]
enum Carry {
    Code,
    /// Inside a block comment, with Rust-style nesting depth.
    Block(u32),
    /// Inside a string opened by this delimiter.
    Str(&'static str),
}

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
}

/// Spans for each line of `text`, chosen by the extension of `path`.
/// Unknown languages and files above [`MAX_BYTES`] get empty lines.
pub fn highlight(path: &str, text: &str) -> Vec<Vec<Span>> {
    let lines: Vec<&str> = text.split('\n').collect();
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    if text.len() > MAX_BYTES {
        return vec![Vec::new(); lines.len()];
    }
    if ext == "md" || ext == "markdown" {
        return markdown(&lines);
    }
    let Some(lang) = lang_for(&ext) else {
        return vec![Vec::new(); lines.len()];
    };
    let mut carry = Carry::Code;
    lines
        .iter()
        .map(|l| line_spans(&lang, l, &mut carry))
        .collect()
}

fn line_spans(lang: &Lang, line: &str, carry: &mut Carry) -> Vec<Span> {
    let b = line.as_bytes();
    let n = b.len();
    let mut out: Vec<Span> = Vec::new();
    let mut push = |s: usize, e: usize, k: Kind| {
        if e > s {
            out.push((s as u32, (e - s) as u32, k as u8));
        }
    };
    let mut i = 0;

    // Finish whatever the previous line left open.
    match carry.clone() {
        Carry::Block(depth) => {
            let (open, close) = lang.block.unwrap_or(("/*", "*/"));
            let (end, d) = scan_block(b, 0, depth, open, close);
            push(0, end, Kind::Comment);
            *carry = if d == 0 { Carry::Code } else { Carry::Block(d) };
            i = end;
        }
        Carry::Str(q) => {
            let (end, closed) = scan_string(b, 0, q);
            push(0, end, Kind::String);
            *carry = if closed { Carry::Code } else { Carry::Str(q) };
            i = end;
        }
        Carry::Code => {}
    }
    if *carry != Carry::Code {
        return out;
    }

    let trimmed = line.trim_start();
    let indent = n - trimmed.len();
    if lang.config && i == 0 {
        if trimmed.starts_with('[') && trimmed.trim_end().ends_with(']') {
            push(indent, n, Kind::Keyword);
            return out;
        }
        let key_end = trimmed
            .bytes()
            .position(|c| !(is_ident(c) || c == b'-' || c == b'.'))
            .unwrap_or(trimmed.len());
        let rest = trimmed[key_end..].trim_start();
        if key_end > 0 && (rest.starts_with('=') || rest.starts_with(':')) {
            push(indent, indent + key_end, Kind::Attribute);
            i = indent + key_end;
        }
    }
    if lang.hash_attr && i == 0 && trimmed.starts_with('#') && !trimmed.starts_with("#[") {
        // `#if` / `#include` / `#available` style directives.
        if trimmed[1..].starts_with(|c: char| c.is_ascii_alphabetic()) {
            let end = indent + 1 + trimmed[1..].bytes().take_while(|c| is_ident(*c)).count();
            push(indent, end, Kind::Attribute);
            i = end;
        }
    }

    while i < n {
        let rest = &line[i..];
        let c = b[i];

        if let Some(lc) = lang.line_comments.iter().find(|lc| rest.starts_with(**lc)) {
            // `#` only starts a comment at a word boundary (`a#b` in shells).
            if *lc != "#" || i == 0 || !is_ident(b[i - 1]) {
                push(i, n, Kind::Comment);
                return out;
            }
        }
        if let Some((open, close)) = lang.block {
            if rest.starts_with(open) {
                let (end, d) = scan_block(b, i + open.len(), 1, open, close);
                push(i, end, Kind::Comment);
                if d > 0 {
                    *carry = Carry::Block(d);
                    return out;
                }
                i = end;
                continue;
            }
        }
        if lang.lifetimes && c == b'\'' && i + 2 < n && is_ident(b[i + 1]) && b[i + 2] != b'\'' {
            // `'a` / `'static`, or a char like `'\n'` handled below.
            if b[i + 1] != b'\\' {
                let end = i + 1 + b[i + 1..].iter().take_while(|x| is_ident(**x)).count();
                if end >= n || b[end] != b'\'' {
                    push(i, end, Kind::Keyword);
                    i = end;
                    continue;
                }
            }
        }
        if let Some(q) = lang.quotes.iter().find(|q| rest.starts_with(**q)) {
            let (end, closed) = scan_string(b, i + q.len(), q);
            let kind = if lang.string_keys && closed && line[end..].trim_start().starts_with(':') {
                Kind::Attribute
            } else {
                Kind::String
            };
            push(i, end, kind);
            if !closed {
                // Single-quoted strings don't run past a line in most of
                // these languages; multi-line ones (`"""`, `` ` ``, Rust's
                // `"`) do.
                if q.len() > 1 || *q == "`" || (*q == "\"" && lang.lifetimes) {
                    *carry = Carry::Str(q);
                }
                return out;
            }
            i = end;
            continue;
        }
        if lang.hash_attr && (rest.starts_with("#[") || rest.starts_with("#![")) {
            let end = rest.find(']').map(|p| i + p + 1).unwrap_or(n);
            push(i, end, Kind::Attribute);
            i = end;
            continue;
        }
        if lang.at_attr && c == b'@' && i + 1 < n && is_ident(b[i + 1]) {
            let end = i + 1 + b[i + 1..].iter().take_while(|x| is_ident(**x)).count();
            push(i, end, Kind::Attribute);
            i = end;
            continue;
        }
        if c.is_ascii_digit() && (i == 0 || !is_ident(b[i - 1])) {
            let end = i + b[i..]
                .iter()
                .take_while(|x| x.is_ascii_alphanumeric() || **x == b'_' || **x == b'.')
                .count();
            // Don't swallow a range `0..n` or a method call `1.max(2)`.
            let end = line[i..end].find("..").map(|p| i + p).unwrap_or(end);
            push(i, end, Kind::Number);
            i = end;
            continue;
        }
        if is_ident(c) {
            let end = i + b[i..].iter().take_while(|x| is_ident(**x)).count();
            let word = &line[i..end];
            let next = b.get(end).copied();
            let kind = if lang.keywords.contains(&word)
                || (lang.keywords == SQL_KW && SQL_KW.contains(&word.to_ascii_lowercase().as_str()))
            {
                Some(Kind::Keyword)
            } else if lang.constants.contains(&word) {
                Some(Kind::Constant)
            } else if lang.macro_bang && next == Some(b'!') && b.get(end + 1) != Some(&b'=') {
                push(i, end + 1, Kind::Attribute);
                i = end + 1;
                continue;
            } else if word.as_bytes()[0].is_ascii_uppercase() {
                Some(Kind::Type)
            } else if next == Some(b'(') {
                Some(Kind::Function)
            } else {
                None
            };
            if let Some(k) = kind {
                push(i, end, k);
            }
            i = end;
            continue;
        }
        i += 1;
    }
    out
}

/// End of a string opened just before `from`, and whether it closed.
fn scan_string(b: &[u8], from: usize, q: &str) -> (usize, bool) {
    let qb = q.as_bytes();
    let mut i = from;
    while i < b.len() {
        if b[i] == b'\\' && q != "'''" {
            i += 2;
            continue;
        }
        if b[i..].starts_with(qb) {
            return (i + qb.len(), true);
        }
        i += 1;
    }
    (b.len(), false)
}

/// End of a block comment and the depth still open at that point.
fn scan_block(b: &[u8], from: usize, mut depth: u32, open: &str, close: &str) -> (usize, u32) {
    let (ob, cb) = (open.as_bytes(), close.as_bytes());
    let mut i = from;
    while i < b.len() {
        if b[i..].starts_with(cb) {
            depth -= 1;
            i += cb.len();
            if depth == 0 {
                return (i, 0);
            }
        } else if b[i..].starts_with(ob) {
            depth += 1;
            i += ob.len();
        } else {
            i += 1;
        }
    }
    (b.len(), depth)
}

fn markdown(lines: &[&str]) -> Vec<Vec<Span>> {
    let mut fenced = false;
    lines
        .iter()
        .map(|l| {
            let n = l.len() as u32;
            let t = l.trim_start();
            if t.starts_with("```") {
                fenced = !fenced;
                return vec![(0, n, Kind::Comment as u8)];
            }
            if fenced {
                return vec![(0, n, Kind::String as u8)];
            }
            if t.starts_with('#') {
                return vec![(0, n, Kind::Keyword as u8)];
            }
            // Inline `code`.
            let mut out = Vec::new();
            let mut open: Option<usize> = None;
            for (i, c) in l.bytes().enumerate() {
                if c == b'`' {
                    match open.take() {
                        Some(s) => out.push((s as u32, (i + 1 - s) as u32, Kind::String as u8)),
                        None => open = Some(i),
                    }
                }
            }
            out
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(path: &str, text: &str) -> Vec<Vec<(String, u8)>> {
        let lines: Vec<&str> = text.split('\n').collect();
        highlight(path, text)
            .into_iter()
            .zip(lines)
            .map(|(spans, l)| {
                spans
                    .into_iter()
                    .map(|(s, n, k)| (l[s as usize..(s + n) as usize].to_string(), k))
                    .collect()
            })
            .collect()
    }

    fn has(v: &[(String, u8)], word: &str, k: Kind) -> bool {
        v.iter().any(|(w, kk)| w == word && *kk == k as u8)
    }

    #[test]
    fn rust_basics() {
        let k = kinds("a.rs", "#[derive(Debug)]\npub fn go<'a>(x: &'a str) -> Option<u8> { println!(\"hi {}\", 42); None } // end");
        assert!(has(&k[0], "#[derive(Debug)]", Kind::Attribute));
        let l = &k[1];
        assert!(has(l, "pub", Kind::Keyword));
        assert!(has(l, "fn", Kind::Keyword));
        assert!(has(l, "'a", Kind::Keyword));
        assert!(has(l, "Option", Kind::Type));
        assert!(has(l, "println!", Kind::Attribute));
        assert!(has(l, "\"hi {}\"", Kind::String));
        assert!(has(l, "42", Kind::Number));
        assert!(has(l, "None", Kind::Constant));
        assert!(has(l, "// end", Kind::Comment));
    }

    #[test]
    fn block_comment_and_multiline_string_carry_across_lines() {
        let k = kinds(
            "a.swift",
            "/* one\ntwo */ let x = \"\"\"\nbody\n\"\"\"\nfunc f()",
        );
        assert!(has(&k[0], "/* one", Kind::Comment));
        assert!(has(&k[1], "two */", Kind::Comment));
        assert!(has(&k[1], "let", Kind::Keyword));
        assert!(has(&k[2], "body", Kind::String));
        assert!(has(&k[3], "\"\"\"", Kind::String));
        assert!(has(&k[4], "func", Kind::Keyword));
        assert!(has(&k[4], "f", Kind::Function));
    }

    #[test]
    fn json_keys_differ_from_values() {
        let k = kinds("a.json", "{\"name\": \"chorus\", \"n\": 1, \"ok\": true}");
        assert!(has(&k[0], "\"name\"", Kind::Attribute));
        assert!(has(&k[0], "\"chorus\"", Kind::String));
        assert!(has(&k[0], "true", Kind::Constant));
    }

    #[test]
    fn offsets_are_bytes_within_the_line_and_survive_thai() {
        let text = "let s = \"สวัสดี\" // ไทย";
        let spans = &highlight("a.rs", text)[0];
        for (s, n, _) in spans {
            assert!(text.is_char_boundary(*s as usize));
            assert!(text.is_char_boundary((*s + *n) as usize));
        }
        let k = kinds("a.rs", text);
        assert!(has(&k[0], "\"สวัสดี\"", Kind::String));
        assert!(has(&k[0], "// ไทย", Kind::Comment));
    }

    #[test]
    fn unknown_language_is_plain_with_one_entry_per_line() {
        let out = highlight("notes.xyz", "a\nb\nc");
        assert_eq!(out.len(), 3);
        assert!(out.iter().all(|l| l.is_empty()));
    }

    #[test]
    fn toml_sections_and_keys() {
        let k = kinds("Cargo.toml", "[package]\nname = \"chorus\" # c");
        assert!(has(&k[0], "[package]", Kind::Keyword));
        assert!(has(&k[1], "name", Kind::Attribute));
        assert!(has(&k[1], "\"chorus\"", Kind::String));
        assert!(has(&k[1], "# c", Kind::Comment));
    }
}
