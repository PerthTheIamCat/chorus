//! Side-effect journal (gap map R6b).
//!
//! A turn that dies halfway (quota, crash, Stop) may have already pushed a
//! branch or opened a PR. The next agent cannot tell from the transcript,
//! because the dead agent's events are lost with its process, so it repeats
//! the push or assumes it happened.
//!
//! So before an agent's tool call that reaches outside the machine runs, an
//! *intent* is written to the ledger, and once the call reports back a *done*
//! mark follows. A handoff lists every intent with no mark as "may be
//! half-done". No new event kind: both are `Decision` rows whose body starts
//! with a marker (rule 2.2 — the ledger stays the single source), and every
//! reader that treats `Decision` as "something settled" skips them through
//! [`is_marker`].

use crate::types::{Event, EventKind};
use serde_json::json;

/// Body prefix of an intent row.
pub const INTENT: &str = "chorus:intent ";
/// Body prefix of the row that closes an intent.
pub const INTENT_DONE: &str = "chorus:intent-done ";
/// Body prefix of a compare result (see `chorus-cli`'s `judge` module).
pub const COMPARE: &str = "chorus:compare ";
/// Body prefix of a cross-provider review (R6a).
pub const REVIEW: &str = "chorus:review ";

/// Whether a `Decision` body is bookkeeping written by Chorus rather than a
/// decision someone took. Readers that fold decisions into "done" skip these.
pub fn is_marker(body: &str) -> bool {
    [INTENT, INTENT_DONE, COMPARE, REVIEW]
        .iter()
        .any(|p| body.starts_with(p))
}

/// What an effectful tool call does to the world.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Effect {
    /// `push`, `pr`, `publish`, `deploy`, `http`, `api`.
    pub kind: &'static str,
    /// The call as the agent named it, trimmed for the brief.
    pub what: String,
}

const MAX_WHAT: usize = 160;

fn trimmed(title: &str) -> String {
    let one_line = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= MAX_WHAT {
        return one_line;
    }
    let cut: String = one_line.chars().take(MAX_WHAT).collect();
    format!("{cut}…")
}

/// Whether the tool call named `title` reaches outside the machine in a way
/// that cannot be taken back by restoring files: pushes, PRs, publishes,
/// deploys, mutating HTTP calls. `None` for everything else, which is nearly
/// every call, so the journal stays short.
///
/// The classifier reads the title because that is all the tool observer gets
/// at call time; Claude titles a shell call with the command itself.
pub fn classify(title: &str) -> Option<Effect> {
    let lower = title.to_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();
    let has = |w: &str| words.contains(&w);
    let seq = |a: &str, b: &str| words.windows(2).any(|p| p[0] == a && p[1] == b);
    let any_flag = |flags: &[&str]| {
        words.iter().any(|w| {
            flags
                .iter()
                .any(|f| w == f || (f.starts_with("--") && w.starts_with(&format!("{f}="))))
        })
    };
    let dry = any_flag(&["--dry-run", "-n"]) && !has("curl");

    // `git [-C dir] [-c k=v] push`: skip git's own options to find the subcommand.
    let git_push = words.iter().position(|w| *w == "git").is_some_and(|i| {
        let mut rest = words[i + 1..].iter();
        while let Some(w) = rest.next() {
            match *w {
                "-c" | "--git-dir" | "--work-tree" => {
                    rest.next();
                }
                w if w.starts_with('-') => {}
                w => return w == "push",
            }
        }
        false
    });
    let kind = if git_push {
        (!dry).then_some("push")
    } else if has("gh")
        && (seq("pr", "create")
            || seq("pr", "merge")
            || seq("pr", "close")
            || seq("pr", "comment")
            || seq("pr", "review")
            || seq("issue", "create")
            || seq("issue", "comment")
            || seq("release", "create"))
    {
        Some("pr")
    } else if has("gh") && has("api") {
        let mutating = words.windows(2).any(|p| {
            matches!(p[0], "-x" | "--method")
                && matches!(p[1], "post" | "put" | "patch" | "delete")
        }) || any_flag(&["-f", "-F", "--field", "--raw-field", "--input"]);
        mutating.then_some("api")
    } else if has("curl") || has("wget") {
        let mutating = words.windows(2).any(|p| {
            matches!(p[0], "-x" | "--request") && matches!(p[1], "post" | "put" | "patch" | "delete")
        }) || any_flag(&[
            "-d",
            "--data",
            "--data-raw",
            "--data-binary",
            "--data-urlencode",
            "-F",
            "--form",
            "-T",
            "--upload-file",
            "--post-data",
            "--post-file",
        ]);
        mutating.then_some("http")
    } else if ((has("npm") || has("pnpm") || has("yarn") || has("cargo") || has("bun"))
        && has("publish")
        && !dry)
        || seq("docker", "push")
        || seq("twine", "upload")
        || seq("gem", "push")
    {
        Some("publish")
    } else if seq("kubectl", "apply")
        || seq("kubectl", "delete")
        || seq("terraform", "apply")
        || seq("terraform", "destroy")
        || seq("helm", "upgrade")
        || seq("helm", "install")
        || ((has("vercel") || has("netlify") || has("fly") || has("flyctl")) && has("deploy"))
    {
        Some("deploy")
    } else if [
        "create_pull_request",
        "merge_pull_request",
        "push_files",
        "create_or_update_file",
        "delete_file",
        "add_issue_comment",
        "create_branch",
        "create_release",
    ]
    .iter()
    .any(|t| lower.contains(t))
    {
        Some("api")
    } else {
        None
    }?;

    Some(Effect {
        kind,
        what: trimmed(title),
    })
}

/// Body of the row written *before* the call runs.
pub fn intent_body(effect: &Effect, agent: &str) -> String {
    format!(
        "{INTENT}{}",
        json!({ "kind": effect.kind, "what": effect.what, "agent": agent })
    )
}

/// Body of the row that closes `intent_seq`. `ok = false` means the call
/// reported failure, so nothing is half-done: it did not happen.
pub fn done_body(intent_seq: i64, ok: bool) -> String {
    format!("{INTENT_DONE}{}", json!({ "intent": intent_seq, "ok": ok }))
}

/// Whether a tool-result status means the call went through.
pub fn status_ok(status: &str) -> bool {
    matches!(
        status.trim().to_lowercase().as_str(),
        "completed" | "success" | "ok" | "done"
    )
}

/// An intent nothing closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenIntent {
    pub seq: i64,
    pub kind: String,
    pub what: String,
    pub agent: String,
}

/// Intents with no done mark, oldest first.
pub fn open_intents(events: &[(Event, String)]) -> Vec<OpenIntent> {
    let mut open: Vec<OpenIntent> = Vec::new();
    for (e, body) in events {
        if e.kind != EventKind::Decision {
            continue;
        }
        if let Some(rest) = body.strip_prefix(INTENT_DONE) {
            let closed = serde_json::from_str::<serde_json::Value>(rest)
                .ok()
                .and_then(|v| v["intent"].as_i64());
            if let Some(id) = closed {
                open.retain(|o| o.seq != id);
            }
        } else if let Some(rest) = body.strip_prefix(INTENT) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(rest) {
                open.push(OpenIntent {
                    seq: e.seq,
                    kind: v["kind"].as_str().unwrap_or("").to_string(),
                    what: v["what"].as_str().unwrap_or("").to_string(),
                    agent: v["agent"].as_str().unwrap_or("").to_string(),
                });
            }
        }
    }
    open
}

/// One line per open intent, for the handoff brief's `pending`.
pub fn half_done_lines(events: &[(Event, String)]) -> Vec<String> {
    open_intents(events)
        .into_iter()
        .map(|o| {
            let who = if o.agent.is_empty() {
                String::new()
            } else {
                format!(" by {}", o.agent)
            };
            format!(
                "may be half-done: `{}` ({}{who}) started, no result recorded — check the remote before repeating it",
                o.what, o.kind
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(seq: i64, body: &str) -> (Event, String) {
        (
            Event {
                seq,
                t: 0,
                sess: 1,
                parent: None,
                kind: EventKind::Decision,
                actor: 1,
                body: Some(body.to_string()),
                blob: None,
                tok_in: None,
                tok_out: None,
                cost_usd: None,
                model_id: None,
            },
            body.to_string(),
        )
    }

    #[test]
    fn classifies_what_leaves_the_machine() {
        for t in [
            "git push origin main",
            "git -C repo push -u origin feat",
            "gh pr create --title x",
            "gh api -X POST repos/a/b/issues",
            "gh api repos/a/b/issues -f title=x",
            "curl -X POST https://x.test/hook",
            "curl -d '{}' https://x.test",
            "npm publish",
            "cargo publish --token t",
            "docker push img:1",
            "kubectl apply -f k.yaml",
            "mcp__github__create_pull_request",
        ] {
            assert!(classify(t).is_some(), "{t} should be journaled");
        }
        for t in [
            "git status",
            "git push --dry-run origin main",
            "gh pr view 12",
            "gh api repos/a/b/issues",
            "curl https://example.com",
            "cargo test",
            "npm publish --dry-run",
            "Read file src/lib.rs",
        ] {
            assert!(classify(t).is_none(), "{t} should not be journaled");
        }
    }

    #[test]
    fn an_intent_without_a_done_mark_is_half_done() {
        let effect = classify("git push origin main").unwrap();
        let intent = intent_body(&effect, "claude");
        let events = vec![ev(10, &intent)];
        let lines = half_done_lines(&events);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("git push origin main") && lines[0].contains("claude"));

        // Closed as done, or closed as failed: neither is half-done.
        for ok in [true, false] {
            let closed = vec![ev(10, &intent), ev(11, &done_body(10, ok))];
            assert!(half_done_lines(&closed).is_empty());
        }
    }

    #[test]
    fn markers_are_recognised_and_decisions_are_not() {
        let effect = classify("gh pr create").unwrap();
        assert!(is_marker(&intent_body(&effect, "codex")));
        assert!(is_marker(&done_body(1, true)));
        assert!(is_marker("chorus:compare {}"));
        assert!(is_marker("chorus:review {}"));
        assert!(!is_marker("use pnpm, not npm"));
    }

    #[test]
    fn long_commands_are_trimmed_for_the_brief() {
        let long = format!("git push origin {}", "x".repeat(400));
        let e = classify(&long).unwrap();
        assert!(e.what.chars().count() <= MAX_WHAT + 1);
    }
}
