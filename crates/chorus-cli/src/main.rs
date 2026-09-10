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
use chorus_cli::session::{Policy, Session};
use chorus_core::handoff::HandoffReason;
use chorus_core::context::{AssembleInput, Assembler, Budget, Tier, TurnItem};
use chorus_core::render::{self, ContextItem, FileChange};
use chorus_core::tokens::{CachedCounter, HeuristicCounter, TokenCounter};
use chorus_core::{actor, ledger, memory, EventKind, NewEvent, Store};
use std::path::PathBuf;

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
        other => {
            eprintln!("unknown command {other:?}; try: chorus bench | chorus run | chorus handoff");
            std::process::exit(2);
        }
    }
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
            other => anyhow::bail!("unknown flag {other:?}"),
        }
    }
    let db = db.context("--db is required")?;
    anyhow::ensure!(!goal.is_empty(), "--goal is required");
    let diff = match diff_file {
        Some(p) => std::fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?,
        None => String::new(),
    };

    let mut s = Session::open(&db, session_id).context("open session")?;
    // Constraints come from project-scope memory facts.
    let constraints: Vec<String> = Vec::new();

    let r = s.handoff(&to, "openai", reason, &goal, done, pending, constraints, diff, policy)?;

    println!("── brief handed to {} (seq {}) ──", r.to, r.handoff_seq);
    println!("{}", r.brief.render_markdown());
    println!("\n── {} echo-back ──────────────────────────────", r.to);
    println!("{}", r.echo_back.trim());
    println!("\nstop: {} · {} events recorded", r.stop_reason, r.agent_seqs.len());
    Ok(())
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

    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--db" => db = it.next().map(PathBuf::from),
            "--agent" => {
                if let Some(v) = it.next() {
                    agent = v.clone();
                }
            }
            "--yes" => policy = Policy::AllowAll,
            _ => positional.push(a),
        }
    }
    let [cwd, prompt] = positional.as_slice() else {
        anyhow::bail!("usage: chorus run <cwd> <prompt> [--db PATH] [--agent NAME] [--yes]");
    };

    let db = db.unwrap_or_else(|| {
        std::env::temp_dir().join(format!("chorus-run-{}.db", std::process::id()))
    });
    let cwd = PathBuf::from(cwd);
    let mut s = Session::create(&db, &cwd, prompt).context("create session")?;
    eprintln!("session {} · store {}", s.session_id(), db.display());

    let report = s.run_turn(&agent, "openai", prompt, policy)?;

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
        &NewEvent::new(sess, EventKind::Msg, user, "หน้า order list ช้ามาก ช่วยดูให้หน่อย"),
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
    let blob_bytes: i64 = c
        .query_row("select coalesce(sum(length(bytes)),0) from blob", [], |r| r.get(0))?;
    let orig_bytes: i64 = c
        .query_row("select coalesce(sum(orig_len),0) from blob", [], |r| r.get(0))?;
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
                text_l1: Some(format!("[สรุป] {}", body.chars().take(80).collect::<String>())),
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
                FileChange { path: "src/order/repo.ts".into(), status: "modified".into(), added: 38, removed: 11 },
                FileChange { path: "migrations/0042_idx.sql".into(), status: "added".into(), added: 6, removed: 0 },
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
        println!("  demoted / dropped     {} / {}", out.demoted_turns, out.dropped_turns);
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
    println!("  store.db              {:.1} KB for {event_rows} events", db_size as f64 / 1024.0);
    println!("  per event             {:.0} bytes", db_size as f64 / event_rows as f64);
    println!("\n  {}", db_path.display());
    Ok(())
}
