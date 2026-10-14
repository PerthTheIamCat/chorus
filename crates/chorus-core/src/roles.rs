//! Custom agent roles and AI role synthesis.
//!
//! Custom roles define specialized agents with tailored system prompts, tool
//! boundaries, and model preferences. They are loaded from `.chorus/agents/*.md`
//! files with YAML/Markdown frontmatter, falling back to built-in presets
//! (orchestrator, investigator, builder, reviewer).
//!
//! The AI Role Generator ([`synthesize_role`]) dynamically drafts ad-hoc roles
//! for orchestrating agents delegating novel subtasks.

use std::path::{Path, PathBuf};

/// Where project-level custom agent definitions live, relative to workspace root.
pub const AGENTS_DIR: &str = ".chorus/agents";

/// A defined agent role specification.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AgentRole {
    /// Identifier / filename slug, e.g. "code-reviewer" or "test-engineer".
    pub name: String,
    /// Brief explanation of the role's purpose and when to use it.
    pub description: String,
    /// Core system prompt handed to the model when spawned in this role.
    pub system_prompt: String,
    /// Tools this role is permitted to use, e.g. `["read", "bash", "diff"]`.
    pub tools: Vec<String>,
    /// Optional model preference for this role.
    pub model: Option<String>,
}

impl AgentRole {
    /// Parse an agent role definition from markdown with optional frontmatter.
    ///
    /// If frontmatter is absent, `fallback_name` is used as the role name,
    /// the first non-heading prose line becomes the description, and the
    /// full text becomes the system prompt.
    pub fn parse(content: &str, fallback_name: &str) -> Self {
        let trimmed = content.trim();
        if !trimmed.starts_with("---") {
            let (desc, _) = extract_first_prose(trimmed);
            return Self {
                name: fallback_name.to_string(),
                description: if desc.is_empty() {
                    format!("Agent role {fallback_name}")
                } else {
                    desc
                },
                system_prompt: trimmed.to_string(),
                tools: default_tools(false),
                model: None,
            };
        }

        // Parse YAML frontmatter between leading `---` and closing `---`
        let mut lines = trimmed.lines();
        let _ = lines.next(); // skip opening ---

        let mut frontmatter_lines = Vec::new();
        let mut body_lines = Vec::new();
        let mut in_front = true;

        for line in lines {
            if in_front {
                if line.trim() == "---" {
                    in_front = false;
                } else {
                    frontmatter_lines.push(line);
                }
            } else {
                body_lines.push(line);
            }
        }

        let mut name = String::new();
        let mut description = String::new();
        let mut front_prompt = String::new();
        let mut tools = Vec::new();
        let mut model = None;

        let mut i = 0;
        while i < frontmatter_lines.len() {
            let line = frontmatter_lines[i].trim();
            if let Some(rest) = line.strip_prefix("name:") {
                name = unquote(rest);
            } else if let Some(rest) = line.strip_prefix("description:") {
                description = unquote(rest);
            } else if let Some(rest) = line.strip_prefix("system_prompt:") {
                front_prompt = unquote(rest);
            } else if let Some(rest) = line.strip_prefix("model:") {
                let m = unquote(rest);
                if !m.is_empty() {
                    model = Some(m);
                }
            } else if let Some(rest) = line.strip_prefix("tools:") {
                let rest_trim = rest.trim();
                if rest_trim.starts_with('[') && rest_trim.ends_with(']') {
                    let inner = &rest_trim[1..rest_trim.len() - 1];
                    for item in inner.split(',') {
                        let t = unquote(item);
                        if !t.is_empty() {
                            tools.push(t);
                        }
                    }
                } else if !rest_trim.is_empty() {
                    for item in rest_trim.split(',') {
                        let t = unquote(item);
                        if !t.is_empty() {
                            tools.push(t);
                        }
                    }
                } else {
                    // Check for indented list items: ` - item`
                    while i + 1 < frontmatter_lines.len() {
                        let next_line = frontmatter_lines[i + 1].trim();
                        if let Some(item) = next_line.strip_prefix('-') {
                            let t = unquote(item);
                            if !t.is_empty() {
                                tools.push(t);
                            }
                            i += 1;
                        } else {
                            break;
                        }
                    }
                }
            }
            i += 1;
        }

        let body = body_lines.join("\n").trim().to_string();
        let system_prompt = if !body.is_empty() {
            body
        } else if !front_prompt.is_empty() {
            front_prompt
        } else {
            description.clone()
        };

        if name.is_empty() {
            name = fallback_name.to_string();
        }
        if description.is_empty() {
            let (desc, _) = extract_first_prose(&system_prompt);
            description = if desc.is_empty() {
                format!("Agent role {name}")
            } else {
                desc
            };
        }
        if tools.is_empty() {
            tools = default_tools(false);
        }

        Self {
            name,
            description,
            system_prompt,
            tools,
            model,
        }
    }

    /// Render this role specification to standard YAML frontmatter + markdown format.
    pub fn to_markdown(&self) -> String {
        let tools_repr: Vec<String> = self.tools.iter().map(|t| format!("\"{t}\"")).collect();
        let mut s = format!(
            "---\nname: {}\ndescription: \"{}\"\ntools: [{}]\n",
            self.name,
            self.description.replace('"', "\\\""),
            tools_repr.join(", ")
        );
        if let Some(ref m) = self.model {
            s.push_str(&format!("model: \"{m}\"\n"));
        }
        s.push_str("---\n\n");
        s.push_str(self.system_prompt.trim());
        s.push('\n');
        s
    }

    /// Save this role definition to `.chorus/agents/<name>.md` under `dir`.
    pub fn save_to_dir(&self, dir: &Path) -> std::io::Result<PathBuf> {
        let agents_dir = dir.join(AGENTS_DIR);
        std::fs::create_dir_all(&agents_dir)?;
        let file_path = agents_dir.join(format!("{}.md", self.name));
        std::fs::write(&file_path, self.to_markdown())?;
        Ok(file_path)
    }
}

fn unquote(s: &str) -> String {
    s.trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '`')
        .trim()
        .to_string()
}

fn extract_first_prose(text: &str) -> (String, String) {
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') || t.starts_with("---") {
            continue;
        }
        let clipped: String = t.chars().take(160).collect();
        return (clipped, t.to_string());
    }
    (String::new(), String::new())
}

fn default_tools(write_mode: bool) -> Vec<String> {
    if write_mode {
        vec![
            "read".into(),
            "write".into(),
            "edit".into(),
            "bash".into(),
            "diff".into(),
        ]
    } else {
        vec!["read".into(), "grep".into(), "find".into(), "diff".into()]
    }
}

/// The built-in baseline agent roles Chorus provides out of the box.
pub fn builtin_roles() -> Vec<AgentRole> {
    vec![
        AgentRole {
            name: "orchestrator".into(),
            description: "Coordinates complex workflows by breaking down objectives and steering child subagents."
                .into(),
            system_prompt: "You are the Lead Orchestrator agent for Chorus. Your mandate is to \
decompose high-level project goals into focused subtasks, spawn specialized subagents \
(e.g., investigator, builder, reviewer), monitor their progress, and aggregate their findings into \
a coherent final outcome. Keep your own context lean by relying on child agent summary briefs."
                .into(),
            tools: vec![
                "chorus_spawn_agent".into(),
                "chorus_send_message".into(),
                "chorus_wait_agent".into(),
                "chorus_collect_results".into(),
                "chorus_cancel_agent".into(),
                "chorus_update_checklist".into(),
                "chorus_jump_to_file".into(),
                "chorus_request_review".into(),
            ],
            model: None,
        },
        AgentRole {
            name: "investigator".into(),
            description: "Codebase exploration, architectural tracing, and root-cause bug diagnosis."
                .into(),
            system_prompt: "You are a specialized Investigator subagent. Your mission is to explore \
the codebase, search symbols and references, trace execution flows, and diagnose bugs. \
You operate in read-only mode: do not modify repository files. Return a concise, high-signal brief \
detailing your findings, file locations, and recommendations."
                .into(),
            tools: vec!["read".into(), "grep".into(), "find".into(), "diff".into()],
            model: None,
        },
        AgentRole {
            name: "builder".into(),
            description: "Implements focused code modifications, verifies builds, and runs unit tests."
                .into(),
            system_prompt: "You are a specialized Builder subagent. Your mission is to implement \
clean, minimal, and correct code changes according to your task instructions. Write unit tests \
for all new functionality, run local verification commands, and report a clear diff summary upon completion."
                .into(),
            tools: vec![
                "read".into(),
                "write".into(),
                "edit".into(),
                "bash".into(),
                "diff".into(),
            ],
            model: None,
        },
        AgentRole {
            name: "reviewer".into(),
            description: "Audits diffs for correctness, security vulnerabilities, edge cases, and test coverage."
                .into(),
            system_prompt: "You are a specialized Reviewer subagent. Your mandate is to rigorously \
critique proposed diffs, checking for correctness, security issues, performance regressions, \
and boundary conditions. Provide clear, actionable feedback with specific line numbers."
                .into(),
            tools: vec!["read".into(), "grep".into(), "diff".into()],
            model: None,
        },
    ]
}

/// Load all available agent roles for the workspace at `cwd`.
///
/// Discovers any `.chorus/agents/*.md` definitions and merges them with the
/// built-in roles. Custom definitions override built-in roles if their names match.
pub fn load_roles(cwd: &Path) -> Vec<AgentRole> {
    let mut roles = builtin_roles();
    let agents_path = cwd.join(AGENTS_DIR);

    if let Ok(entries) = std::fs::read_dir(&agents_path) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("md") {
                let fallback = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("custom-agent");
                if let Ok(content) = std::fs::read_to_string(&path) {
                    let custom = AgentRole::parse(&content, fallback);
                    // Replace built-in with same name, or add new
                    if let Some(pos) = roles.iter().position(|r| r.name == custom.name) {
                        roles[pos] = custom;
                    } else {
                        roles.push(custom);
                    }
                }
            }
        }
    }

    roles.sort_by(|a, b| a.name.cmp(&b.name));
    roles
}

/// Look up a specific role by name in `cwd`, returning the role if found.
pub fn find_role(cwd: &Path, name: &str) -> Option<AgentRole> {
    let target = name.trim().to_lowercase();
    load_roles(cwd)
        .into_iter()
        .find(|r| r.name.to_lowercase() == target)
}

/// AI Role Generator: Synthesize an optimized custom subagent role definition
/// for a specialized task.
///
/// Infers role identity, operational boundaries, appropriate tool permissions,
/// and tailored system instructions from the task description.
pub fn synthesize_role(task: &str, write_mode: bool, name_hint: Option<&str>) -> AgentRole {
    let task_lower = task.to_lowercase();

    let name = if let Some(hint) = name_hint {
        slugify(hint)
    } else if task_lower.contains("database")
        || task_lower.contains("postgres")
        || task_lower.contains("migration")
        || task_lower.contains("sql")
        || task_lower.contains("schema")
    {
        "database-specialist".into()
    } else if task_lower.contains("review")
        || task_lower.contains("audit")
        || task_lower.contains("security")
    {
        "security-reviewer".into()
    } else if task_lower.contains("test")
        || task_lower.contains("spec")
        || task_lower.contains("bench")
    {
        "test-engineer".into()
    } else if task_lower.contains("refactor") || task_lower.contains("clean") {
        "refactoring-specialist".into()
    } else if task_lower.contains("ui")
        || task_lower.contains("css")
        || task_lower.contains("frontend")
        || task_lower.contains("component")
    {
        "ui-component-engineer".into()
    } else if task_lower.contains("docs")
        || task_lower.contains("readme")
        || task_lower.contains("changelog")
    {
        "documentation-writer".into()
    } else if task_lower.contains("bug")
        || task_lower.contains("fix")
        || task_lower.contains("patch")
    {
        "bug-fixer".into()
    } else if task_lower.contains("investigate")
        || task_lower.contains("explore")
        || task_lower.contains("research")
    {
        "codebase-investigator".into()
    } else if write_mode {
        "specialist-builder".into()
    } else {
        "domain-analyst".into()
    };

    let title_cased = name
        .split('-')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");

    let description = format!("Specialized agent synthesized for: {}", clip(task, 120));

    let tools = default_tools(write_mode);

    let system_prompt = format!(
        "You are the {title_cased} subagent for Chorus.\n\n\
         PRIMARY MISSION:\n\
         {task}\n\n\
         OPERATIONAL DIRECTIVES:\n\
         1. Focus strictly on the assigned task without diverging into unrelated cleanup.\n\
         2. {boundary_rule}\n\
         3. Keep your communications concise. When reporting results, provide a compact summary, \
            diff statistics, and the exact file paths examined or modified.\n\
         4. Verify all findings or code changes thoroughly before handoff.",
        boundary_rule = if write_mode {
            "Make minimal, high-precision changes. Run project verification commands to ensure no breakage."
        } else {
            "Operate in read-only mode: inspect files and trace execution without making modifications."
        }
    );

    AgentRole {
        name,
        description,
        system_prompt,
        tools,
        model: None,
    }
}

fn slugify(text: &str) -> String {
    let slug: String = text
        .chars()
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    let cleaned: Vec<&str> = slug.split('-').filter(|s| !s.is_empty()).collect();
    if cleaned.is_empty() {
        "custom-agent".to_string()
    } else {
        cleaned.join("-")
    }
}

fn clip(text: &str, max_len: usize) -> String {
    let t = text.trim();
    if t.chars().count() <= max_len {
        t.to_string()
    } else {
        let truncated: String = t.chars().take(max_len.saturating_sub(3)).collect();
        format!("{truncated}...")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_yaml_frontmatter_role() {
        let md = r#"---
name: security-auditor
description: "Audits code for vulnerabilities and leaked secrets"
tools: ["read", "grep", "diff"]
model: "claude-3-7-sonnet"
---

# Security Auditor
Inspect authentication barriers and cryptographically sensitive flows.
"#;
        let role = AgentRole::parse(md, "fallback");
        assert_eq!(role.name, "security-auditor");
        assert_eq!(
            role.description,
            "Audits code for vulnerabilities and leaked secrets"
        );
        assert_eq!(role.tools, vec!["read", "grep", "diff"]);
        assert_eq!(role.model.as_deref(), Some("claude-3-7-sonnet"));
        assert!(role.system_prompt.contains("Inspect authentication barriers"));
    }

    #[test]
    fn parse_bare_markdown_role() {
        let md = "Investigate SQLite database schema issues and report indexes.";
        let role = AgentRole::parse(md, "db-helper");
        assert_eq!(role.name, "db-helper");
        assert_eq!(role.description, md);
        assert_eq!(role.system_prompt, md);
    }

    #[test]
    fn roundtrip_to_markdown() {
        let original = AgentRole {
            name: "api-designer".into(),
            description: "Designs REST and MCP interfaces".into(),
            system_prompt: "Define clear endpoint contracts and JSON schemas.".into(),
            tools: vec!["read".into(), "edit".into()],
            model: Some("gpt-4o".into()),
        };

        let rendered = original.to_markdown();
        let parsed = AgentRole::parse(&rendered, "other");
        assert_eq!(parsed.name, original.name);
        assert_eq!(parsed.description, original.description);
        assert_eq!(parsed.tools, original.tools);
        assert_eq!(parsed.model, original.model);
        assert_eq!(parsed.system_prompt, original.system_prompt);
    }

    #[test]
    fn load_roles_merges_with_builtins() {
        let temp = tempfile::tempdir().unwrap();
        let custom_dir = temp.path().join(AGENTS_DIR);
        std::fs::create_dir_all(&custom_dir).unwrap();

        let custom_file = custom_dir.join("investigator.md");
        std::fs::write(
            custom_file,
            "---\nname: investigator\ndescription: \"Custom repo investigator\"\ntools: [read]\n---\nCustom prompt",
        )
        .unwrap();

        let roles = load_roles(temp.path());
        assert!(roles.len() >= 4);

        let inv = roles.iter().find(|r| r.name == "investigator").unwrap();
        assert_eq!(inv.description, "Custom repo investigator");
        assert_eq!(inv.system_prompt, "Custom prompt");
    }

    #[test]
    fn synthesize_role_creates_tailored_specialist() {
        let role = synthesize_role(
            "Audit postgres migrations for missing indices and lock contention",
            false,
            None,
        );
        assert_eq!(role.name, "database-specialist");
        assert!(role.description.contains("Audit postgres migrations"));
        assert!(role.system_prompt.contains("Database Specialist"));
        assert!(!role.tools.contains(&"write".to_string()));

        let writer = synthesize_role("Implement new auth middleware and unit tests", true, None);
        assert!(writer.tools.contains(&"write".to_string()));
    }
}
