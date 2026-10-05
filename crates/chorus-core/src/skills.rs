//! Skill registry — one set of skills, projected into whatever shape each
//! agent can actually read.
//!
//! The third promise this app makes, after one session and one memory: every
//! agent sees the same skills. Today that promise is broken by the tools
//! themselves, not by the user — Claude Code loads `SKILL.md` folders from
//! `.claude/skills/`, and Codex has no skill mechanism at all. Keeping a set
//! of skills usable from both means maintaining two copies by hand, which is
//! the same chore as maintaining `CLAUDE.md` and `AGENTS.md` separately.
//!
//! So this works exactly like [`crate::projector`]: one canonical source in
//! `.chorus/skills/`, rendered out per agent before a turn.
//!
//! - **Claude** gets real skills: each one written to
//!   `.claude/skills/<name>/SKILL.md`, where its loader already looks
//!   (verified against code.claude.com/docs/en/skills, 12 Sep 2026).
//! - **Codex and Gemini** have nowhere to put a skill, so they get a compact
//!   index in their instruction file instead: name, one line of description,
//!   and the path. That is enough for an agent with a file reader to open the
//!   one it needs, and it costs a line per skill rather than a whole document.
//!
//! Pure string and directory work — no ledger, no network.

use std::path::{Path, PathBuf};

/// Where the canonical skills live, relative to the workspace root.
pub const SKILLS_DIR: &str = ".chorus/skills";

/// Where Claude Code looks for project skills.
pub const CLAUDE_SKILLS_DIR: &str = ".claude/skills";

/// One skill as the registry sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    /// `name:` from the frontmatter, else the directory name.
    pub name: String,
    /// `description:` from the frontmatter, else the first non-empty line of
    /// the body. This is the line every agent sees, so it is the one part
    /// that has to carry "when would I use this".
    pub description: String,
    /// The skill's own directory under [`SKILLS_DIR`].
    pub dir: PathBuf,
    /// The whole `SKILL.md`, verbatim.
    pub body: String,
}

impl Skill {
    /// Where this skill is written for Claude, relative to the workspace.
    pub fn claude_path(&self) -> PathBuf {
        Path::new(CLAUDE_SKILLS_DIR).join(&self.name).join("SKILL.md")
    }

    /// The path an agent without a skill loader should be told to read.
    pub fn source_path(&self) -> PathBuf {
        self.dir.join("SKILL.md")
    }
}

/// Read every skill under `<cwd>/.chorus/skills/*/SKILL.md`.
///
/// Sorted by name so the projected index is stable: an unstable order would
/// rewrite the agent files on every turn and make every diff look like a
/// change. Unreadable or nameless entries are skipped rather than failing the
/// turn — a broken skill folder must not stop someone from working.
pub fn discover(cwd: &Path) -> Vec<Skill> {
    let root = cwd.join(SKILLS_DIR);
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut out: Vec<Skill> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let dir = e.path();
            let body = std::fs::read_to_string(dir.join("SKILL.md")).ok()?;
            let dir_name = dir.file_name()?.to_string_lossy().into_owned();
            let (name, description) = parse_front_matter(&body, &dir_name);
            Some(Skill {
                name,
                description,
                dir: PathBuf::from(SKILLS_DIR).join(&dir_name),
                body,
            })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// `(name, description, dir)` for every `*/SKILL.md` under an arbitrary root.
///
/// For listing skills that are not ours to project — the user's global
/// `~/.claude/skills`, or a project's own `.claude/skills` — so Settings can
/// show everything an agent could see. Sorted by name, unreadable entries skipped.
pub fn list_dir(root: &Path) -> Vec<(String, String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out: Vec<_> = entries
        .flatten()
        .filter_map(|e| {
            // `is_dir` follows symlinks, which is how most global skills are installed.
            let dir = e.path();
            if !dir.is_dir() {
                return None;
            }
            let body = std::fs::read_to_string(dir.join("SKILL.md")).ok()?;
            let dir_name = dir.file_name()?.to_string_lossy().into_owned();
            let (name, description) = parse_front_matter(&body, &dir_name);
            Some((name, description, dir))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Pull `name` and `description` out of a leading `---` block.
///
/// A deliberately small parser, not a YAML dependency: the frontmatter we
/// care about is two scalar keys, and the file belongs to the user — a full
/// parser would give us more ways to reject their file, not more value. Keys
/// we do not understand are left alone, and a file with no frontmatter still
/// works: the directory name and the first real line stand in.
fn parse_front_matter(body: &str, dir_name: &str) -> (String, String) {
    let mut name = String::new();
    let mut description = String::new();

    if body.starts_with("---") {
        for line in body.lines().skip(1) {
            if line.trim() == "---" {
                break;
            }
            if let Some(v) = line.strip_prefix("name:") {
                name = unquote(v);
            } else if let Some(v) = line.strip_prefix("description:") {
                description = unquote(v);
            }
        }
    }
    if name.is_empty() {
        name = dir_name.to_string();
    }
    if description.is_empty() {
        description = first_prose_line(body);
    }
    (name, description)
}

fn unquote(v: &str) -> String {
    v.trim().trim_matches(|c| c == '"' || c == '\'').trim().to_string()
}

/// The first line that is neither frontmatter nor a heading — the closest
/// thing to a description a file without one has.
fn first_prose_line(body: &str) -> String {
    let mut in_front = body.starts_with("---");
    for (i, line) in body.lines().enumerate() {
        let t = line.trim();
        if in_front {
            if i > 0 && t == "---" {
                in_front = false;
            }
            continue;
        }
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        return t.chars().take(160).collect();
    }
    String::new()
}

/// The compact index handed to agents that cannot load skills themselves.
///
/// One row per skill, header once — the table shape from the storage spec,
/// for the same reason: repeating a JSON key per skill is paying tokens to
/// say nothing. Empty when there are no skills, so the caller can leave the
/// whole section out rather than render an empty heading.
pub fn index(skills: &[Skill]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let mut s = String::from(
        "Skills available in this workspace. Read the file before using one.\n\n\
         name | when to use | path\n",
    );
    for k in skills {
        s.push_str(&format!(
            "{} | {} | {}\n",
            k.name,
            k.description.replace('|', "/"),
            k.source_path().display()
        ));
    }
    s
}

/// Is `path` (workspace-relative) a file the skill projector writes?
///
/// The revert guard and the "files the agent edited" list both need this:
/// a projected skill is Chorus's own output, and blaming an agent for it is
/// the same bug the `CLAUDE.md` filter already fixed once.
pub fn is_projected(path: &str) -> bool {
    path.starts_with(CLAUDE_SKILLS_DIR)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, dir: &str, body: &str) {
        let d = root.join(SKILLS_DIR).join(dir);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("SKILL.md"), body).unwrap();
    }

    #[test]
    fn frontmatter_wins_and_missing_fields_fall_back() {
        let t = tempfile::tempdir().unwrap();
        write(
            t.path(),
            "review",
            "---\nname: code-review\ndescription: \"Review a diff for security\"\n---\n\nSteps...\n",
        );
        write(t.path(), "bare", "# Deploy\n\nPush the thing to staging.\n");

        let found = discover(t.path());
        assert_eq!(found.len(), 2);

        // sorted by name: bare, code-review
        assert_eq!(found[0].name, "bare", "no frontmatter → directory name");
        assert_eq!(
            found[0].description, "Push the thing to staging.",
            "no description → first line that is not a heading"
        );
        assert_eq!(found[1].name, "code-review", "frontmatter name overrides the directory");
        assert_eq!(found[1].description, "Review a diff for security");
        assert_eq!(
            found[1].claude_path(),
            Path::new(".claude/skills/code-review/SKILL.md"),
            "projected under the frontmatter name, which is what the agent will call it"
        );
    }

    #[test]
    fn the_index_is_one_line_per_skill_and_names_the_file_to_read() {
        let t = tempfile::tempdir().unwrap();
        write(t.path(), "a", "---\ndescription: does A\n---\nbody\n");
        write(t.path(), "b", "---\ndescription: does B\n---\nbody\n");

        let idx = index(&discover(t.path()));
        let rows: Vec<&str> = idx.lines().filter(|l| l.contains(" | ")).collect();
        assert_eq!(rows.len(), 3, "header plus one row per skill: {idx}");
        assert!(idx.contains(".chorus/skills/a/SKILL.md"), "{idx}");
        assert!(!idx.contains("body"), "the index must not inline whole skills: {idx}");
    }

    #[test]
    fn no_skills_directory_is_not_an_error() {
        let t = tempfile::tempdir().unwrap();
        assert!(discover(t.path()).is_empty());
        assert_eq!(index(&[]), "", "an empty index renders no section at all");
    }
}
