//! High-Density Repository Map and Personalized PageRank (PPR) Engine.
//!
//! Inspired by Aider and RepoGraph research, this module builds a Code Property
//! Graph of symbols (functions, structs, classes, traits, protocols), computes
//! Personalized PageRank seeded on active files, and formats a dense, budget-bounded
//! structural map (typically ~1,024 tokens) for prompt context injection.

use crate::tokens::{HeuristicCounter, TokenCounter};
use crate::{ChorusError, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// A code symbol extracted from source code.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SymbolDef {
    pub file_path: String,
    pub name: String,
    pub kind: String, // "fn", "struct", "class", "trait", "protocol", "interface", "enum"
    pub signature: String,
    pub line_number: usize,
}

/// Extracted symbol graph representation.
#[derive(Debug, Clone, Default)]
pub struct SymbolGraph {
    pub definitions: Vec<SymbolDef>,
    pub symbol_to_index: HashMap<String, Vec<usize>>,
    /// Directed edges: index_u -> set of index_v (u references v)
    pub outgoing_edges: HashMap<usize, HashSet<usize>>,
    /// Reverse edges: index_v -> set of index_u
    pub incoming_edges: HashMap<usize, HashSet<usize>>,
}

impl SymbolGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_def(&mut self, def: SymbolDef) -> usize {
        let idx = self.definitions.len();
        self.symbol_to_index
            .entry(def.name.clone())
            .or_default()
            .push(idx);
        self.definitions.push(def);
        idx
    }

    pub fn add_reference(&mut self, from_idx: usize, to_idx: usize) {
        if from_idx != to_idx {
            self.outgoing_edges.entry(from_idx).or_default().insert(to_idx);
            self.incoming_edges.entry(to_idx).or_default().insert(from_idx);
        }
    }
}

/// Extractor for source code symbols across popular languages.
pub struct SymbolExtractor;

impl SymbolExtractor {
    pub fn extract_from_file<P: AsRef<Path>>(path: P) -> Result<Vec<SymbolDef>> {
        let p = path.as_ref();
        let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
        let content = fs::read_to_string(p).map_err(ChorusError::Io)?;
        let path_str = p.to_string_lossy().to_string();

        let mut defs = Vec::new();
        match ext {
            "rs" => Self::extract_rust(&content, &path_str, &mut defs),
            "swift" => Self::extract_swift(&content, &path_str, &mut defs),
            "ts" | "tsx" | "js" | "jsx" => Self::extract_typescript(&content, &path_str, &mut defs),
            "py" => Self::extract_python(&content, &path_str, &mut defs),
            "go" => Self::extract_go(&content, &path_str, &mut defs),
            _ => {}
        }
        Ok(defs)
    }

    fn extract_rust(content: &str, path: &str, out: &mut Vec<SymbolDef>) {
        for (i, line) in content.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.starts_with("//") || trimmed.starts_with("/*") {
                continue;
            }

            if let Some(rest) = trimmed.strip_prefix("pub fn ").or_else(|| trimmed.strip_prefix("fn ")) {
                if let Some(name) = rest.split('(').next().map(str::trim) {
                    let sig = trimmed.split('{').next().unwrap_or(trimmed).trim();
                    out.push(SymbolDef {
                        file_path: path.to_string(),
                        name: name.to_string(),
                        kind: "fn".to_string(),
                        signature: sig.to_string(),
                        line_number: i + 1,
                    });
                }
            } else if let Some(rest) = trimmed.strip_prefix("pub struct ").or_else(|| trimmed.strip_prefix("struct ")) {
                if let Some(name) = rest.split(['{', '(', ';', '<']).next().map(str::trim) {
                    out.push(SymbolDef {
                        file_path: path.to_string(),
                        name: name.to_string(),
                        kind: "struct".to_string(),
                        signature: trimmed.split('{').next().unwrap_or(trimmed).trim().to_string(),
                        line_number: i + 1,
                    });
                }
            } else if let Some(rest) = trimmed.strip_prefix("pub trait ").or_else(|| trimmed.strip_prefix("trait ")) {
                if let Some(name) = rest.split(['{', ':', '<']).next().map(str::trim) {
                    out.push(SymbolDef {
                        file_path: path.to_string(),
                        name: name.to_string(),
                        kind: "trait".to_string(),
                        signature: trimmed.split('{').next().unwrap_or(trimmed).trim().to_string(),
                        line_number: i + 1,
                    });
                }
            } else if let Some(rest) = trimmed.strip_prefix("pub enum ").or_else(|| trimmed.strip_prefix("enum ")) {
                if let Some(name) = rest.split(['{', '<']).next().map(str::trim) {
                    out.push(SymbolDef {
                        file_path: path.to_string(),
                        name: name.to_string(),
                        kind: "enum".to_string(),
                        signature: trimmed.split('{').next().unwrap_or(trimmed).trim().to_string(),
                        line_number: i + 1,
                    });
                }
            }
        }
    }

    fn extract_swift(content: &str, path: &str, out: &mut Vec<SymbolDef>) {
        for (i, line) in content.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.starts_with("//") {
                continue;
            }

            for prefix in &["func ", "class ", "struct ", "protocol ", "enum ", "actor "] {
                if let Some(pos) = trimmed.find(prefix) {
                    // Make sure it's preceded by access control or at start
                    let lead = trimmed[..pos].trim();
                    if lead.is_empty() || ["public", "private", "open", "fileprivate", "internal", "final", "mutating", "static"].contains(&lead) {
                        let rest = &trimmed[pos + prefix.len()..];
                        let name = rest.split(['(', '<', ':', '{']).next().unwrap_or("").trim();
                        if !name.is_empty() {
                            let kind = prefix.trim();
                            let sig = trimmed.split('{').next().unwrap_or(trimmed).trim();
                            out.push(SymbolDef {
                                file_path: path.to_string(),
                                name: name.to_string(),
                                kind: kind.to_string(),
                                signature: sig.to_string(),
                                line_number: i + 1,
                            });
                        }
                    }
                }
            }
        }
    }

    fn extract_typescript(content: &str, path: &str, out: &mut Vec<SymbolDef>) {
        for (i, line) in content.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.starts_with("//") {
                continue;
            }

            for prefix in &["export function ", "function ", "export class ", "class ", "export interface ", "interface ", "export type ", "type "] {
                if let Some(rest) = trimmed.strip_prefix(prefix) {
                    let name = rest.split(['(', '<', '=', '{', ' ']).next().unwrap_or("").trim();
                    if !name.is_empty() {
                        let kind = prefix.split_whitespace().last().unwrap_or("fn");
                        let sig = trimmed.split('{').next().unwrap_or(trimmed).trim();
                        out.push(SymbolDef {
                            file_path: path.to_string(),
                            name: name.to_string(),
                            kind: kind.to_string(),
                            signature: sig.to_string(),
                            line_number: i + 1,
                        });
                    }
                }
            }
        }
    }

    fn extract_python(content: &str, path: &str, out: &mut Vec<SymbolDef>) {
        for (i, line) in content.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.starts_with('#') {
                continue;
            }

            if let Some(rest) = trimmed.strip_prefix("def ") {
                let name = rest.split('(').next().unwrap_or("").trim();
                if !name.is_empty() {
                    let sig = trimmed.split(':').next().unwrap_or(trimmed).trim();
                    out.push(SymbolDef {
                        file_path: path.to_string(),
                        name: name.to_string(),
                        kind: "def".to_string(),
                        signature: sig.to_string(),
                        line_number: i + 1,
                    });
                }
            } else if let Some(rest) = trimmed.strip_prefix("class ") {
                let name = rest.split(['(', ':']).next().unwrap_or("").trim();
                if !name.is_empty() {
                    let sig = trimmed.split(':').next().unwrap_or(trimmed).trim();
                    out.push(SymbolDef {
                        file_path: path.to_string(),
                        name: name.to_string(),
                        kind: "class".to_string(),
                        signature: sig.to_string(),
                        line_number: i + 1,
                    });
                }
            }
        }
    }

    fn extract_go(content: &str, path: &str, out: &mut Vec<SymbolDef>) {
        for (i, line) in content.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.starts_with("//") {
                continue;
            }

            if let Some(rest) = trimmed.strip_prefix("func ") {
                let name = rest.split('(').next().unwrap_or("").trim();
                if !name.is_empty() {
                    let sig = trimmed.split('{').next().unwrap_or(trimmed).trim();
                    out.push(SymbolDef {
                        file_path: path.to_string(),
                        name: name.to_string(),
                        kind: "func".to_string(),
                        signature: sig.to_string(),
                        line_number: i + 1,
                    });
                }
            }
        }
    }
}

/// Personalized PageRank Engine for ranking symbols.
pub struct RepoMapEngine;

impl RepoMapEngine {
    /// Build graph from files and resolve intra-workspace reference edges.
    pub fn build_graph<P: AsRef<Path>>(root: P, files: &[PathBuf]) -> Result<SymbolGraph> {
        let mut graph = SymbolGraph::new();
        let root_p = root.as_ref();

        // 1. Extract definitions
        let mut file_contents = HashMap::new();
        for file in files {
            let full_path = if file.is_absolute() { file.clone() } else { root_p.join(file) };
            if full_path.is_file() {
                if let Ok(content) = fs::read_to_string(&full_path) {
                    let defs = SymbolExtractor::extract_from_file(&full_path)?;
                    for def in defs {
                        graph.add_def(def);
                    }
                    file_contents.insert(full_path.to_string_lossy().to_string(), content);
                }
            }
        }

        // 2. Edges: every symbol defined in a file references each symbol whose
        // name that file mentions as a whole identifier. One pass per file over
        // its identifiers — not a substring search per symbol per file, which is
        // O(symbols × files × bytes) and took seconds on a mid-size repo.
        let mut defs_by_file: HashMap<&str, Vec<usize>> = HashMap::new();
        for (j, def) in graph.definitions.iter().enumerate() {
            defs_by_file.entry(def.file_path.as_str()).or_default().push(j);
        }
        let mut edges = Vec::new();
        for (f_path, content) in &file_contents {
            let Some(from) = defs_by_file.get(f_path.as_str()) else {
                continue;
            };
            let mut seen: HashSet<&str> = HashSet::new();
            for word in identifiers(content) {
                if !seen.insert(word) {
                    continue;
                }
                if let Some(targets) = graph.symbol_to_index.get(word) {
                    for &i in targets {
                        for &j in from {
                            if j != i {
                                edges.push((j, i));
                            }
                        }
                    }
                }
            }
        }
        for (u, v) in edges {
            graph.add_reference(u, v);
        }

        Ok(graph)
    }

    /// Compute Personalized PageRank (PPR) over the SymbolGraph.
    /// `seed_files` receive the teleportation probability mass.
    pub fn compute_ppr(
        graph: &SymbolGraph,
        seed_files: &[String],
        damping_factor: f64,
        max_iterations: usize,
    ) -> Vec<f64> {
        let n = graph.definitions.len();
        if n == 0 {
            return Vec::new();
        }

        // Initialize seed probability vector p
        let mut p = vec![0.0; n];
        let mut seed_indices = Vec::new();

        for (idx, def) in graph.definitions.iter().enumerate() {
            if seed_files.iter().any(|sf| def.file_path.ends_with(sf) || sf.ends_with(&def.file_path)) {
                seed_indices.push(idx);
            }
        }

        if seed_indices.is_empty() {
            // Uniform personalization if no seed matched
            let uniform = 1.0 / (n as f64);
            p.fill(uniform);
        } else {
            let mass = 1.0 / (seed_indices.len() as f64);
            for &idx in &seed_indices {
                p[idx] = mass;
            }
        }

        // Power iteration
        let mut r = p.clone();
        let mut next_r = vec![0.0; n];

        for _ in 0..max_iterations {
            let mut dangling_mass = 0.0;

            for (i, &ri) in r.iter().enumerate() {
                let out_deg = graph.outgoing_edges.get(&i).map_or(0, |set| set.len());
                if out_deg == 0 {
                    dangling_mass += ri;
                }
            }

            for j in 0..n {
                let mut incoming_sum = 0.0;
                if let Some(in_edges) = graph.incoming_edges.get(&j) {
                    for &i in in_edges {
                        let out_deg = graph.outgoing_edges.get(&i).map_or(1, |set| set.len());
                        incoming_sum += r[i] / (out_deg as f64);
                    }
                }

                next_r[j] = (1.0 - damping_factor) * p[j]
                    + damping_factor * (incoming_sum + dangling_mass / (n as f64));
            }

            // Check convergence
            let mut diff = 0.0;
            for i in 0..n {
                diff += (next_r[i] - r[i]).abs();
            }
            r.copy_from_slice(&next_r);

            if diff < 1e-6 {
                break;
            }
        }

        r
    }

    /// Render a compact Repo Map formatted for LLM system prompt injection,
    /// adhering strictly to a token budget.
    pub fn render_repo_map(
        graph: &SymbolGraph,
        ranks: &[f64],
        root_dir: &Path,
        max_tokens: usize,
    ) -> String {
        Self::render_budgeted(graph, ranks, root_dir, max_tokens as i64, &HeuristicCounter)
    }

    /// [`render_repo_map`] measured with `counter`, so the map is costed the
    /// same way as every other tier it shares the budget with.
    pub fn render_budgeted<C: TokenCounter>(
        graph: &SymbolGraph,
        ranks: &[f64],
        root_dir: &Path,
        max_tokens: i64,
        counter: &C,
    ) -> String {
        if graph.definitions.is_empty() || ranks.len() != graph.definitions.len() {
            return String::new();
        }

        let mut ranked_indices: Vec<usize> = (0..graph.definitions.len()).collect();
        ranked_indices.sort_by(|&a, &b| ranks[b].total_cmp(&ranks[a]));

        // Fence + one header line per file are paid for as they appear.
        let mut used = counter.count("```repo-map\n```\n");
        let mut file_to_symbols: BTreeMap<String, Vec<&SymbolDef>> = BTreeMap::new();

        for idx in ranked_indices {
            let def = &graph.definitions[idx];
            let rel_path = Path::new(&def.file_path)
                .strip_prefix(root_dir)
                .unwrap_or(Path::new(&def.file_path))
                .to_string_lossy()
                .to_string();

            let mut cost = counter.count(&format!("  {} (L{})\n", def.signature, def.line_number));
            if !file_to_symbols.contains_key(&rel_path) {
                cost += counter.count(&format!("{rel_path}:\n"));
            }
            if used + cost > max_tokens {
                // Lower-ranked but shorter lines may still fit.
                continue;
            }
            file_to_symbols.entry(rel_path).or_default().push(def);
            used += cost;
        }
        if file_to_symbols.is_empty() {
            return String::new();
        }

        let mut output = String::from("```repo-map\n");
        for (file, mut defs) in file_to_symbols {
            defs.sort_by_key(|d| d.line_number);
            output.push_str(&format!("{}:\n", file));
            for def in defs {
                output.push_str(&format!("  {} (L{})\n", def.signature, def.line_number));
            }
        }
        output.push_str("```\n");
        output
    }
}

/// Identifier-shaped words in `text` (ASCII letters, digits, `_`, not
/// starting with a digit) — what a symbol name can be in every language the
/// extractor reads.
fn identifiers(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| w.len() > 1 && !w.as_bytes()[0].is_ascii_digit())
}

/// How much one file using `name` says about the definitions of it.
/// Without this the map was a list of `new`/`default`/`match`: an identifier
/// scan cannot tell a call from the same English word in a comment, so the
/// name itself has to carry the evidence (Aider weighs names the same way).
/// - defined in more than 3 places: names nothing in particular → 0
/// - split across overloads, so one name counts once
/// - `snake_case` / `camelCase` / `PascalCase` of 8+ chars is almost never
///   prose → ×10; a plain lowercase word often is → ×0.1
fn name_weight(name: &str, defined: usize) -> f64 {
    if defined > 3 {
        return 0.0;
    }
    let has_upper = name.chars().any(|c| c.is_ascii_uppercase());
    let has_lower = name.chars().any(|c| c.is_ascii_lowercase());
    let compound = name.trim_matches('_').contains('_') || (has_upper && has_lower);
    let style = if compound && name.len() >= 8 {
        10.0
    } else if !has_upper && !name.contains('_') {
        0.1
    } else {
        1.0
    };
    style / defined as f64
}

/// Source files larger than this are left out of the map: generated code and
/// vendored bundles, whose symbols would crowd out the ones people edit.
const MAX_FILE_BYTES: u64 = 256 * 1024;
/// Upper bound on files scanned per build, so a monorepo costs a bounded
/// first build instead of a turn that never starts.
const MAX_FILES: usize = 3000;

fn is_mapped_source(path: &str) -> bool {
    matches!(
        Path::new(path).extension().and_then(|e| e.to_str()),
        Some("rs" | "swift" | "ts" | "tsx" | "js" | "jsx" | "py" | "go")
    )
}

/// The repo map as context, kept across turns. Rebuilt only when the set of
/// source files or one of their sizes/mtimes changes; the ranking is redone
/// every call because the seeds (files this session touched) change.
///
/// Ranks files, not symbols (as Aider does): a symbol-level graph links every
/// symbol of a file to every name it mentions, which on this repo's own
/// ContentView.swift is millions of edges and ~300 ms of PageRank per turn.
/// Over ~600 files it is a few milliseconds, and each file's rank is then
/// handed to the symbols it references.
///
/// Storage ≠ context (CLAUDE.md 2.1): nothing here is ever written to disk.
#[derive(Default)]
pub struct RepoMapCache {
    stamp: Vec<(String, u64, u128)>,
    /// Only `definitions` is filled — what rendering needs.
    graph: SymbolGraph,
    /// Workspace-relative path of each file in `stamp` order.
    files: Vec<String>,
    /// Index into `files` of the file each definition is in.
    def_file: Vec<usize>,
    /// For each file, the definitions (in other files) whose names it uses,
    /// with how much a use of that name says (see [`name_weight`]).
    refs: Vec<Vec<(usize, f64)>>,
}

impl RepoMapCache {
    /// A map of `files` (paths relative to `root`) within `max_tokens`,
    /// ranked towards `seeds`. Empty when there is nothing to map or no room.
    pub fn render<C: TokenCounter>(
        &mut self,
        root: &Path,
        files: &[String],
        seeds: &[String],
        max_tokens: i64,
        counter: &C,
    ) -> String {
        if max_tokens <= 0 {
            return String::new();
        }
        let mut stamp: Vec<(String, u64, u128)> = files
            .iter()
            .filter(|f| is_mapped_source(f))
            .filter_map(|f| {
                let meta = fs::metadata(root.join(f)).ok()?;
                if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
                    return None;
                }
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_nanos());
                Some((f.clone(), meta.len(), mtime))
            })
            .collect();
        stamp.sort();
        stamp.truncate(MAX_FILES);
        if stamp.is_empty() {
            return String::new();
        }
        if stamp != self.stamp {
            self.rebuild(root, stamp);
        }
        let ranks = self.rank(seeds);
        RepoMapEngine::render_budgeted(&self.graph, &ranks, root, max_tokens, counter)
    }

    fn rebuild(&mut self, root: &Path, stamp: Vec<(String, u64, u128)>) {
        let mut graph = SymbolGraph::new();
        let mut def_file = Vec::new();
        let mut contents = Vec::with_capacity(stamp.len());
        for (fi, (f, _, _)) in stamp.iter().enumerate() {
            let full = root.join(f);
            // A file that vanished or is not UTF-8 maps as empty.
            let content = fs::read_to_string(&full).unwrap_or_default();
            if let Ok(defs) = SymbolExtractor::extract_from_file(&full) {
                for def in defs {
                    graph.add_def(def);
                    def_file.push(fi);
                }
            }
            contents.push(content);
        }
        // Each file's distinct identifiers, and in how many files each
        // defined name appears — a name in most files says little (IDF).
        let words: Vec<HashSet<&str>> = contents.iter().map(|c| identifiers(c).collect()).collect();
        let mut df: HashMap<&str, usize> = HashMap::new();
        for set in &words {
            for &w in set {
                if graph.symbol_to_index.contains_key(w) {
                    *df.entry(w).or_default() += 1;
                }
            }
        }
        let n_files = words.len() as f64;
        let refs = words
            .iter()
            .enumerate()
            .map(|(fi, set)| {
                let mut out = Vec::new();
                for &word in set {
                    let Some(targets) = graph.symbol_to_index.get(word) else {
                        continue;
                    };
                    let idf = (n_files / df.get(word).copied().unwrap_or(1) as f64).ln();
                    let w = name_weight(word, targets.len()) * idf.max(0.0);
                    if w > 0.0 {
                        out.extend(
                            targets.iter().copied().filter(|&d| def_file[d] != fi).map(|d| (d, w)),
                        );
                    }
                }
                out
            })
            .collect();
        self.files = stamp.iter().map(|(f, _, _)| f.clone()).collect();
        self.stamp = stamp;
        self.graph = graph;
        self.def_file = def_file;
        self.refs = refs;
    }

    /// Personalized PageRank over files (edge f → g per symbol of g that f
    /// uses), then each symbol scores the rank its users pass to it — plus a
    /// sliver of its own file's rank, so a seed file's unreferenced
    /// definitions (an entry point) still rank above unrelated files.
    fn rank(&self, seeds: &[String]) -> Vec<f64> {
        let n = self.files.len();
        let d_count = self.graph.definitions.len();
        if n == 0 || d_count == 0 {
            return Vec::new();
        }
        let mut p = vec![0.0; n];
        let seeded: Vec<usize> = (0..n)
            .filter(|&i| seeds.iter().any(|s| s == &self.files[i] || s.ends_with(&self.files[i])))
            .collect();
        if seeded.is_empty() {
            p.fill(1.0 / n as f64);
        } else {
            for &i in &seeded {
                p[i] = 1.0 / seeded.len() as f64;
            }
        }
        let totals: Vec<f64> = self.refs.iter().map(|rs| rs.iter().map(|&(_, w)| w).sum()).collect();
        let damping = 0.85;
        let mut r = p.clone();
        for _ in 0..25 {
            let mut next = vec![0.0; n];
            let mut dangling = 0.0;
            for ((refs, &total), &rf) in self.refs.iter().zip(&totals).zip(&r) {
                if total <= 0.0 {
                    dangling += rf;
                    continue;
                }
                for &(d, w) in refs {
                    next[self.def_file[d]] += rf * w / total;
                }
            }
            let mut diff = 0.0;
            for ((v, &pf), &rf) in next.iter_mut().zip(&p).zip(&r) {
                *v = (1.0 - damping) * pf + damping * (*v + dangling * pf);
                diff += (*v - rf).abs();
            }
            r = next;
            if diff < 1e-9 {
                break;
            }
        }
        let mut score: Vec<f64> = (0..d_count).map(|d| 1e-3 * r[self.def_file[d]]).collect();
        for ((refs, &total), &rf) in self.refs.iter().zip(&totals).zip(&r) {
            if total <= 0.0 {
                continue;
            }
            for &(d, w) in refs {
                score[d] += rf * w / total;
            }
        }
        score
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn extract_rust_and_rank_symbols() {
        let dir = tempdir().unwrap();
        let path = dir.path();

        let code_a = r#"
pub struct UserAuth {
    pub token: String,
}

pub fn login(user: &UserAuth) -> bool {
    true
}
"#;
        let code_b = r#"
pub fn run_session() {
    let auth = UserAuth { token: "abc".into() };
    login(&auth);
}
"#;
        fs::write(path.join("auth.rs"), code_a).unwrap();
        fs::write(path.join("session.rs"), code_b).unwrap();

        let files = vec![PathBuf::from("auth.rs"), PathBuf::from("session.rs")];
        let graph = RepoMapEngine::build_graph(path, &files).unwrap();

        assert_eq!(graph.definitions.len(), 3); // UserAuth, login, run_session

        // Personalize on session.rs
        let ranks = RepoMapEngine::compute_ppr(
            &graph,
            &["session.rs".to_string()],
            0.85,
            30,
        );
        assert_eq!(ranks.len(), 3);

        let map = RepoMapEngine::render_repo_map(&graph, &ranks, path, 500);
        assert!(map.contains("session.rs:"));
        assert!(map.contains("auth.rs:"));
        assert!(map.contains("pub fn login"));
    }

    #[test]
    fn cache_respects_budget_ranks_seeds_and_rebuilds_on_edit() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("auth.rs"), "pub fn login() -> bool { true }\n").unwrap();
        fs::write(root.join("cli.rs"), "pub fn main_entry() { login(); }\n").unwrap();
        fs::write(root.join("notes.md"), "pub fn not_code() {}\n").unwrap();
        let files: Vec<String> = ["auth.rs", "cli.rs", "notes.md"].map(String::from).to_vec();
        let counter = HeuristicCounter;

        let mut cache = RepoMapCache::default();
        let map = cache.render(root, &files, &["cli.rs".into()], 200, &counter);
        assert!(map.starts_with("```repo-map\n"), "{map}");
        assert!(map.contains("auth.rs:") && map.contains("pub fn login() -> bool"));
        assert!(!map.contains("not_code"), "only source files are mapped");
        assert!(counter.count(&map) <= 200);

        // No room: nothing, not a truncated fence.
        assert_eq!(cache.render(root, &files, &[], 3, &counter), "");

        // An edit shows up on the next call without a new cache.
        fs::write(root.join("auth.rs"), "pub fn login() -> bool { true }\npub fn logout() {}\n").unwrap();
        let map = cache.render(root, &files, &[], 200, &counter);
        assert!(map.contains("pub fn logout()"), "{map}");
    }

    #[test]
    fn identifiers_are_whole_words() {
        let words: Vec<&str> = identifiers("login(user); relogin_now 9x _a b").collect();
        assert_eq!(words, ["login", "user", "relogin_now", "_a"]);
    }
}
