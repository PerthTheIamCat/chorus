//! Manual smoke test of the Codex adapter against the real bridge.
//! Needs network + a Codex login (`codex login`).
//!
//!   cargo run -p chorus-acp --example codex_smoke -- /tmp/codex-smoke
//!
//! Denies every permission request, so it never writes outside the sandbox.

use chorus_acp::codex::{CodexAgent, CodexConfig, Permission, PermissionAsk};
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let cwd = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("codex-smoke"));
    std::fs::create_dir_all(&cwd)?;
    std::fs::write(cwd.join("README.md"), "# codex smoke\n\nHello from Chorus.\n")?;

    let log = chorus_acp::codex::file_log(&cwd.join("smoke.jsonl"))?;
    let mut agent = CodexAgent::start(&cwd, CodexConfig { log: Some(log), ..Default::default() })?;
    println!("session: {}", agent.session_id());

    let mut deny = |ask: &PermissionAsk| {
        println!("  permission asked: {} ({}) — DENY", ask.tool, ask.kind);
        Permission::Deny
    };
    let out = agent.prompt(
        "Read README.md and tell me its first heading. Do not modify anything.",
        &mut deny,
    )?;

    println!("stop_reason : {}", out.stop_reason);
    println!("context     : {:?} / {:?}", out.context_used, out.context_window);
    println!("message     : {}", out.message);
    println!("events      : {} ({:?})", out.events.len(), out.events);
    Ok(())
}
