//! `chorus bench` — the Phase 0 measurement harness.
//!
//! The Storage Spec makes three claims. This binary is how we hold the
//! implementation to them, on every run, from day one:
//!
//!   1. Re-reading the same file costs almost nothing on disk (dedup).
//!   2. The hot context is far smaller than the raw ledger it came from.
//!   3. Running out of budget demotes old turns down the rungs rather than
//!      truncating the tail, so the task state always survives.

use anyhow::{Context, Result};
use chorus_cli::session::{AgentSpec, Policy, Session};
use chorus_core::context::{AssembleInput, Assembler, Budget, Tier, TurnItem};
use chorus_core::handoff::HandoffReason;
use chorus_core::render::{self, ContextItem, FileChange};
use chorus_core::tokens::{CachedCounter, HeuristicCounter, TokenCounter};
use chorus_core::{actor, ledger, memory, EventKind, NewEvent, Store};
use std::path::{Path, PathBuf};

/// A believable tool result: an agent reading a 40 KB source file.
fn fake_file(path: &str, lines: usize) -> String {
    let mut s = String::with_capacity(lines * 60);
    s.push_str(&format!("// {path}\n"));
    for i in 0..lines {
        s.push_str(&format!(
            "  const row{i} = await db.query('select * from orders where id = $1', [id]); // {i}\n"
        ));
    }
    s
}

fn tier_name(t: Tier) -> &'static str {
    match t {
        Tier::System => "system",
        Tier::TaskState => "task state",
        Tier::Memory => "memory",
        Tier::Conversation => "conversation",
        Tier::Code => "code",
        Tier::Reserve => "reserve",
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("bench");
    match cmd {
        "bench" => bench(),
        "run" => run(&args[1..]),
        "handoff" => handoff(&args[1..]),
        "memory" => memory_cmd(&args[1..]),
        "compare" => compare_cmd(&args[1..]),
        "compact" => compact_cmd(&args[1..]),
        "context" => context_cmd(&args[1..]),
        "skills" => skills_cmd(&args[1..]),
        "mcp" => mcp_cmd(&args[1..]),
        "models" => models_cmd(&args[1..]),
        "usage" => {
            println!("{}", serde_json::to_string_pretty(&chorus_cli::limits::usage_report())?);
            Ok(())
        }
        "doctor" => doctor_cmd(),
        "demo" => demo_cmd(&args[1..]),
        "agent" => agent_cmd(&args[1..]),
        "sessions" => sessions_cmd(&args[1..]),
        "secrets" => secrets_cmd(&args[1..]),
        other => {
            eprintln!(
                "unknown command {other:?}; try: chorus agent | demo | doctor | bench | run | handoff | memory | compare | compact | context | skills | mcp | models | sessions | secrets"
            );
            std::process::exit(2);
        }
    }
}

/// `chorus secrets [list|forget NAME|scan FILE]`
///
/// What the vault remembers — names and kinds, never values. `scan` shows
/// what masking would do to a file without storing anything.
fn secrets_cmd(args: &[String]) -> Result<()> {
    use chorus_core::secrets;
    let vault = secrets::user_vault();
    match args.first().map(String::as_str).unwrap_or("list") {
        "list" => {
            let all = vault.list();
            if all.is_empty() {
                println!("ยังไม่มี secret ที่จำไว้");
            }
            for e in all {
                println!("  ${}  ({})", e.name, e.kind);
            }
        }
        "forget" => {
            let name = args.get(1).context("usage: chorus secrets forget NAME")?;
            let name = name.trim_start_matches('$');
            vault.forget(name).map_err(anyhow::Error::msg)?;
            println!("ลืม ${name} แล้ว");
        }
        "scan" => {
            let path = args.get(1).context("usage: chorus secrets scan FILE")?;
            let text = std::fs::read_to_string(path).with_context(|| format!("read {path}"))?;
            // Kinds and positions only: printing what was found would print
            // the secret.
            for f in secrets::find(&text) {
                let line = text[..f.start].matches('\n').count() + 1;
                println!("  {path}:{line}  {}", f.kind);
            }
        }
        other => anyhow::bail!("unknown: chorus secrets {other} (list | forget NAME | scan FILE)"),
    }
    Ok(())
}

/// `chorus memory <pending|approve ID|reject ID|add TEXT> --db PATH [--scope N]`
///
/// The human side of the approve queue (design §4). Agents propose via the
/// `memory.propose` tool; a person clears the queue here.
fn memory_cmd(args: &[String]) -> Result<()> {
    use chorus_core::memory;

    let mut db: Option<PathBuf> = None;
    let mut scope: i64 = 1;
    let mut rest: Vec<&String> = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--db" => db = it.next().map(PathBuf::from),
            "--scope" => scope = it.next().and_then(|v| v.parse().ok()).unwrap_or(1),
            _ => rest.push(a),
        }
    }
    let db = db.context("--db is required")?;
    let store = Store::open(&db).context("open store")?;
    let c = store.conn();
    let sub = rest.first().map(|s| s.as_str()).unwrap_or("pending");

    match sub {
        "pending" => {
            let q = memory::pending(c)?;
            if q.is_empty() {
                println!("queue empty");
            }
            for p in q {
                println!("  [{}] scope {} · {}", p.id, p.scope, p.text);
            }
        }
        "approve" => {
            let id: i64 = rest
                .get(1)
                .and_then(|v| v.parse().ok())
                .context("approve <id>")?;
            let fact = memory::approve(c, id)?;
            println!("approved → fact {fact}");
        }
        "reject" => {
            let id: i64 = rest
                .get(1)
                .and_then(|v| v.parse().ok())
                .context("reject <id>")?;
            memory::reject(c, id)?;
            println!("rejected {id}");
        }
        "add" => {
            let text = rest.get(1).context("add <text>")?;
            let id = memory::add(c, scope, text)?;
            println!("added → fact {id} (scope {scope})");
        }
        other => anyhow::bail!("memory: unknown subcommand {other:?}"),
    }
    Ok(())
}

/// `chorus compare <prompt> --db PATH --session N --agents codex,claude-opus [--yes]`
///
/// Forks the session per agent, runs the same prompt on each, prints what
/// each produced and the branch session id to resume.
fn compare_cmd(args: &[String]) -> Result<()> {
    let mut db: Option<PathBuf> = None;
    let mut session_id: i64 = 1;
    let mut agents: Vec<String> = Vec::new();
    let mut policy = Policy::DenyAll;
    let mut positional: Vec<&String> = Vec::new();

    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--db" => db = it.next().map(PathBuf::from),
            "--session" => session_id = it.next().and_then(|v| v.parse().ok()).unwrap_or(1),
            "--agents" => {
                if let Some(v) = it.next() {
                    agents = v
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
            }
            "--yes" => policy = Policy::AllowAll,
            "--read-only" => policy = Policy::ReadOnly,
            _ => positional.push(a),
        }
    }
    let db = db.context("--db is required")?;
    let [prompt] = positional.as_slice() else {
        anyhow::bail!("usage: chorus compare <prompt> --db PATH --session N --agents a,b");
    };
    anyhow::ensure!(agents.len() >= 2, "--agents needs at least two");

    let specs: Vec<AgentSpec> = agents
        .iter()
        .map(|n| AgentSpec::new(n, provider_of(n)))
        .collect();
    let mut s = Session::open(&db, session_id).context("open session")?
        .log_permissions(true);
    let branches = s.compare(&specs, prompt, policy)?;

    for b in branches {
        println!(
            "\n── {} · resume with --session {} ──────────────",
            b.agent, b.session_id
        );
        let cost = b.cost_usd.map(|c| format!(" · ${c:.4}")).unwrap_or_default();
        println!(
            "{:.1}s · {} in / {} out tokens{cost}",
            b.elapsed_ms as f64 / 1000.0,
            b.tokens_in,
            b.tokens_out
        );
        match b.result {
            Ok((msg, stop)) => println!("{}\n[{stop}]", msg.trim()),
            Err(e) => println!("failed: {e:#}"),
        }
        if !b.diff.trim().is_empty() {
            println!("\n{}", b.diff.trim_end());
        }
    }
    Ok(())
}

/// `chorus handoff --db PATH --session N --to NAME --reason R --goal TEXT
///  [--done TEXT]... [--pending TEXT]... [--diff-file PATH] [--yes]`
///
/// Hands the session to another agent: builds the brief from the ledger,
/// records a Handoff event, and runs the incoming agent's Verify turn.
fn handoff(args: &[String]) -> Result<()> {
    let mut db: Option<PathBuf> = None;
    let mut session_id: i64 = 1;
    let mut to = "codex".to_string();
    let mut reason = HandoffReason::Manual;
    let mut goal = String::new();
    let mut done: Vec<String> = Vec::new();
    let mut pending: Vec<String> = Vec::new();
    let mut diff_file: Option<PathBuf> = None;
    let mut policy = Policy::DenyAll;

    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut next = || it.next().cloned().unwrap_or_default();
        match a.as_str() {
            "--db" => db = Some(PathBuf::from(next())),
            "--session" => session_id = next().parse().unwrap_or(1),
            "--to" => to = next(),
            "--reason" => reason = parse_reason(&next()),
            "--goal" => goal = next(),
            "--done" => done.push(next()),
            "--pending" => pending.push(next()),
            "--diff-file" => diff_file = Some(PathBuf::from(next())),
            "--yes" => policy = Policy::AllowAll,
            "--read-only" => policy = Policy::ReadOnly,
            other => anyhow::bail!("unknown flag {other:?}"),
        }
    }
    let db = db.context("--db is required")?;
    anyhow::ensure!(!goal.is_empty(), "--goal is required");
    let diff = match diff_file {
        Some(p) => std::fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?,
        None => String::new(),
    };

    let mut s = Session::open(&db, session_id).context("open session")?
        .log_permissions(true);
    // Constraints come from project-scope memory facts.
    let constraints: Vec<String> = Vec::new();

    let r = s.handoff(
        &to,
        "openai",
        reason,
        &goal,
        done,
        pending,
        constraints,
        diff,
        policy,
    )?;

    println!("── brief handed to {} (seq {}) ──", r.to, r.handoff_seq);
    println!("{}", r.brief.render_markdown());
    println!("\n── {} echo-back ──────────────────────────────", r.to);
    println!("{}", r.echo_back.trim());
    println!(
        "\nstop: {} · {} events recorded",
        r.stop_reason,
        r.agent_seqs.len()
    );
    Ok(())
}

/// Best-guess provider for an agent name, for actor interning.
fn provider_of(name: &str) -> &'static str {
    let n = name.to_lowercase();
    if n.contains("claude") {
        "anthropic"
    } else if n.contains("antigravity") || n.contains("agy") || n.contains("gemini") {
        "google"
    } else if n.contains("codex") || n.contains("gpt") || n.contains("o3") {
        "openai"
    } else {
        "local"
    }
}

fn parse_reason(s: &str) -> HandoffReason {
    match s.replace(['-', ' '], "_").to_lowercase().as_str() {
        "rate_limit" => HandoffReason::RateLimit,
        "quota_exhausted" | "quota" => HandoffReason::QuotaExhausted,
        "context_exhausted" | "context" => HandoffReason::ContextExhausted,
        "user_requested" | "user" => HandoffReason::UserRequested,
        "failover" => HandoffReason::Failover,
        _ => HandoffReason::Manual,
    }
}

/// `chorus run <cwd> <prompt> [--db PATH] [--agent NAME] [--yes]`
///
/// One real Codex turn, driven into an app-owned ledger. Denies every
/// permission request unless `--yes` is passed.
fn run(args: &[String]) -> Result<()> {
    let mut positional: Vec<&String> = Vec::new();
    let mut db: Option<PathBuf> = None;
    let mut agent = "codex".to_string();
    let mut policy = Policy::DenyAll;
    let mut continue_session: Option<i64> = None;

    let mut chain: Vec<String> = Vec::new();
    let mut model: Option<String> = None;
    let mut computer_use = false;
    let mut claude_chrome = false;
    let mut show_secret_files = false;

    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--db" => db = it.next().map(PathBuf::from),
            "--agent" => {
                if let Some(v) = it.next() {
                    agent = v.clone();
                }
            }
            "--chain" => {
                if let Some(v) = it.next() {
                    chain = v
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
            }
            "--model" => model = it.next().cloned(),
            "--computer" => computer_use = true,
            "--claude-chrome" => claude_chrome = true,
            "--show-secret-files" => show_secret_files = true,
            "--session" => continue_session = it.next().and_then(|v| v.parse().ok()),
            "--yes" => policy = Policy::AllowAll,
            "--read-only" => policy = Policy::ReadOnly,
            _ => positional.push(a),
        }
    }

    let (mut s, prompt) = match continue_session {
        Some(id) => {
            let [prompt] = positional.as_slice() else {
                anyhow::bail!(
                    "usage: chorus run <prompt> --session N --db PATH [--agent NAME] [--yes]"
                );
            };
            let db = db.context("--db is required with --session")?;
            let s = Session::open(&db, id).context("open session")?
        .log_permissions(true);
            eprintln!("continuing session {id} · store {}", db.display());
            (s, (*prompt).clone())
        }
        None => {
            let [cwd, prompt] = positional.as_slice() else {
                anyhow::bail!(
                    "usage: chorus run <cwd> <prompt> [--db PATH] [--agent NAME] [--model ID] [--yes]\n   \
                     or: chorus run <prompt> --session N --db PATH"
                );
            };
            let db = db.unwrap_or_else(|| {
                std::env::temp_dir().join(format!("chorus-run-{}.db", std::process::id()))
            });
            let s = Session::create(&db, &PathBuf::from(cwd), prompt).context("create session")?
                .log_permissions(true);
            eprintln!("session {} · store {}", s.session_id(), db.display());
            (s, (*prompt).clone())
        }
    };

    if computer_use {
        // The permissions belong to whatever ran `chorus` (Terminal, iTerm…).
        s.set_computer_use(true);
    }
    s.set_claude_chrome(claude_chrome);
    s.set_hide_secret_files(!show_secret_files);
    if let Some(m) = &model {
        // Pinned for whoever answers, including anyone the relay hands to:
        // "use the cheap model" is a statement about the task, not about one
        // provider, and a failover that silently upgrades is a surprise on the
        // bill.
        s.set_model(&agent, m);
        for name in &chain {
            s.set_model(name, m);
        }
    }

    let report = if chain.is_empty() {
        s.run_turn(&agent, "openai", &prompt, policy)?
    } else {
        let specs: Vec<AgentSpec> = chain
            .iter()
            .map(|name| AgentSpec::new(name, provider_of(name)))
            .collect();
        let relay = s.run_relay(&specs, &prompt, policy)?;
        for (from, to, why) in &relay.handoffs {
            eprintln!("  relay: {from} → {to} ({})", why.as_phrase());
        }
        agent = relay.answered_by;
        relay.turn
    };

    println!("\n── {agent} ──────────────────────────────────────────");
    println!("{}", report.message.trim());
    println!("\nstop: {}", report.stop_reason);
    if let (Some(u), Some(w)) = (report.context_used, report.context_window) {
        println!("context: {u} / {w} ({:.0}%)", u as f64 / w as f64 * 100.0);
    }
    println!(
        "ledger: user seq {} + {} agent events",
        report.user_seq,
        report.agent_seqs.len()
    );
    Ok(())
}

fn bench() -> Result<()> {
    let dir: PathBuf = std::env::temp_dir().join(format!("chorus-bench-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let db_path = dir.join("store.db");
    let _ = std::fs::remove_file(&db_path);

    let store = Store::open(&db_path).context("open store")?;
    let c = store.conn();

    let claude = actor::intern(c, "claude-opus", "anthropic")?;
    let codex = actor::intern(c, "codex", "openai")?;
    let user = actor::intern(c, "user", "human")?;

    let sess = ledger::new_session(c, "แก้ N+1 query ในหน้า order list", "/Users/perth/work/api")?;

    // ---- 1. dedup -------------------------------------------------------
    // The same file, read back eight times over the course of the session.
    // Each re-read must cost a reference, not another copy.
    let file = fake_file("src/order/repo.ts", 600);
    let reads = 8;
    ledger::append(
        c,
        &NewEvent::new(
            sess,
            EventKind::Msg,
            user,
            "หน้า order list ช้ามาก ช่วยดูให้หน่อย",
        ),
    )?;
    for i in 0..reads {
        ledger::append(
            c,
            &NewEvent::new(sess, EventKind::ToolCall, claude, "read src/order/repo.ts"),
        )?;
        ledger::append(
            c,
            &NewEvent::new(sess, EventKind::ToolResult, claude, file.clone()),
        )
        .with_context(|| format!("append read #{i}"))?;
    }

    // Some ordinary turns on top, so the ledger isn't only tool traffic.
    for i in 0..60 {
        ledger::append(
            c,
            &NewEvent::new(
                sess,
                EventKind::Msg,
                if i % 2 == 0 { claude } else { user },
                format!("เทิร์นที่ {i}: อธิบายสิ่งที่เจอและถามต่อเรื่อง eager loading กับ index"),
            ),
        )?;
    }

    // A handoff, mid-session: Claude runs out of quota and Codex takes over.
    ledger::append(
        c,
        &NewEvent::new(
            sess,
            EventKind::Handoff,
            codex,
            "reason=rate_limit from=claude-opus to=codex; brief carried goal/done/pending",
        ),
    )?;

    let blob_rows: i64 = c.query_row("select count(*) from blob", [], |r| r.get(0))?;
    let event_rows: i64 = c.query_row("select count(*) from event", [], |r| r.get(0))?;
    let blob_bytes: i64 =
        c.query_row("select coalesce(sum(length(bytes)),0) from blob", [], |r| {
            r.get(0)
        })?;
    let orig_bytes: i64 = c.query_row("select coalesce(sum(orig_len),0) from blob", [], |r| {
        r.get(0)
    })?;
    let naive = file.len() as i64 * reads as i64;

    println!("── dedup ─────────────────────────────────────────────");
    println!("  events                {event_rows}");
    println!("  reads of one file     {reads}");
    println!("  blob rows             {blob_rows}   (naive would be {reads})");
    println!(
        "  stored                {:.1} KB compressed / {:.1} KB raw",
        blob_bytes as f64 / 1024.0,
        orig_bytes as f64 / 1024.0
    );
    println!(
        "  naive would store     {:.1} KB   → saved {:.1}%",
        naive as f64 / 1024.0,
        100.0 - (blob_bytes as f64 / naive as f64) * 100.0
    );

    // ---- 2. memory: stored with metadata, sent without ------------------
    for f in [
        "ใช้ pnpm ไม่ใช่ npm; รัน test ด้วย `pnpm test:order`",
        "Postgres 16, migration อยู่ใน migrations/ เรียงเลขสี่หลัก",
        "ห้ามแก้ schema โดยไม่ถามก่อน",
    ] {
        memory::add(c, 1, f)?;
    }
    let facts = memory::top(c, 1, 8)?;
    let facts_rendered = memory::render(&facts);

    // ---- 3. context assembly -------------------------------------------
    let counter = CachedCounter::new(HeuristicCounter);
    let events = ledger::tail(c, sess, 200)?;
    let raw_total: i64 = events
        .iter()
        .map(|e| counter.count(&ledger::body(c, e).unwrap_or_default()))
        .sum();

    let turns: Vec<TurnItem> = events
        .iter()
        .rev()
        .enumerate()
        .map(|(age, e)| {
            let body = ledger::body(c, e).unwrap_or_default();
            TurnItem {
                text_l1: Some(format!(
                    "[สรุป] {}",
                    body.chars().take(80).collect::<String>()
                )),
                text_l2: Some("[สรุปช่วงงาน] สืบสาเหตุ N+1 และเพิ่ม index".into()),
                text_l0: body,
                age,
                key: e.blob.as_ref().map(|b| b.0.clone()),
            }
        })
        .collect();

    let code = vec![
        ContextItem {
            text: file.clone(),
            key: Some("b3:repo".into()),
            label: "src/order/repo.ts".into(),
        },
        ContextItem {
            text: file.clone(),
            key: Some("b3:repo".into()),
            label: "src/order/repo.ts".into(),
        },
        ContextItem {
            text: render::file_manifest(&[
                FileChange {
                    path: "src/order/repo.ts".into(),
                    status: "modified".into(),
                    added: 38,
                    removed: 11,
                },
                FileChange {
                    path: "migrations/0042_idx.sql".into(),
                    status: "added".into(),
                    added: 6,
                    removed: 0,
                },
            ]),
            key: None,
            label: "diff".into(),
        },
    ];

    let input = AssembleInput {
        system: "You are a coding agent working in a Postgres/TypeScript repo.".into(),
        task_state: "goal: ทำหน้า order list ให้ต่ำกว่า 200ms\ndone: เจอ N+1 ใน findAll, เพิ่ม index แล้ว\npending: eager-load, รัน pnpm test:order".into(),
        memory: facts_rendered
            .lines()
            .map(|l| ContextItem { text: l.to_string(), key: None, label: "fact".into() })
            .collect(),
        turns,
        code,
    };

    for total in [200_000_i64, 24_000, 6_000] {
        let a = Assembler::new(Budget::new(total), CachedCounter::new(HeuristicCounter));
        let out = a.assemble(&input);
        let used: i64 = out.per_tier.iter().map(|(_, n)| *n).sum();
        println!("\n── context @ {total} token window ─────────────────────");
        println!("  ledger raw            {raw_total} token");
        println!(
            "  assembled             {used} token   ({:.1}% ของดิบ)",
            used as f64 / raw_total.max(1) as f64 * 100.0
        );
        println!(
            "  demoted / dropped     {} / {}",
            out.demoted_turns, out.dropped_turns
        );
        for (t, n) in &out.per_tier {
            let cap = Budget::new(total).cap(*t);
            println!("    {:<14} {:>7} / {:>7}", tier_name(*t), n, cap);
        }
        assert!(
            out.text.contains("goal:"),
            "task state must survive every budget — it outranks everything"
        );
    }

    // Fold the WAL back into the main file before measuring, or the size is a lie.
    c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    let db_size = std::fs::metadata(&db_path)?.len();
    println!("\n── on disk ───────────────────────────────────────────");
    println!(
        "  store.db              {:.1} KB for {event_rows} events",
        db_size as f64 / 1024.0
    );
    println!(
        "  per event             {:.0} bytes",
        db_size as f64 / event_rows as f64
    );
    println!("\n  {}", db_path.display());
    // Most of this is fixed page overhead — every table and index takes at
    // least one 4 KB page whether or not it has rows — so it grows with the
    // schema more than with the events. 172 KB at schema v7; the ceiling
    // leaves room for a few more tables and catches a payload that stopped
    // going through the blob store (8 copies of the 49 KB file ≈ +390 KB).
    const DB_CEILING_BYTES: u64 = 256 * 1024;
    assert!(
        db_size <= DB_CEILING_BYTES,
        "store.db is {db_size} bytes for {event_rows} events (ceiling {DB_CEILING_BYTES}) — \
         check that large bodies still go to the deduplicated blob store"
    );

    bench_real_path(&dir)?;
    bench_handoff_in_context(&dir)?;
    Ok(())
}

/// What one handoff costs in every later turn's context.
///
/// The brief is stored as JSON (cold) because `task_state` reads pending and
/// constraints back from it. Everything the next agent needs from it already
/// reaches the model elsewhere — goal/pending/constraints through the task
/// state, files through the workspace — and its diff is a snapshot that goes
/// stale on the next edit. So in context (hot) a handoff is one line. Two
/// identical sessions, one with a handoff carrying a realistic diff, measured
/// through the real `assemble_context`: the difference is what it costs.
fn bench_handoff_in_context(dir: &Path) -> Result<()> {
    use chorus_core::handoff::{Brief, HandoffMeta};
    const HANDOFF_MAX_TOKENS: i64 = 30;

    let build = |name: &str, with_handoff: bool| -> Result<(String, String)> {
        let db_path = dir.join(format!("{name}.db"));
        let _ = std::fs::remove_file(&db_path);
        let cwd = dir.join(format!("{name}-ws"));
        std::fs::create_dir_all(&cwd)?;
        let s = Session::create(&db_path, &cwd, "bench handoff")?;
        let sess = s.session_id();
        {
            let store = Store::open(&db_path)?;
            let c = store.conn();
            let user = actor::intern(c, "user", "human")?;
            let claude = actor::intern(c, "claude", "anthropic")?;
            let codex = actor::intern(c, "codex", "openai")?;
            ledger::append(c, &NewEvent::new(sess, EventKind::Msg, user, "ทำหน้า order list ให้ต่ำกว่า 200ms"))?;
            for i in 0..5 {
                ledger::append(c, &NewEvent::new(sess, EventKind::Msg, claude, format!("เทิร์น {i}: ไล่ดู OrderRepo.findAll")))?;
            }
            if with_handoff {
                let diff: String = (0..300)
                    .map(|i| format!("-  const row{i} = await db.query('select * from orders where id = $1', [id]);\n+  const row{i} = rows[{i}];\n"))
                    .collect();
                let brief = Brief {
                    goal: "ทำหน้า order list ให้ต่ำกว่า 200ms".into(),
                    done: vec!["ใช้ eager loading ไม่ใช่ N+1".into()],
                    pending: vec!["รัน test ชุด order".into()],
                    constraints: vec!["ห้ามแตะ schema เพิ่ม".into()],
                    files: vec!["src/order/repo.ts".into()],
                    diff,
                    handoff: HandoffMeta { from: "claude".into(), to: "codex".into(), reason: HandoffReason::QuotaExhausted, at: 0 },
                };
                // Stored exactly as `Session::handoff` stores it.
                ledger::append(c, &NewEvent::new(sess, EventKind::Handoff, codex, brief.to_json()))?;
            }
            for i in 0..3 {
                ledger::append(c, &NewEvent::new(sess, EventKind::Msg, codex, format!("ต่อจาก claude {i}")))?;
            }
        }
        let s = Session::open(&db_path, sess)?;
        Ok((s.assemble_context(200_000)?, s.task_state()?.render()))
    };

    let counter = HeuristicCounter;
    let (without, ts_without) = build("handoff-none", false)?;
    let (with, ts_with) = build("handoff-one", true)?;
    // The brief's pending/constraints *should* arrive — through the task
    // state. Count that separately so the ceiling measures only the line.
    let carried = counter.count(&ts_with).saturating_sub(counter.count(&ts_without));
    let cost = counter.count(&with).saturating_sub(counter.count(&without)).saturating_sub(carried);
    println!("\n── handoff in later turns' context ───────────────────");
    println!("  one handoff costs     {cost} token per turn (ceiling {HANDOFF_MAX_TOKENS})");
    println!("  pending/constraints   {carried} token, via task state");
    assert!(
        !with.contains("```diff") && !with.contains("\"constraints\"") && !with.contains("\"diff\""),
        "the stored brief JSON (and its stale diff) leaked into the hot context"
    );
    assert!(
        with.contains("รัน test ชุด order") && with.contains("ห้ามแตะ schema เพิ่ม"),
        "pending/constraints from the brief must still reach the model, via the task state"
    );
    assert!(cost <= HANDOFF_MAX_TOKENS, "a handoff costs {cost} tokens in every later turn");
    Ok(())
}

/// The part of the bench that runs the **real** `Session::assemble_context`,
/// not a hand-fed `AssembleInput`.
///
/// This exists because of a regression that the synthetic bench above could
/// never have caught: `assemble_context` passed `task_state: String::new()`
/// for months while this file fed the tier a fixture and reported it working.
/// A measurement harness that supplies its own inputs measures the harness.
/// Everything below asserts against the code path the product actually runs.
fn bench_real_path(dir: &Path) -> Result<()> {
    let db_path = dir.join("real.db");
    let _ = std::fs::remove_file(&db_path);
    let cwd = dir.join("workspace");
    std::fs::create_dir_all(&cwd)?;

    let mut s = Session::create(&db_path, &cwd, "bench real path")?;
    let sess = s.session_id();

    // A session that has said something: the human's goal, then agent turns.
    {
        let store = Store::open(&db_path)?;
        let c = store.conn();
        let user = actor::intern(c, "user", "human")?;
        let agent = actor::intern(c, "codex", "openai")?;
        ledger::append(
            c,
            &NewEvent::new(
                sess,
                EventKind::Msg,
                user,
                "ทำหน้า order list ให้ต่ำกว่า 200ms",
            ),
        )?;
        for i in 0..30 {
            ledger::append(
                c,
                &NewEvent::new(
                    sess,
                    EventKind::Msg,
                    agent,
                    format!("เทิร์นที่ {i}: ไล่ดู OrderRepo.findAll แล้วเจอว่าโหลด relation ทีละแถว"),
                ),
            )?;
        }
        ledger::append(
            c,
            &NewEvent::new(sess, EventKind::Decision, agent, "ใช้ eager loading ไม่ใช่ N+1"),
        )?;
    }
    s = Session::open(&db_path, sess)?;

    println!("\n── real path (Session::assemble_context) ─────────────");

    // 1. The never-trimmed tier must not be empty.
    let ts = s.task_state()?;
    let assembled = s.assemble_context(6_000)?;
    println!("  task state            {} chars", ts.render().len());
    println!("  goal                  {}", ts.goal);
    println!("  done items            {}", ts.done.len());
    assert!(
        !ts.is_empty(),
        "task state is empty on a session that has said something — \
         the tier the design says is never trimmed is carrying nothing"
    );
    assert!(
        assembled.contains(ts.goal.trim()),
        "the goal never reached the assembled context"
    );

    // 2. Reads come from the stored warm tier, not from a summariser.
    let pending_before = s.uncompacted_count()?;
    let r = s.compact(500)?;
    println!("  compact               {} rows via {}", r.compacted, r.model);
    assert!(pending_before > 0 && r.compacted as i64 == pending_before);
    assert_eq!(s.uncompacted_count()?, 0, "compact left rows behind");

    // Overwrite one stored rung with a sentinel: if it shows up in the
    // assembled text, the read path is genuinely reading the table.
    {
        let store = Store::open(&db_path)?;
        let c = store.conn();
        let seq: i64 = c.query_row(
            "SELECT max(seq) FROM event WHERE sess = ?1",
            [sess],
            |r| r.get(0),
        )?;
        chorus_core::compaction::save_rungs(
            c,
            seq,
            &chorus_core::compaction::Rungs {
                l1: "SENTINEL-L1".into(),
                l2: "SENTINEL-L2".into(),
            },
            "bench",
        )?;
    }
    let s = Session::open(&db_path, sess)?;
    let tight = s.assemble_context(900)?;
    assert!(
        tight.contains("SENTINEL"),
        "a tight budget did not fall back to the stored rung — \
         assemble_context is not reading the warm tier"
    );
    println!("  stored rung reused    yes");

    // 3. The read path must not consult `compaction::best()`. If it ever does
    // again, this env var turns every event into a network round trip; the
    // assertion is on wall-clock so the regression is loud rather than subtle.
    std::env::set_var("CHORUS_SUMMARY_MODEL", "chorus-bench-nonexistent-model");
    let t0 = std::time::Instant::now();
    let _ = s.assemble_context(6_000)?;
    let took = t0.elapsed();
    std::env::remove_var("CHORUS_SUMMARY_MODEL");
    println!("  assemble w/ model set {:.0} ms", took.as_secs_f64() * 1000.0);
    assert!(
        took < std::time::Duration::from_secs(2),
        "assemble_context took {took:?} with CHORUS_SUMMARY_MODEL set — \
         it is summarising on the read path again"
    );

    println!("\n  invariants ผ่านครบ");
    Ok(())
}

/// `chorus models [agent...]`
///
/// Ask each agent what it can actually run, and cache the answer.
///
/// This spawns a real bridge per agent, so it is slow and deliberately manual
/// — the picker reads the cache. Worth the wait once: before this existed the
/// app offered `gpt-5.1-codex`, a model Codex does not have, because the list
/// was a guess written by hand.
fn models_cmd(args: &[String]) -> Result<()> {
    let wanted: Vec<String> = if args.is_empty() {
        vec!["claude".into(), "codex".into()]
    } else {
        args.to_vec()
    };
    let cwd = std::env::current_dir()?;

    for id in &wanted {
        rule(id);
        match chorus_cli::agents::refresh_models(id, &cwd) {
            Ok(models) => {
                for m in &models {
                    let mark = if m.is_default { "*" } else { " " };
                    println!("  {mark} {:<24} {}", m.id, m.label);
                }
                println!("\n  {} โมเดล · เก็บไว้แล้ว ครั้งหน้าไม่ต้องถามใหม่", models.len());
            }
            Err(e) => println!("  ถามไม่ได้: {e:#}"),
        }
        println!();
    }
    Ok(())
}

/// `chorus mcp [list|import] [DIR]`
///
/// The tool list every agent in this workspace gets, and where it came from.
///
/// Prints names, transports and command lines — and the *names* of the env
/// vars and headers each server carries, never their values. A config is
/// debugged by knowing that `GITHUB_TOKEN` is set; printing what it is set to
/// would put a live credential in a terminal, a scrollback, and every bug
/// report that follows.
fn mcp_cmd(args: &[String]) -> Result<()> {
    use chorus_core::mcp_config;

    let sub = args.first().map(String::as_str).unwrap_or("list");
    let cwd = args
        .get(if sub == "list" || sub == "import" { 1 } else { 0 })
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir()?);

    if sub == "import" {
        let added = mcp_config::import_claude_project(&cwd)?;
        rule("import");
        if added.is_empty() {
            println!(
                "  ไม่มีอะไรให้ import — {}/{} ไม่มีเซิร์ฟเวอร์ใหม่",
                cwd.display(),
                mcp_config::CLAUDE_PROJECT_CONFIG
            );
        } else {
            println!("  เพิ่ม {} เซิร์ฟเวอร์เข้า {}:", added.len(), mcp_config::CONFIG);
            for n in &added {
                println!("    + {n}");
            }
            println!("\n  (ของเดิมในไฟล์ไม่ถูกทับ — รันซ้ำได้ปลอดภัย)");
        }
        return Ok(());
    }

    let servers = mcp_config::load(&cwd);
    rule("MCP — ทุก agent ได้ชุดเดียวกันนี้");
    if servers.is_empty() {
        println!("  ยังไม่มีเซิร์ฟเวอร์ใน {}\n", mcp_config::config_path(&cwd).display());
        println!("  ถ้ามี .mcp.json อยู่แล้ว:  chorus mcp import {}", cwd.display());
        return Ok(());
    }
    for r in mcp_config::redact(&servers) {
        println!("  ● {}  [{}]", r.name, r.transport);
        println!("      {}", r.target);
        if !r.secret_keys.is_empty() {
            println!("      ตัวแปรลับ: {} (ไม่แสดงค่า)", r.secret_keys.join(", "));
        }
    }
    println!(
        "\n  {} เซิร์ฟเวอร์ · ส่งเข้า session/new ของทั้ง Claude และ Codex",
        servers.len()
    );
    println!("  + chorus เองอีกหนึ่งตัว (memory_propose) ตอนเปิด session จริง");
    Ok(())
}

/// `chorus skills [DIR]`
///
/// What every agent in this workspace will see, and where each one will see
/// it. The registry is only worth having if you can check it without starting
/// a turn — "the skill was there but Codex never mentioned it" is otherwise
/// impossible to tell apart from "the skill was never found".
fn skills_cmd(args: &[String]) -> Result<()> {
    let cwd = args
        .first()
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir()?);
    let found = chorus_core::skills::discover(&cwd);

    rule("skills");
    if found.is_empty() {
        println!(
            "  ยังไม่มี skill ใน {}/{}\n",
            cwd.display(),
            chorus_core::skills::SKILLS_DIR
        );
        println!("  สร้างอันแรก:");
        println!("    mkdir -p {}/review", chorus_core::skills::SKILLS_DIR);
        println!("    $EDITOR {}/review/SKILL.md", chorus_core::skills::SKILLS_DIR);
        return Ok(());
    }
    for k in &found {
        println!("  ● {}", k.name);
        println!("      {}", k.description);
        println!("      ต้นฉบับ  {}", k.source_path().display());
        println!("      Claude   {}", k.claude_path().display());
        println!("      Codex    AGENTS.md § Skills (หนึ่งบรรทัด)");
    }

    rule("หน้าตาที่ agent ที่ไม่มี skill loader จะเห็น");
    for line in chorus_core::skills::index(&found).lines() {
        println!("  │ {line}");
    }
    println!(
        "\n  ({} skill · เขียนที่เดียว ใช้ได้ทุกเจ้า)",
        found.len()
    );
    Ok(())
}

/// `chorus context --db PATH --session N [--budget N] [--raw]`
///
/// Print exactly what the next agent would be handed — the whole product
/// claim in one command. Everything else in this CLI reports what *happened*;
/// this shows what the receiving agent *sees*, which is the only way to tell a
/// working handoff from one that merely did not crash.
///
/// Read-only and summariser-free: it calls the same `task_state()` and
/// `assemble_context()` the relay calls, so what it prints is not a
/// reconstruction — it is the artefact itself.
fn context_cmd(args: &[String]) -> Result<()> {
    let mut db: Option<PathBuf> = None;
    let mut session_id: i64 = 1;
    let mut budget: i64 = 8_000;
    let mut raw = false;

    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--db" => db = it.next().map(PathBuf::from),
            "--session" => session_id = it.next().and_then(|v| v.parse().ok()).unwrap_or(1),
            "--budget" => budget = it.next().and_then(|v| v.parse().ok()).unwrap_or(8_000),
            "--raw" => raw = true,
            other => anyhow::bail!("unknown flag {other:?}"),
        }
    }
    let db = db.context("--db is required")?;
    let s = Session::open(&db, session_id).context("open session")?;

    let ctx = s.assemble_context(budget)?;
    if raw {
        print!("{ctx}");
        return Ok(());
    }

    rule("task state — ชั้นที่ไม่ถูกตัดทิ้ง");
    let ts = s.task_state()?;
    if ts.is_empty() {
        println!("  (ว่าง — session นี้ยังไม่มีอะไรเลย)");
    } else {
        for line in ts.render().lines() {
            println!("  {line}");
        }
    }

    rule("context ที่จะส่งจริง");
    println!(
        "  {} ตัวอักษร · ประมาณ {} token · เพดาน {budget}\n",
        ctx.chars().count(),
        {
            use chorus_core::tokens::TokenCounter;
            chorus_core::tokens::HeuristicCounter.count(&ctx)
        },
    );
    for line in ctx.lines() {
        println!("  │ {line}");
    }
    println!("\n  (--raw เพื่อเอาเฉพาะตัว context ดิบ ไม่มีหัวข้อ)");
    Ok(())
}

/// `chorus compact --db PATH --session N [--limit N]`
///
/// Fill in the warm tier — the stored L1/L2 rungs (storage spec). This is the
/// only place a summariser is allowed to be slow, which is precisely why it
/// lives here and not in `run`: `assemble_context` reads these rows and must
/// never generate them, or every turn would pay a model call per event.
///
/// Set `CHORUS_SUMMARY_MODEL` to use Ollama; otherwise the model-free
/// extractive summariser writes the rows.
fn compact_cmd(args: &[String]) -> Result<()> {
    let mut db: Option<PathBuf> = None;
    let mut session: Option<i64> = None;
    let mut limit: usize = 500;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--db" => db = it.next().map(PathBuf::from),
            "--session" => session = it.next().and_then(|v| v.parse().ok()),
            "--limit" => limit = it.next().and_then(|v| v.parse().ok()).unwrap_or(500),
            other => anyhow::bail!("unknown flag {other:?}"),
        }
    }
    let db = db.context("--db is required")?;
    let session = session.context("--session is required")?;
    let s = Session::open(&db, session)?.log_permissions(true);

    let before = s.uncompacted_count()?;
    let t0 = std::time::Instant::now();
    let r = s.compact(limit)?;
    println!("summariser   {}", r.model);
    println!("compacted    {} of {before} pending", r.compacted);
    println!("remaining    {}", r.remaining);
    println!("took         {:.1}s", t0.elapsed().as_secs_f64());
    if r.remaining > 0 {
        println!("\nยังเหลืออยู่ — รัน compact ซ้ำได้เลย (ทำทีละ {limit})");
    }
    Ok(())
}

/// `chorus sessions [list|search QUERY|import PROVIDER ID|PATH --db PATH [--cwd DIR]]`
///
/// Conversations Claude Code, Codex and Gemini CLI left on this machine.
/// `import` copies one into the ledger as a Chorus session, so the next
/// `chorus run --session N` continues it with whichever agent you pick.
fn sessions_cmd(args: &[String]) -> Result<()> {
    use chorus_core::external_sessions as ext;
    let sub = args.first().map(String::as_str).unwrap_or("list");
    let mut db: Option<PathBuf> = None;
    let mut cwd: Option<PathBuf> = None;
    let mut limit: usize = 30;
    let mut positional: Vec<&str> = Vec::new();
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--db" => db = it.next().map(PathBuf::from),
            "--cwd" => cwd = it.next().map(PathBuf::from),
            "--limit" => limit = it.next().and_then(|v| v.parse().ok()).unwrap_or(30),
            other if other.starts_with("--") => anyhow::bail!("unknown flag {other:?}"),
            other => positional.push(other),
        }
    }
    let roots = chorus_cli::accounts::session_roots();
    let print = |found: &[ext::ExternalSession]| {
        if found.is_empty() {
            println!("ไม่เจอ session");
        }
        for s in found {
            let who = s.account.as_ref().map(|a| format!("{}@{a}", s.provider.agent())).unwrap_or_else(|| s.provider.agent().into());
            println!("{who:<14} {:<38} {}", s.id, s.title);
            if let Some(cwd) = &s.cwd {
                println!("{:<14} {cwd}", "");
            }
            if let Some(snip) = &s.snippet {
                println!("{:<14} “{snip}”", "");
            }
        }
    };
    match sub {
        "list" => print(&ext::discover(&roots, limit)),
        "search" => {
            let q = positional.join(" ");
            anyhow::ensure!(!q.trim().is_empty(), "usage: chorus sessions search QUERY");
            print(&ext::search(&roots, &q, limit));
        }
        "import" => {
            let [provider, id] = positional[..] else {
                anyhow::bail!("usage: chorus sessions import <claude|codex|gemini> <ID|PATH> --db PATH [--cwd DIR]");
            };
            let provider = ext::Provider::parse(provider).with_context(|| format!("unknown provider {provider:?}"))?;
            let db = db.context("--db is required")?;
            let s = ext::find(&roots, provider, id).with_context(|| format!("ไม่เจอ session {id} ของ {}", provider.label()))?;
            // The session's own directory when it still exists, so the next
            // turn runs (and checkpoints) where the conversation happened.
            let dir = cwd
                .or_else(|| s.cwd.as_ref().map(PathBuf::from).filter(|p| p.is_dir()))
                .or_else(|| std::env::current_dir().ok())
                .context("no working directory")?;
            let dir = dir.canonicalize().unwrap_or(dir);
            let store = Store::open(&db)?;
            let r = ext::import(store.conn(), &s, &dir.to_string_lossy())?;
            match (r.reused, r.events, r.not_appended) {
                (false, n, _) => println!("นำเข้าแล้ว → session {} ({n} events)", r.session_id),
                (true, 0, 0) => println!("นำเข้าไว้แล้ว → session {} (ไม่มีอะไรใหม่)", r.session_id),
                (true, 0, k) => println!(
                    "session {} คุยต่อใน Chorus ไปแล้ว — {k} ข้อความใหม่จาก {} ไม่ได้ต่อท้ายเพื่อไม่ให้ปนกัน",
                    r.session_id,
                    provider.label()
                ),
                (true, n, _) => println!("อัปเดต session {} (+{n} events)", r.session_id),
            }
            println!("\nคุยต่อ:  chorus run \"<prompt>\" --session {} --db {}", r.session_id, db.display());
        }
        other => anyhow::bail!("unknown sessions command {other:?}; try: list | search QUERY | import PROVIDER ID"),
    }
    Ok(())
}

// ---- demo -----------------------------------------------------------------

fn rule(title: &str) {
    println!("\n\u{2501}\u{2501} {title} \u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}");
}

/// The in-repo mock agent, so the demo needs no API key and no network.
///
/// Looked up in the order a person would expect it to be found: next to this
/// binary (the `cargo install` and `target/debug` cases), then on `PATH`, then
/// the workspace build directory for someone running from a checkout. The
/// error names the exact command that fixes it rather than saying "not found" —
/// `chorus` and `acp-probe` live in different crates, which is not obvious.
fn mock_bridge() -> Result<Vec<String>> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("acp-probe"));
        }
    }
    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            candidates.push(dir.join("acp-probe"));
        }
    }
    for rel in ["target/debug/acp-probe", "target/release/acp-probe"] {
        candidates.push(PathBuf::from(rel));
    }

    for c in &candidates {
        if c.is_file() {
            warn_if_stale(c);
            return Ok(vec![c.to_string_lossy().into_owned(), "--serve-mock".into()]);
        }
    }
    anyhow::bail!(
        "หา acp-probe ไม่เจอ — มันอยู่คนละ crate กับ chorus\n  \
         ในโฟลเดอร์โปรเจกต์:  cargo run -p chorus-cli -- demo\n  \
         หรือติดตั้งให้ครบ:    cargo install --path crates/chorus-acp"
    )
}

/// `cargo run -p chorus-cli` builds the chorus-acp *library*, not its
/// `acp-probe` **binary** — so a stale probe from before the last change keeps
/// being spawned, silently, and the demo shows yesterday's behaviour with no
/// hint that anything is wrong. (That is not hypothetical: it is how a fixed
/// mock kept reporting the old hardcoded path.)
///
/// Staleness is measured against chorus-acp's *sources*, not against this
/// binary: a single `cargo build --workspace` compiles chorus-acp before
/// chorus-cli, so "probe older than me" is true by milliseconds on every
/// healthy build — and it stays false when neither crate was rebuilt but the
/// mock was edited, which is exactly the case worth catching. The source path
/// is baked in at build time, so an installed binary simply skips the check.
fn warn_if_stale(probe: &Path) {
    fn mtime(p: &Path) -> Option<std::time::SystemTime> {
        std::fs::metadata(p).and_then(|m| m.modified()).ok()
    }
    fn newest_rs(dir: &Path) -> Option<std::time::SystemTime> {
        let mut newest = None;
        for entry in std::fs::read_dir(dir).ok()?.flatten() {
            let path = entry.path();
            let t = if path.is_dir() {
                newest_rs(&path)
            } else if path.extension().is_some_and(|e| e == "rs") {
                mtime(&path)
            } else {
                None
            };
            if let Some(t) = t {
                if newest.is_none_or(|n| t > n) {
                    newest = Some(t);
                }
            }
        }
        newest
    }

    let src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|crates| crates.join("chorus-acp").join("src"));
    let (Some(probe_t), Some(src_t)) = (mtime(probe), src.as_deref().and_then(newest_rs)) else {
        return;
    };
    if src_t > probe_t {
        eprintln!(
            "\u{26a0}  acp-probe ({}) เก่ากว่าโค้ดของ chorus-acp\n   \
             `cargo run -p chorus-cli` ไม่ได้ build binary ของ crate อื่นให้ \u{2014} \
             เดโมจะใช้ของเก่าเงียบ ๆ\n   \
             แก้: cargo build --workspace แล้วรันใหม่\n",
            probe.display()
        );
    }
}

/// `chorus demo [--keep]`
///
/// The whole product in thirty seconds, against mock agents: two agents in one
/// session, one of them hits a rate limit mid-task, and the other picks up
/// *knowing what happened* — which is the entire reason this app exists.
///
/// It runs on the in-repo mock bridge, so it needs no API key, no login and no
/// network. Nothing here is staged output: every number printed is read back
/// out of the ledger the run just wrote.
fn demo_cmd(args: &[String]) -> Result<()> {
    let keep = args.iter().any(|a| a == "--keep");
    let dir = std::env::temp_dir().join(format!("chorus-demo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let db = dir.join("store.db");
    std::fs::write(
        dir.join("repo.ts"),
        "export async function findAll(ids: string[]) {\n  // …\n}\n",
    )?;

    let bridge = mock_bridge()?;
    let goal = "ทำหน้า order list ให้ต่ำกว่า 200ms";

    println!("\u{2501}\u{2501} Chorus demo \u{2501}\u{2501} agent ปลอมล้วน ไม่ต้องมี API key \u{2501}\u{2501}");
    println!("workspace: {}", dir.display());

    let mut s = Session::create(&db, &dir, "demo")?.with_bridge(bridge.clone());
    let sess = s.session_id();

    // ---- 1. Claude works -------------------------------------------------
    rule("1. Claude เริ่มงาน");
    s.run_turn("claude-opus", "anthropic", goal, Policy::AllowAll)?;
    s.run_turn(
        "claude-opus",
        "anthropic",
        "เจออะไรบ้าง ทำต่อได้เลย",
        Policy::AllowAll,
    )?;
    let n_before = s.transcript()?.len();
    println!("  เทิร์นที่ 1–2 ตอบโดย claude-opus · ledger มี {n_before} event");

    // A decision worth carrying across the handoff.
    {
        let store = Store::open(&db)?;
        let c = store.conn();
        let a = actor::intern(c, "claude-opus", "anthropic")?;
        ledger::append(
            c,
            &NewEvent::new(
                sess,
                EventKind::Decision,
                a,
                "ใช้ eager loading ไม่ใช่ N+1 — ห้ามแก้ schema เพิ่ม",
            ),
        )?;
    }
    let s = Session::open(&db, sess)?.with_bridge(bridge.clone());

    // ---- 2. warm tier ----------------------------------------------------
    rule("2. ย่อความจำไว้ล่วงหน้า (warm tier)");
    let r = s.compact(500)?;
    println!("  สรุปเก็บไว้ {} รายการ ด้วย {}", r.compacted, r.model);
    println!("  (ทำตอนนี้ เพื่อที่ตอนประกอบ context จะไม่ต้องสร้างใหม่)");

    // ---- 3. the rate limit ----------------------------------------------
    rule("3. Claude ชนลิมิตกลางงาน");
    let mut s = Session::open(&db, sess)?.with_bridge(bridge.clone());
    let chain = vec![
        AgentSpec {
            name: "claude-opus".into(),
            provider: "anthropic".into(),
            bridge: {
                let mut b = bridge.clone();
                b.push("--fail".into());
                b.push("Error: 429 rate limit exceeded".into());
                b
            },
        },
        AgentSpec {
            name: "codex".into(),
            provider: "openai".into(),
            bridge: bridge.clone(),
        },
    ];
    let relay = s.run_relay(&chain, "เขียน eager-load ใน findAll แล้วรัน test", Policy::AllowAll)?;
    for (from, to, why) in &relay.handoffs {
        println!("  {from} ตอบไม่ได้ ({})", why.as_phrase());
        println!("  Chorus ส่งไม้ต่อให้ {to} เองโดยไม่ถาม");
    }
    println!("  เทิร์นนี้ตอบโดย: {}", relay.answered_by);

    // ---- 4. what the second agent actually received ----------------------
    rule("4. สิ่งที่ codex ได้รับตอนรับช่วง  \u{2190} หัวใจของแอปนี้");
    let brief_json = {
        let store = Store::open(&db)?;
        let c = store.conn();
        let evs = ledger::history(c, sess, 400)?;
        let ho = evs
            .iter()
            .rev()
            .find(|e| e.kind == EventKind::Handoff)
            .context("no handoff recorded")?;
        ledger::body(c, ho)?
    };
    let brief: serde_json::Value = serde_json::from_str(&brief_json).unwrap_or_default();
    let list = |k: &str| -> Vec<String> {
        brief[k]
            .as_array()
            .map(|a| a.iter().filter_map(|x| x.as_str()).map(str::to_string).collect())
            .unwrap_or_default()
    };
    println!("  goal      {}", brief["goal"].as_str().unwrap_or("—"));
    for d in list("done") {
        println!("  done      {d}");
    }
    for p in list("pending") {
        println!("  pending   {p}");
    }
    for f in list("files") {
        println!("  files     {f}");
    }
    let diff_len = brief["diff"].as_str().unwrap_or("").len();
    println!("  diff      {diff_len} bytes จาก checkpoint");

    // ---- 5. the comparison ----------------------------------------------
    rule("5. เทียบกับการทำมือ");
    let ctx = s.assemble_context(120_000)?;
    let counter = CachedCounter::new(HeuristicCounter);
    let ctx_tokens = counter.count(&ctx);
    let ts = s.task_state()?;
    println!("  ไม่มี Chorus   codex เริ่มจากศูนย์ — ไม่รู้ goal, ไม่รู้ว่าอะไรทำไปแล้ว,");
    println!("                 ไม่รู้ข้อห้าม, ไม่เห็น diff · เพื่อนต้องเล่าใหม่เองทุกครั้ง");
    println!("  มี Chorus      {ctx_tokens} token ประกอบจาก ledger อัตโนมัติ");
    println!("                 goal ✓ · done {} ข้อ · ไฟล์ที่แตะ {} · diff ✓",
        ts.done.len(),
        list("files").len());

    // ---- 6. one memory, many files ---------------------------------------
    rule("6. ความจำก้อนเดียว ฉายออกไปหลายไฟล์");
    let mut seen = Vec::new();
    for f in ["CLAUDE.md", "AGENTS.md", "GEMINI.md"] {
        if let Ok(t) = std::fs::read_to_string(dir.join(f)) {
            seen.push((f, t.len()));
        }
    }
    if seen.is_empty() {
        println!("  (ยังไม่มี fact ให้ฉาย — projector เขียนเมื่อมีความจำหรือ task state)");
    } else {
        for (f, n) in &seen {
            println!("  {f:<12} {n} bytes");
        }
        println!("  เนื้อเดียวกันทั้งหมด ต่างแค่ชื่อไฟล์ที่แต่ละเจ้าไปอ่าน");
        println!("  แก้ที่เดียว ไม่ต้องไล่แก้สามที่");
    }

    // ---- 7. the ledger ---------------------------------------------------
    rule("7. ledger เป็นของแอป ไม่ใช่ของ provider");
    let store = Store::open(&db)?;
    let c = store.conn();
    for e in ledger::history(c, sess, 400)? {
        let who = actor::name(c, e.actor).unwrap_or_else(|_| "?".into());
        let body = ledger::body(c, &e).unwrap_or_default();
        let kind = match e.kind {
            EventKind::Msg => "msg",
            EventKind::ToolCall => "tool",
            EventKind::ToolResult => "result",
            EventKind::Diff => "diff",
            EventKind::Decision => "decision",
            EventKind::Handoff => "handoff",
            EventKind::Summary => "summary",
        };
        let line = if e.kind == EventKind::Handoff {
            let b: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
            format!(
                "{} \u{2192} {} ({})",
                b["handoff"]["from"].as_str().unwrap_or("?"),
                b["handoff"]["to"].as_str().unwrap_or("?"),
                b["handoff"]["reason"].as_str().unwrap_or("handoff"),
            )
        } else {
            body.lines().next().unwrap_or("").chars().take(52).collect()
        };
        println!("  #{:<3} {:<12} {:<9} {}", e.seq, who, kind, line);
    }
    println!("\n  สองเจ้าคนละบริษัท อยู่ในเทรดเดียวกัน เรียงต่อกันจริง ๆ");
    println!("  ถ้าปิดแอปตอนนี้แล้วเปิดใหม่ ทุกอย่างยังอยู่ — ไม่ได้ฝากไว้กับ provider ไหนเลย");

    rule("จบ");
    println!("  ลองเองต่อได้:");
    println!("    chorus run {} \"<โจทย์>\" --db {} --yes", dir.display(), db.display());
    println!("    chorus compact --db {} --session {sess}", db.display());
    println!("    chorus context --db {} --session {sess}   # ดูว่าเจ้าต่อไปได้อะไรไปจริง ๆ", db.display());
    if keep {
        println!("\n  เก็บไว้ที่ {}", dir.display());
    } else {
        let _ = std::fs::remove_dir_all(&dir);
        println!("\n  (ลบ workspace ชั่วคราวแล้ว — ใส่ --keep ถ้าอยากเก็บไว้ดู)");
    }
    Ok(())
}

/// `chorus doctor`
///
/// Why a turn will or will not run, before you try it. Every check here is the
/// same one the macOS Settings pane makes — the two must never disagree, so
/// they call the same code (`chorus_cli::agents`).
///
/// Nothing here logs in, reads a credential file, or touches the network.
fn doctor_cmd() -> Result<()> {
    use chorus_cli::agents::{list_agents, AuthState};

    println!("\u{2501}\u{2501} agents \u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}");
    let agents = list_agents();
    let mut ready = 0;
    for a in &agents {
        let (mark, state) = match a.auth {
            AuthState::Ok => ("\u{25cf}", "พร้อมใช้"),
            AuthState::NeedsLogin => ("\u{25cf}", "ยังไม่ได้ login"),
            AuthState::MissingCli => ("\u{25cb}", "ยังไม่ได้ติดตั้ง CLI"),
        };
        if matches!(a.auth, AuthState::Ok) {
            ready += 1;
        }
        println!("  {mark} {:<8} {:<22} {}", a.label, state, a.detail);
        if !matches!(a.auth, AuthState::Ok) {
            println!("      แก้ด้วย: {}", a.hint);
        }
        println!("      bridge:  {}", a.bridge.join(" "));
        let models: Vec<String> = a
            .models
            .iter()
            .map(|m| if m.is_default { format!("{}*", m.id) } else { m.id.clone() })
            .collect();
        let src = match a.model_source {
            chorus_cli::agents::ModelSource::Probed => "ถามจาก agent แล้ว",
            chorus_cli::agents::ModelSource::Env => "จาก env",
            chorus_cli::agents::ModelSource::Fallback => "ยังไม่เคยถาม — เป็นค่าเดา รัน `chorus models` เพื่อดึงของจริง",
        };
        println!("      models:  {}  (* = ดีฟอลต์)", models.join(", "));
        println!("               {src}");
        println!();
    }

    println!("\u{2501}\u{2501} สรุป \u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}");
    match ready {
        0 => println!("  ยังรันไม่ได้เลย — ต้อง login อย่างน้อยหนึ่งเจ้าก่อน"),
        1 => println!(
            "  รันได้ 1 เจ้า — พอทำงานได้ แต่ยังสลับตอนชนลิมิตไม่ได้\n  \
             (ต้องมีอย่างน้อย 2 เจ้าถึงจะได้ failover กับ compare)"
        ),
        n => println!("  รันได้ {n} เจ้า — สลับตอนชนลิมิตและเทียบหลายตัวได้ครบ"),
    }

    let unmatched = chorus_cli::limits::unclassified_failures(5);
    if !unmatched.is_empty() {
        println!("\n\u{2501}\u{2501} error ที่ failover ไม่รู้จัก (ล่าสุด {}) \u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}", unmatched.len());
        for f in &unmatched {
            println!("  {:<10} {}", f.agent, f.message);
        }
        println!("  ถ้าข้อไหนเป็นข้อความลิมิต ให้เพิ่มคำนั้นใน HandoffReason::from_failure");
        println!("  และเก็บข้อความเต็มแบบคำต่อคำไว้ใน tests/handoff.rs (CLAUDE.md 7.6b, 7.17)");
    }

    println!("\n\u{2501}\u{2501} env ที่ override ได้ \u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501}");
    for (key, what) in [
        ("CHORUS_CLAUDE_BRIDGE", "คำสั่ง bridge ของ Claude"),
        ("CHORUS_CODEX_BRIDGE", "คำสั่ง bridge ของ Codex"),
        ("CHORUS_CLAUDE_MODELS", "รายการโมเดล Claude (id=ชื่อ, คั่นจุลภาค)"),
        ("CHORUS_CODEX_MODELS", "รายการโมเดล Codex"),
        ("CHORUS_SUMMARY_MODEL", "โมเดล Ollama สำหรับย่อ (ว่าง = ใช้ extractive)"),
        ("CHORUS_COMPACT_BUDGET_MS", "เพดานเวลาต่อรอบ compact (ดีฟอลต์ 3000)"),
    ] {
        match std::env::var(key) {
            Ok(v) if !v.trim().is_empty() => println!("  {key:<26} = {v}"),
            _ => println!("  {key:<26}   (ไม่ได้ตั้ง — {what})"),
        }
    }
    Ok(())
}

/// `chorus agent <subcommand> [options]`
///
/// Dual Control Surface CLI companion for agent orchestration and task visibility.
fn agent_cmd(args: &[String]) -> Result<()> {
    use chorus_core::{agent_control, roles, Store};

    let sub = args.first().map(String::as_str).unwrap_or("help");
    let mut db: Option<PathBuf> = None;
    let mut session_id: Option<i64> = None;
    let mut rest: Vec<&String> = Vec::new();
    let mut it = args.iter().skip(1);
    let mut worktree = false;
    let mut join_mode = "compact".to_string();
    let mut name_hint: Option<String> = None;
    let mut save_role = false;

    while let Some(a) = it.next() {
        match a.as_str() {
            "--db" => db = it.next().map(PathBuf::from),
            "--session" => session_id = it.next().and_then(|v| v.parse().ok()),
            "--worktree" => worktree = true,
            "--mode" => join_mode = it.next().cloned().unwrap_or_else(|| "compact".into()),
            "--name" => name_hint = it.next().cloned(),
            "--save" => save_role = true,
            _ => rest.push(a),
        }
    }

    let db_path = db.unwrap_or_else(|| PathBuf::from(".chorus/store.sqlite3"));

    match sub {
        "list" => {
            let store = Store::open(&db_path).context("open store")?;
            let agents = agent_control::list_subagents(store.conn(), session_id)?;
            if agents.is_empty() {
                println!("No subagents found.");
                return Ok(());
            }
            println!("ID | Role | Status | Task | Worktree");
            println!("---+------+--------+------+---------");
            for a in agents {
                println!(
                    "{} | {} | {} | {} | {}",
                    a.id,
                    a.role,
                    a.status.as_str(),
                    a.task.chars().take(40).collect::<String>(),
                    a.worktree.as_deref().unwrap_or("-")
                );
            }
        }
        "spawn" => {
            let role = rest.first().context("agent spawn <role> <task>")?;
            let task = rest.get(1).context("agent spawn <role> <task>")?;
            let store = Store::open(&db_path).context("open store")?;
            let cwd = std::env::current_dir()?.to_string_lossy().to_string();
            let agent = agent_control::spawn_agent(
                store.conn(),
                session_id,
                role,
                task,
                worktree,
                &cwd,
            )?;
            println!("Spawned child agent #{} (role: {})", agent.id, agent.role);
        }
        "message" => {
            let id: i64 = rest.first().and_then(|s| s.parse().ok()).context("agent message <id> <text>")?;
            let text = rest.get(1).context("agent message <id> <text>")?;
            let store = Store::open(&db_path).context("open store")?;
            // Typed by the person at the CLI, so it is the user speaking.
            let user = actor::intern(store.conn(), "user", "human")?;
            let msg = agent_control::send_message(store.conn(), id, user, text)?;
            println!("Sent message to agent #{}: {}", id, msg.content);
        }
        "wait" => {
            let id: Option<i64> = rest.first().and_then(|s| s.parse().ok());
            let store = Store::open(&db_path).context("open store")?;
            let rep = agent_control::wait_agent(store.conn(), session_id, id, 30)?;
            println!("Wait report: all_done={}, running={:?}, completed={:?}", rep.all_done, rep.running, rep.completed);
        }
        "collect" => {
            let id: Option<i64> = rest.first().and_then(|s| s.parse().ok());
            let ids_vec = id.map(|i| vec![i]);
            let store = Store::open(&db_path).context("open store")?;
            let brief = agent_control::collect_results(store.conn(), session_id, ids_vec.as_deref(), &join_mode)?;
            print!("{}", brief.rendered_markdown);
        }
        "cancel" => {
            let id: i64 = rest.first().and_then(|s| s.parse().ok()).context("agent cancel <id>")?;
            let store = Store::open(&db_path).context("open store")?;
            let agent = agent_control::cancel_agent(store.conn(), id, None)?;
            println!("Cancelled agent #{}", agent.id);
        }
        "roles" => {
            let cwd = std::env::current_dir()?;
            let loaded = roles::load_roles(&cwd);
            println!("Available Agent Roles:");
            for r in loaded {
                println!("- {} : {}", r.name, r.description);
            }
        }
        "synthesize-role" => {
            let task = rest.first().context("agent synthesize-role <task> [--name <name>] [--save]")?;
            let role = roles::synthesize_role(task, worktree, name_hint.as_deref());
            println!("{}", role.to_markdown());
            if save_role {
                let cwd = std::env::current_dir()?;
                let p = role.save_to_dir(&cwd)?;
                println!("Saved role definition to {}", p.display());
            }
        }
        "checklist" => {
            let store = Store::open(&db_path).context("open store")?;
            let sess = session_id.unwrap_or(1);
            let items = agent_control::get_checklist(store.conn(), sess)?;
            if items.is_empty() {
                println!("No checklist items for session #{sess}.");
            } else {
                for i in items {
                    let marker = if i.done { "[x]" } else if i.in_progress { "[~]" } else { "[ ]" };
                    println!("{} {}", marker, i.text);
                }
            }
        }
        "jump" => {
            let path = rest.first().context("agent jump <path> [line]")?;
            let line: Option<u32> = rest.get(1).and_then(|s| s.parse().ok());
            let store = Store::open(&db_path).context("open store")?;
            let sess = session_id.unwrap_or(1);
            let jump = agent_control::jump_to_file(store.conn(), sess, path, line)?;
            println!("Jump directive recorded: {}:{:?}", jump.path, jump.line);
        }
        "review" => {
            let summary = rest.first().context("agent review <summary>")?;
            let store = Store::open(&db_path).context("open store")?;
            let sess = session_id.unwrap_or(1);
            let req = agent_control::request_review(store.conn(), sess, summary, &serde_json::json!([]))?;
            println!("Review request #{} created: status={}", req.id, req.status);
        }
        _ => {
            eprintln!("Usage: chorus agent <list|spawn|message|wait|collect|cancel|roles|synthesize-role|checklist|jump|review> [options]");
        }
    }
    Ok(())
}

