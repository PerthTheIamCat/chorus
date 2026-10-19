//! High-Density Repository Map and Personalized PageRank (PPR) Engine.
//!
//! Inspired by Aider and RepoGraph research, this module builds a Code Property
//! Graph of symbols (functions, structs, classes, traits, protocols), computes
//! Personalized PageRank seeded on active files, and formats a dense, budget-bounded
//! structural map (typically ~1,024 tokens) for prompt context injection.

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

        // 2. Scan file contents for occurrences of other symbols to build edges
        let mut edges = Vec::new();
        let n = graph.definitions.len();
        for i in 0..n {
            let def_name = &graph.definitions[i].name;
            // Find which other files mention this def_name
            for (f_path, content) in &file_contents {
                if content.contains(def_name) {
                    // Link all symbols defined in f_path to symbol i
                    for j in 0..n {
                        if &graph.definitions[j].file_path == f_path && j != i {
                            edges.push((j, i));
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
    /// adhering strictly to a token budget (~4 chars per token).
    pub fn render_repo_map(
        graph: &SymbolGraph,
        ranks: &[f64],
        root_dir: &Path,
        max_tokens: usize,
    ) -> String {
        if graph.definitions.is_empty() || ranks.is_empty() {
            return String::new();
        }

        // Rank symbols in descending order
        let mut ranked_indices: Vec<usize> = (0..graph.definitions.len()).collect();
        ranked_indices.sort_by(|&a, &b| ranks[b].partial_cmp(&ranks[a]).unwrap());

        // Group by relative file path
        let max_chars = max_tokens * 4;
        let mut current_chars = 0;
        let mut file_to_symbols: BTreeMap<String, Vec<&SymbolDef>> = BTreeMap::new();

        for idx in ranked_indices {
            let def = &graph.definitions[idx];
            let rel_path = Path::new(&def.file_path)
                .strip_prefix(root_dir)
                .unwrap_or(Path::new(&def.file_path))
                .to_string_lossy()
                .to_string();

            let line_cost = def.signature.len() + 10;
            if current_chars + line_cost > max_chars {
                break;
            }

            file_to_symbols
                .entry(rel_path)
                .or_default()
                .push(def);
            current_chars += line_cost;
        }

        let mut output = String::from("```repo-map\n");
        for (file, defs) in file_to_symbols {
            output.push_str(&format!("{}:\n", file));
            for def in defs {
                output.push_str(&format!("  {} (L{})\n", def.signature, def.line_number));
            }
        }
        output.push_str("```\n");
        output
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
}
