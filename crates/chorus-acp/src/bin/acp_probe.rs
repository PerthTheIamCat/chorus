//! `acp-probe` — run one scripted ACP conversation against a real agent and
//! write down exactly what came back.
//!
//! Usage:
//!   acp-probe [--cwd DIR] [--out FILE] [--prompt TEXT] [--mode ID] [--allow] (--mock | -- <agent cmd...>)
//!
//! `--mode ID` sends `session/set_mode` right after `session/new` (e.g. `read-only`
//! for Codex) to try to force every approval back through us. Valid ids come from
//! `initialize`'s `modes.availableModes`. Note: the Codex bridge accepts the call
//! but does not actually honour it — see CLAUDE.md §8 spike 1 Q2.
//!
//! By default the transcript is written to `<cwd>/.chorus-probe/acp-probe-<ms>.jsonl`.
//! Pass `--out FILE` to choose an explicit output path.
//!
//! Examples:
//!   acp-probe --cwd /tmp/scratch --mock              # ลองกับ agent ปลอมในตัว
//!   acp-probe --cwd /tmp/scratch -- codex acp
//!   acp-probe --cwd /tmp/scratch -- antigravity-acp
//!
//! By default every permission request is DENIED. That is on purpose: we want to
//! see the shape of the request, not let an agent loose on a real repo. Pass
//! `--allow` in a throwaway directory to also see what diffs look like.

use anyhow::{anyhow, Result};
use chorus_acp::{Agent, Inbound, Log};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt::Write;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Default)]
struct Findings {
    protocol_version: Option<Value>,
    agent_capabilities: Option<Value>,
    auth_methods: Option<Value>,
    session_id: Option<String>,
    new_session_extra: Vec<String>,
    requested_mode: Option<String>,
    set_mode_outcome: Option<String>,
    update_kinds: BTreeMap<String, usize>,
    inbound_requests: BTreeMap<String, usize>,
    permission_options: Vec<Value>,
    stop_reason: Option<Value>,
    load_supported: Option<bool>,
    load_result: Option<Value>,
    load_replayed_updates: usize,
    errors: Vec<String>,
}

/// `session/update` carries a tagged union; the tag tells us whether we get
/// structured diffs and terminals or just text.
fn update_kind(params: &Value) -> String {
    params
        .get("update")
        .and_then(|u| u.get("sessionUpdate"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| "<no sessionUpdate tag>".into())
}

struct Opts {
    cwd: PathBuf,
    out: PathBuf,
    default_out: bool,
    prompt: String,
    mode: Option<String>,
    allow: bool,
    cmd: Vec<String>,
}

fn parse_args() -> Result<Opts> {
    let mut cwd = std::env::current_dir()?;
    let mut out: Option<PathBuf> = None;
    let mut prompt = String::from(
        "List the files in this directory, then tell me the first heading of README.md. \
         Do not modify anything.",
    );
    let mut mode: Option<String> = None;
    let mut allow = false;
    let mut cmd: Vec<String> = Vec::new();
    let mut mock = false;

    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--cwd" => cwd = PathBuf::from(it.next().ok_or_else(|| anyhow!("--cwd needs a value"))?),
            "--out" => out = Some(PathBuf::from(it.next().ok_or_else(|| anyhow!("--out needs a value"))?)),
            "--prompt" => prompt = it.next().ok_or_else(|| anyhow!("--prompt needs a value"))?,
            "--mode" => mode = Some(it.next().ok_or_else(|| anyhow!("--mode needs a value"))?),
            "--allow" => allow = true,
            "--mock" => mock = true,
            "--" => {
                cmd.extend(it.by_ref());
                break;
            }
            other => return Err(anyhow!("unknown argument {other:?}; agent command goes after --")),
        }
    }
    if mock && cmd.is_empty() {
        // Re-run ourselves in mock mode: no second binary to build, nothing to
        // put on PATH, no way to point at a stale copy.
        cmd = vec![std::env::current_exe()?.to_string_lossy().into_owned(), "--serve-mock".into()];
    }
    if cmd.is_empty() {
        return Err(anyhow!(
            "no agent command. try:\n  \
             acp-probe --cwd /tmp/scratch --mock            (agent ปลอมในตัว)\n  \
             acp-probe --cwd /tmp/scratch -- codex acp\n  \
             acp-probe --cwd /tmp/scratch -- antigravity-acp"
        ));
    }
    let cwd = cwd.canonicalize().unwrap_or(cwd);
    let default_out = out.is_none();
    let out = out.unwrap_or_else(|| cwd.join(".chorus-probe").join(format!("acp-probe-{}.jsonl", chorus_acp::now_ms())));
    Ok(Opts { cwd, out, default_out, prompt, mode, allow, cmd })
}

/// Serve `fs/read_text_file` ourselves, but only inside the probe's cwd. The
/// client owning file access is the part of ACP we most want to confirm works.
fn read_text_file(cwd: &Path, params: &Value) -> Result<Value> {
    let path = params.get("path").and_then(Value::as_str).ok_or_else(|| anyhow!("no path"))?;
    let p = PathBuf::from(path);
    let canon = p.canonicalize().map_err(|e| anyhow!("{path}: {e}"))?;
    if !canon.starts_with(cwd) {
        return Err(anyhow!("refused: {} is outside {}", canon.display(), cwd.display()));
    }
    let text = std::fs::read_to_string(&canon)?;
    Ok(json!({ "content": text }))
}

fn main() -> Result<()> {
    // Mock mode: this process *is* the agent for whoever spawned us.
    if std::env::args().any(|a| a == "--serve-mock") {
        return chorus_acp::mock::serve();
    }

    let opts = parse_args()?;
    if opts.default_out {
        std::fs::create_dir_all(opts.cwd.join(".chorus-probe"))?;
    }
    let file = File::create(&opts.out)?;
    let log: Log = Arc::new(Mutex::new(Box::new(file)));
    let f = RefCell::new(Findings::default());

    println!("agent      {:?}", opts.cmd);
    println!("cwd        {}", opts.cwd.display());
    println!("permission {}", if opts.allow { "ALLOW (throwaway dir!)" } else { "deny (default)" });
    if let Some(m) = &opts.mode {
        println!("mode       {m} (via session/set_mode)");
    }
    println!("transcript {}\n", opts.out.display());

    let mut agent = match Agent::spawn(&opts.cmd, log.clone()) {
        Ok(a) => a,
        Err(e) => {
            return Err(anyhow!(
                "{e}\n  รันคำสั่งนี้เองในเทอร์มินัลดูก่อนว่ามันมีจริงไหม: {}",
                opts.cmd.join(" ")
            ))
        }
    };

    // Anything the agent sends us mid-call lands here.
    let cwd = opts.cwd.clone();
    let allow = opts.allow;
    let mut handle = |a: &Agent, inbound: Inbound| match inbound {
        Inbound::Notification { method, params } => {
            if method == "session/update" {
                *f.borrow_mut().update_kinds.entry(update_kind(&params)).or_insert(0) += 1;
            } else {
                *f.borrow_mut().update_kinds.entry(format!("<{method}>")).or_insert(0) += 1;
            }
        }
        Inbound::Request { id, method, params } => {
            *f.borrow_mut().inbound_requests.entry(method.clone()).or_insert(0) += 1;
            let r = match method.as_str() {
                "session/request_permission" => {
                    let options = params.get("options").cloned().unwrap_or(Value::Null);
                    if f.borrow().permission_options.is_empty() {
                        if let Some(arr) = options.as_array() {
                            f.borrow_mut().permission_options = arr.clone();
                        }
                    }
                    // Pick by the option's own `kind`, never by position.
                    let want = if allow { "allow_once" } else { "reject_once" };
                    let chosen = options.as_array().and_then(|arr| {
                        arr.iter()
                            .find(|o| o.get("kind").and_then(Value::as_str) == Some(want))
                            .or_else(|| arr.first())
                    });
                    match chosen.and_then(|o| o.get("optionId")).and_then(Value::as_str) {
                        Some(oid) => a.respond(
                            &id,
                            json!({"outcome":{"outcome":"selected","optionId":oid}}),
                        ),
                        None => a.respond(&id, json!({"outcome":{"outcome":"cancelled"}})),
                    }
                }
                "fs/read_text_file" => match read_text_file(&cwd, &params) {
                    Ok(v) => a.respond(&id, v),
                    Err(e) => a.respond_err(&id, -32000, &e.to_string()),
                },
                "fs/write_text_file" if allow => {
                    // Even with --allow we do not write. Seeing the request is the point.
                    a.respond_err(&id, -32000, "probe never writes files")
                }
                _ => a.respond_err(&id, -32601, "probe does not implement this method"),
            };
            if let Err(e) = r {
                f.borrow_mut().errors.push(format!("failed to answer {method}: {e}"));
            }
        }
    };

    // 1 ── initialize
    let id = agent.request(
        "initialize",
        json!({
            "protocolVersion": 1,
            "clientCapabilities": { "fs": { "readTextFile": true, "writeTextFile": false }, "terminal": false },
            "clientInfo": { "name": "chorus-acp-probe", "title": "Chorus ACP probe", "version": "0.1.0" }
        }),
    )?;
    let init = match agent.wait_for(id, Duration::from_secs(30), &mut handle) {
        Ok(v) => v,
        Err(e) => {
            let detail = agent.diagnose(&e);
            return Err(anyhow!(
                "agent ไม่ตอบ initialize\n  {detail}\n\n  \
                 ถ้าเป็น \"unrecognized subcommand\" แปลว่าเวอร์ชันที่ลงไว้ยังไม่มีโหมด ACP ในตัว\n  \
                 ลอง `codex --help` ดูว่ามีคำสั่งไหนที่ใช่ หรือใช้ bridge แยก แล้วชี้ path มาที่ binary นั้น\n  \
                 เฟรมและ stderr ทั้งหมดอยู่ใน {}",
                opts.out.display()
            ));
        }
    };
    f.borrow_mut().protocol_version = init.get("protocolVersion").cloned();
    f.borrow_mut().agent_capabilities = init.get("agentCapabilities").cloned();
    f.borrow_mut().auth_methods = init.get("authMethods").cloned();
    f.borrow_mut().load_supported = init
        .get("agentCapabilities")
        .and_then(|c| c.get("loadSession"))
        .and_then(Value::as_bool);

    // 2 ── session/new
    let id = agent.request(
        "session/new",
        json!({ "cwd": opts.cwd.to_string_lossy(), "mcpServers": [] }),
    )?;
    let new = agent.wait_for(id, Duration::from_secs(60), &mut handle)?;
    f.borrow_mut().session_id =
        new.get("sessionId").and_then(Value::as_str).map(str::to_string);
    if let Some(obj) = new.as_object() {
        f.borrow_mut().new_session_extra = obj.keys().filter(|k| *k != "sessionId").cloned().collect();
    }
    let session_id = f
        .borrow()
        .session_id
        .clone()
        .ok_or_else(|| anyhow!("agent returned no sessionId: {new}"))?;

    // 2b ── session/set_mode: try to force approvals back through us before prompting
    if let Some(mode) = opts.mode.clone() {
        f.borrow_mut().requested_mode = Some(mode.clone());
        let id = agent.request("session/set_mode", json!({ "sessionId": session_id, "modeId": mode }))?;
        let outcome = match agent.wait_for(id, Duration::from_secs(30), &mut handle) {
            Ok(v) => format!("ok: {v}"),
            Err(e) => {
                f.borrow_mut().errors.push(format!("session/set_mode: {e}"));
                format!("failed: {e}")
            }
        };
        f.borrow_mut().set_mode_outcome = Some(outcome);
    }

    // 3 ── session/prompt
    let id = agent.request(
        "session/prompt",
        json!({
            "sessionId": session_id,
            "prompt": [{ "type": "text", "text": opts.prompt }]
        }),
    )?;
    match agent.wait_for(id, Duration::from_secs(300), &mut handle) {
        Ok(res) => f.borrow_mut().stop_reason = res.get("stopReason").cloned(),
        Err(e) => f.borrow_mut().errors.push(format!("session/prompt: {e}")),
    }
    agent.drain(Duration::from_millis(400), &mut handle);

    // 4 ── session/load, the question the whole adapter layer rests on
    if f.borrow().load_supported == Some(true) {
        let before: usize = f.borrow().update_kinds.values().sum();
        let id = agent.request(
            "session/load",
            json!({ "sessionId": session_id, "cwd": opts.cwd.to_string_lossy(), "mcpServers": [] }),
        )?;
        match agent.wait_for(id, Duration::from_secs(120), &mut handle) {
            Ok(res) => {
                f.borrow_mut().load_result = Some(res);
                agent.drain(Duration::from_millis(400), &mut handle);
                let after: usize = f.borrow().update_kinds.values().sum();
                f.borrow_mut().load_replayed_updates = after.saturating_sub(before);
            }
            Err(e) => f.borrow_mut().errors.push(format!("session/load: {e}")),
        }
    }

    report(&f.borrow(), &opts.out);
    Ok(())
}

fn report(f: &Findings, out: &Path) {
    let mut s = String::new();
    let w = &mut s;
    let _ = writeln!(w, "\n══ ผลการ probe ══════════════════════════════════════");
    let _ = writeln!(w, "  protocolVersion   {}", show(&f.protocol_version));
    let _ = writeln!(w, "  authMethods       {}", show(&f.auth_methods));
    let _ = writeln!(w, "  agentCapabilities {}", show(&f.agent_capabilities));

    let _ = writeln!(w, "\n── Q1: session/load ใช้ได้จริงไหม ────────────────────");
    match f.load_supported {
        Some(true) => {
            let _ = writeln!(w, "  รองรับ: ใช่");
            let _ = writeln!(w, "  ผลลัพธ์: {}", show(&f.load_result));
            let _ = writeln!(w, "  replay update หลัง load: {} เฟรม", f.load_replayed_updates);
            let _ = writeln!(
                w,
                "  → {}",
                if f.load_replayed_updates > 0 {
                    "agent เล่นประวัติย้อนกลับมาให้ เราสร้าง ledger ต่อจากของมันได้"
                } else {
                    "load สำเร็จแต่ไม่ replay อะไร — ประวัติต้องมาจาก ledger ของเราเอง"
                }
            );
        }
        Some(false) => {
            let _ = writeln!(w, "  รองรับ: ไม่ — ต้อง replay จาก ledger ของเราเองทุกครั้ง");
        }
        None => {
            let _ = writeln!(w, "  ไม่ประกาศ capability นี้ ถือว่าไม่รองรับ");
        }
    }
    let _ = writeln!(w, "  sessionId         {:?}", f.session_id);
    if !f.new_session_extra.is_empty() {
        let _ = writeln!(w, "  session/new แถมมา  {:?}", f.new_session_extra);
    }

    let _ = writeln!(w, "\n── Q2: permission คุมจากฝั่งเราได้แค่ไหน ─────────────");
    if let Some(m) = &f.requested_mode {
        let outcome = f.set_mode_outcome.as_deref().unwrap_or("ไม่มีผลตอบกลับ");
        let _ = writeln!(w, "  session/set_mode {m:?} → {outcome}");
    }
    if f.inbound_requests.is_empty() {
        let _ = writeln!(w, "  agent ไม่ขออนุญาตอะไรเลยในรอบนี้");
        let hint = if f.requested_mode.is_some() {
            "แม้ตั้ง mode แล้วก็ยังไม่ขอ — ลอง prompt ที่แตะไฟล์นอก cwd หรือรันคำสั่ง"
        } else {
            "ลอง --mode read-only + prompt ที่ต้องแก้ไฟล์"
        };
        let _ = writeln!(w, "  → {hint}");
    } else {
        for (m, n) in &f.inbound_requests {
            let _ = writeln!(w, "  {m:<28} {n} ครั้ง");
        }
    }
    if !f.permission_options.is_empty() {
        let _ = writeln!(w, "  ตัวเลือกที่มันเสนอมา:");
        for o in &f.permission_options {
            let _ = writeln!(
                w,
                "    kind={:<14} id={:<20} name={}",
                o.get("kind").and_then(Value::as_str).unwrap_or("?"),
                o.get("optionId").and_then(Value::as_str).unwrap_or("?"),
                o.get("name").and_then(Value::as_str).unwrap_or("?")
            );
        }
    }

    let _ = writeln!(w, "\n── Q3: update ที่ได้เป็นโครงสร้างหรือข้อความ ──────────");
    if f.update_kinds.is_empty() {
        let _ = writeln!(w, "  ไม่มี session/update เลย");
    } else {
        for (k, n) in &f.update_kinds {
            let _ = writeln!(w, "  {k:<28} {n}");
        }
    }
    let structured = ["tool_call", "tool_call_update", "plan", "diff"];
    let got: Vec<&str> =
        structured.iter().copied().filter(|k| f.update_kinds.contains_key(*k)).collect();
    let _ = writeln!(
        w,
        "  → {}",
        if got.is_empty() {
            "ได้แต่ข้อความ — diff/tool ต้องแกะจาก text เอง (แพงและเปราะ)".to_string()
        } else {
            format!("ได้ของมีโครงสร้าง: {got:?}")
        }
    );

    let _ = writeln!(w, "\n  stopReason        {}", show(&f.stop_reason));
    if !f.errors.is_empty() {
        let _ = writeln!(w, "\n  ปัญหาที่เจอ:");
        for e in &f.errors {
            let _ = writeln!(w, "    - {e}");
        }
    }
    let _ = writeln!(w, "\n  เฟรมทั้งหมดอยู่ใน {}", out.display());
    println!("{s}");
}

fn show(v: &Option<Value>) -> String {
    v.as_ref().map(|v| v.to_string()).unwrap_or_else(|| "—".into())
}
