//! Installable tree-sitter grammars for the code viewer.
//!
//! The built-in lexer in [`crate::highlight`] colours ~15 languages roughly
//! and needs nothing. A grammar the user installs here replaces it for its
//! language with a real parse: download the grammar's source (GitHub
//! tarball, `curl` + `tar` — no archive crates), compile `src/parser.c`
//! (+ scanner) into a dylib with the system C compiler through
//! `tree-sitter-loader` — the same path the tree-sitter CLI takes — and
//! keep its `queries/highlights.scm`. Removing deletes the folder; nothing
//! else remembers it.
//!
//! Layout: `~/.chorus/grammars/<id>/` holds the unpacked source, the
//! compiled `parser.<dylib>` and an `installed` marker written last, so a
//! half-finished install never counts as installed.
//!
//! Output keeps the lexer's contract: per-line `(start, len, kind)` spans in
//! UTF-8 bytes, so the shell draws both the same way.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};

use serde::Serialize;
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

use crate::highlight::{Kind, Span};

/// One language the user can install.
#[derive(Debug, Clone, Copy)]
pub struct Grammar {
    pub id: &'static str,
    pub label: &'static str,
    /// `owner/repo` on GitHub.
    repo: &'static str,
    /// Folder inside the repo holding `src/` (repos with several grammars).
    subdir: &'static str,
    pub extensions: &'static [&'static str],
    /// Exact file names (no extension), e.g. `Dockerfile`.
    pub file_names: &'static [&'static str],
    /// Interpreter names a `#!` line may carry.
    pub shebangs: &'static [&'static str],
}

const fn g(
    id: &'static str,
    label: &'static str,
    repo: &'static str,
    extensions: &'static [&'static str],
) -> Grammar {
    Grammar {
        id,
        label,
        repo,
        subdir: "",
        extensions,
        file_names: &[],
        shebangs: &[],
    }
}

/// Grammars whose repos commit the generated `src/parser.c` — installing
/// must not need the tree-sitter CLI or Node.
pub const CATALOG: &[Grammar] = &[
    g("rust", "Rust", "tree-sitter/tree-sitter-rust", &["rs"]),
    Grammar {
        shebangs: &["python", "python3"],
        ..g("python", "Python", "tree-sitter/tree-sitter-python", &["py", "pyi", "pyw"])
    },
    Grammar {
        shebangs: &["node", "deno", "bun"],
        ..g(
            "javascript",
            "JavaScript",
            "tree-sitter/tree-sitter-javascript",
            &["js", "jsx", "mjs", "cjs"],
        )
    },
    Grammar {
        subdir: "typescript",
        ..g("typescript", "TypeScript", "tree-sitter/tree-sitter-typescript", &["ts", "mts", "cts"])
    },
    Grammar {
        subdir: "tsx",
        ..g("tsx", "TSX", "tree-sitter/tree-sitter-typescript", &["tsx"])
    },
    g("go", "Go", "tree-sitter/tree-sitter-go", &["go"]),
    g("java", "Java", "tree-sitter/tree-sitter-java", &["java"]),
    g("c", "C", "tree-sitter/tree-sitter-c", &["c", "h"]),
    g(
        "cpp",
        "C++",
        "tree-sitter/tree-sitter-cpp",
        &["cc", "cpp", "cxx", "hpp", "hh", "hxx", "ipp"],
    ),
    g("c_sharp", "C#", "tree-sitter/tree-sitter-c-sharp", &["cs"]),
    Grammar {
        file_names: &["Gemfile", "Rakefile", "Podfile", "Fastfile"],
        shebangs: &["ruby"],
        ..g("ruby", "Ruby", "tree-sitter/tree-sitter-ruby", &["rb", "rake", "gemspec"])
    },
    Grammar {
        subdir: "php",
        shebangs: &["php"],
        ..g("php", "PHP", "tree-sitter/tree-sitter-php", &["php"])
    },
    Grammar {
        file_names: &[".bashrc", ".zshrc", ".profile", ".bash_profile"],
        shebangs: &["sh", "bash", "zsh", "dash"],
        ..g("bash", "Shell (Bash)", "tree-sitter/tree-sitter-bash", &["sh", "bash", "zsh"])
    },
    g("json", "JSON", "tree-sitter/tree-sitter-json", &["json", "jsonc"]),
    g("html", "HTML", "tree-sitter/tree-sitter-html", &["html", "htm"]),
    g("css", "CSS", "tree-sitter/tree-sitter-css", &["css"]),
    g("scala", "Scala", "tree-sitter/tree-sitter-scala", &["scala", "sc"]),
    g("haskell", "Haskell", "tree-sitter/tree-sitter-haskell", &["hs"]),
    g("yaml", "YAML", "tree-sitter-grammars/tree-sitter-yaml", &["yaml", "yml"]),
    Grammar {
        file_names: &["Cargo.lock"],
        ..g("toml", "TOML", "tree-sitter-grammars/tree-sitter-toml", &["toml"])
    },
    Grammar {
        shebangs: &["lua"],
        ..g("lua", "Lua", "tree-sitter-grammars/tree-sitter-lua", &["lua"])
    },
    g("zig", "Zig", "tree-sitter-grammars/tree-sitter-zig", &["zig"]),
    g("kotlin", "Kotlin", "fwcd/tree-sitter-kotlin", &["kt", "kts"]),
    g("elixir", "Elixir", "elixir-lang/tree-sitter-elixir", &["ex", "exs"]),
    Grammar {
        file_names: &["Dockerfile", "Containerfile"],
        ..g("dockerfile", "Dockerfile", "camdencheek/tree-sitter-dockerfile", &["dockerfile"])
    },
    Grammar {
        file_names: &["Makefile", "makefile", "GNUmakefile"],
        ..g("make", "Makefile", "alemuller/tree-sitter-make", &["mk", "mak"])
    },
];

pub fn find(id: &str) -> Option<&'static Grammar> {
    CATALOG.iter().find(|g| g.id == id)
}

/// Which catalog language a file is, from its name, then its `#!` line.
/// Independent of what is installed: the shell shows "install X" off it.
pub fn detect(path: &str, text: &str) -> Option<&'static Grammar> {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    if let Some(g) = CATALOG.iter().find(|g| g.file_names.contains(&name)) {
        return Some(g);
    }
    if let Some((_, ext)) = name.rsplit_once('.').filter(|(stem, _)| !stem.is_empty()) {
        let ext = ext.to_ascii_lowercase();
        if let Some(g) = CATALOG.iter().find(|g| g.extensions.contains(&ext.as_str())) {
            return Some(g);
        }
    }
    shebang_interpreter(text).and_then(|i| CATALOG.iter().find(|g| g.shebangs.contains(&i)))
}

/// `#!/usr/bin/env python3` → `python3`; `#!/bin/bash -e` → `bash`.
fn shebang_interpreter(text: &str) -> Option<&str> {
    let first = text.lines().next()?.strip_prefix("#!")?;
    let mut parts = first.split_whitespace();
    let prog = parts.next()?;
    let base = prog.rsplit('/').next()?;
    let base = if base == "env" {
        parts.find(|p| !p.starts_with('-'))?
    } else {
        base
    };
    // `python3.12` → `python3`
    Some(base.split_once('.').map_or(base, |(a, _)| a))
}

// ---- install / remove --------------------------------------------------

fn root() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CHORUS_GRAMMARS_DIR") {
        return Some(PathBuf::from(dir));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".chorus").join("grammars"))
}

fn dir(id: &str) -> Option<PathBuf> {
    root().map(|r| r.join(id))
}

fn lib_path(id: &str) -> Option<PathBuf> {
    dir(id).map(|d| d.join(format!("parser.{}", std::env::consts::DLL_EXTENSION)))
}

pub fn is_installed(id: &str) -> bool {
    dir(id).is_some_and(|d| d.join("installed").is_file())
}

/// One row for Settings.
#[derive(Debug, Serialize)]
pub struct Row {
    pub id: &'static str,
    pub label: &'static str,
    pub extensions: &'static [&'static str],
    pub installed: bool,
    pub source: String,
}

pub fn list() -> Vec<Row> {
    CATALOG
        .iter()
        .map(|g| Row {
            id: g.id,
            label: g.label,
            extensions: g.extensions,
            installed: is_installed(g.id),
            source: format!("https://github.com/{}", g.repo),
        })
        .collect()
}

/// Download, compile and keep one grammar. Blocking (network + `cc`):
/// call it off the UI thread. Re-installing rebuilds from scratch.
pub fn install(id: &str) -> Result<(), String> {
    let g = find(id).ok_or_else(|| format!("ไม่รู้จักภาษา {id}"))?;
    let dest = dir(id).ok_or("ไม่รู้ตำแหน่ง HOME")?;
    let staging = dest.with_extension("partial");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    let result = (|| {
        let url = format!("https://github.com/{}/archive/HEAD.tar.gz", g.repo);
        let tarball = staging.join("src.tar.gz");
        run(Command::new("curl")
            .args(["-fsSL", "--max-time", "120", "-o"])
            .arg(&tarball)
            .arg(&url))
        .map_err(|e| format!("ดาวน์โหลด {url} ไม่ได้: {e}"))?;
        let src = staging.join("repo");
        std::fs::create_dir_all(&src).map_err(|e| e.to_string())?;
        run(Command::new("tar")
            .args(["-xzf"])
            .arg(&tarball)
            .args(["--strip-components", "1", "-C"])
            .arg(&src))
        .map_err(|e| format!("แตกไฟล์ไม่ได้: {e}"))?;
        let _ = std::fs::remove_file(&tarball);
        let grammar = if g.subdir.is_empty() { src.clone() } else { src.join(g.subdir) };
        if !grammar.join("src").join("parser.c").is_file() {
            return Err(format!("{} ไม่มี src/parser.c ที่ build ไว้แล้ว", g.repo));
        }
        let lib = staging.join(format!("parser.{}", std::env::consts::DLL_EXTENSION));
        let loader = tree_sitter_loader::Loader::with_parser_lib_path(staging.join("cache"));
        loader
            .compile_parser_at_path(&grammar, lib.clone(), &[])
            .map_err(|e| {
                format!(
                    "compile ไม่ผ่าน — ต้องมี C compiler (macOS: `xcode-select --install`): {e}"
                )
            })?;
        if highlights_query(&grammar, &src).is_none() {
            return Err(format!("{} ไม่มี queries/highlights.scm", g.repo));
        }
        Ok(())
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }
    let _ = std::fs::remove_dir_all(staging.join("cache"));
    std::fs::write(staging.join("installed"), g.repo).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::rename(&staging, &dest).map_err(|e| e.to_string())?;
    forget(id);
    Ok(())
}

pub fn remove(id: &str) -> Result<(), String> {
    find(id).ok_or_else(|| format!("ไม่รู้จักภาษา {id}"))?;
    forget(id);
    match dir(id) {
        Some(d) if d.exists() => std::fs::remove_dir_all(&d).map_err(|e| e.to_string()),
        _ => Ok(()),
    }
}

fn run(cmd: &mut Command) -> Result<(), String> {
    let out = cmd.output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().chars().take(400).collect())
    }
}

/// The grammar's highlight query, preferring its own folder over the repo
/// root (multi-grammar repos keep queries at either level).
fn highlights_query(grammar: &Path, repo: &Path) -> Option<String> {
    [grammar, repo]
        .iter()
        .map(|p| p.join("queries").join("highlights.scm"))
        .find_map(|p| std::fs::read_to_string(p).ok())
}

// ---- highlighting ------------------------------------------------------

/// Capture names we recognise, longest-prefix matched by tree-sitter
/// (`function.method` → `function`), with the lexer's kind for each.
const CAPTURES: &[(&str, Kind)] = &[
    ("keyword", Kind::Keyword),
    ("tag", Kind::Keyword),
    ("type", Kind::Type),
    ("constructor", Kind::Type),
    ("module", Kind::Type),
    ("namespace", Kind::Type),
    ("string", Kind::String),
    ("escape", Kind::String),
    ("comment", Kind::Comment),
    ("number", Kind::Number),
    ("float", Kind::Number),
    ("function", Kind::Function),
    ("attribute", Kind::Attribute),
    ("property", Kind::Attribute),
    ("label", Kind::Attribute),
    ("constant", Kind::Constant),
    ("boolean", Kind::Constant),
];

type Loaded = Arc<HighlightConfiguration>;

/// Loaded configurations by id. `None` remembers a failed load so a broken
/// grammar falls back to the lexer once instead of on every keystroke.
fn cache() -> &'static Mutex<HashMap<&'static str, Option<Loaded>>> {
    static C: OnceLock<Mutex<HashMap<&'static str, Option<Loaded>>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

fn forget(id: &str) {
    if let Ok(mut c) = cache().lock() {
        c.retain(|k, _| *k != id);
    }
}

fn load(g: &'static Grammar) -> Option<Loaded> {
    if let Ok(c) = cache().lock() {
        if let Some(hit) = c.get(g.id) {
            return hit.clone();
        }
    }
    let loaded = load_uncached(g);
    if let Ok(mut c) = cache().lock() {
        c.insert(g.id, loaded.clone());
    }
    loaded
}

fn load_uncached(g: &'static Grammar) -> Option<Loaded> {
    if !is_installed(g.id) {
        return None;
    }
    let d = dir(g.id)?;
    let repo = d.join("repo");
    let grammar = if g.subdir.is_empty() { repo.clone() } else { repo.join(g.subdir) };
    let name = grammar_name(&grammar.join("src").join("grammar.json"))?;
    let language =
        tree_sitter_loader::Loader::load_language(&lib_path(g.id)?, &format!("tree_sitter_{}", name.replace('-', "_")))
            .ok()?;
    let query = highlights_query(&grammar, &repo)?;
    let mut config = HighlightConfiguration::new(language, g.id, &query, "", "").ok()?;
    let names: Vec<&str> = CAPTURES.iter().map(|(n, _)| *n).collect();
    config.configure(&names);
    Some(Arc::new(config))
}

fn grammar_name(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    v.get("name")?.as_str().map(str::to_string)
}

/// Spans from an installed grammar, or `None` when there is none for this
/// file (the caller falls back to the built-in lexer).
pub fn highlight(path: &str, text: &str) -> Option<Vec<Vec<Span>>> {
    let g = detect(path, text)?;
    let config = load(g)?;
    let mut highlighter = Highlighter::new();
    let events = highlighter
        .highlight(&config, text.as_bytes(), None, None, |_| None)
        .ok()?;
    let mut ranges: Vec<(usize, usize, u8)> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    for event in events {
        match event.ok()? {
            HighlightEvent::HighlightStart(h) => stack.push(h.0),
            HighlightEvent::HighlightEnd => {
                stack.pop();
            }
            HighlightEvent::Source { start, end } => {
                if let Some(&top) = stack.last() {
                    ranges.push((start, end, CAPTURES[top].1 as u8));
                }
            }
        }
    }
    Some(split_lines(text, &ranges))
}

/// Byte ranges over the whole text → per-line spans, cut at each `\n`.
fn split_lines(text: &str, ranges: &[(usize, usize, u8)]) -> Vec<Vec<Span>> {
    let mut starts = vec![0usize];
    starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
    let mut out: Vec<Vec<Span>> = vec![Vec::new(); starts.len()];
    for &(s, e, k) in ranges {
        let mut pos = s;
        while pos < e {
            let line = starts.partition_point(|&ls| ls <= pos) - 1;
            // Where this line's `\n` sits (or the end of the text).
            let line_end = starts.get(line + 1).map_or(text.len(), |n| n - 1);
            let stop = e.min(line_end);
            if stop > pos {
                out[line].push(((pos - starts[line]) as u32, (stop - pos) as u32, k));
            }
            pos = line_end + 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_by_extension_file_name_and_shebang() {
        assert_eq!(detect("src/main.rs", "").map(|g| g.id), Some("rust"));
        assert_eq!(detect("a/b/Dockerfile", "").map(|g| g.id), Some("dockerfile"));
        assert_eq!(detect("App.TSX", "").map(|g| g.id), Some("tsx"));
        assert_eq!(detect("bin/tool", "#!/usr/bin/env python3.12\nprint()").map(|g| g.id), Some("python"));
        assert_eq!(detect("run", "#!/bin/bash -e\n").map(|g| g.id), Some("bash"));
        assert_eq!(detect(".bashrc", "").map(|g| g.id), Some("bash"));
        assert_eq!(detect("notes.txt", "hello").map(|g| g.id), None);
    }

    #[test]
    fn every_catalog_entry_is_unique_and_routable() {
        let mut ids: Vec<_> = CATALOG.iter().map(|g| g.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), CATALOG.len());
        for g in CATALOG {
            assert!(g.repo.contains('/'), "{}", g.id);
            assert!(!g.extensions.is_empty() || !g.file_names.is_empty(), "{}", g.id);
        }
    }

    #[test]
    fn ranges_split_at_line_breaks() {
        let text = "ab\ncd\nef";
        // "b\ncd\ne" as one comment-like range.
        let out = split_lines(text, &[(1, 7, 3)]);
        assert_eq!(out, vec![vec![(1, 1, 3)], vec![(0, 2, 3)], vec![(0, 1, 3)]]);
        let out = split_lines(text, &[(3, 5, 0)]);
        assert_eq!(out, vec![vec![], vec![(0, 2, 0)], vec![]]);
    }

    /// Real download + compile: `cargo test -p chorus-core -- --ignored
    /// installs_compiles` — needs the network and a C compiler.
    #[test]
    #[ignore]
    fn installs_compiles_highlights_and_removes() {
        let root = std::env::temp_dir().join(format!("chorus-grammars-{}", std::process::id()));
        std::env::set_var("CHORUS_GRAMMARS_DIR", &root);
        install("rust").expect("install rust");
        assert!(is_installed("rust"));
        let lines = highlight("x.rs", "// hi\nfn main() { let s = \"x\"; }").expect("spans");
        assert!(lines[0].iter().any(|s| s.2 == Kind::Comment as u8), "{lines:?}");
        assert!(lines[1].iter().any(|s| s.2 == Kind::Keyword as u8), "{lines:?}");
        assert!(lines[1].iter().any(|s| s.2 == Kind::String as u8), "{lines:?}");
        remove("rust").unwrap();
        assert!(!is_installed("rust"));
        assert!(highlight("x.rs", "fn main() {}").is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn not_installed_means_none() {
        std::env::set_var("CHORUS_GRAMMARS_DIR", std::env::temp_dir().join("chorus-no-grammars"));
        assert!(highlight("x.rs", "fn main() {}").is_none());
        assert!(list().iter().all(|r| !r.installed));
    }
}
