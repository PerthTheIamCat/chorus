//! `antigravity-acp` — Lightweight Agent Client Protocol (ACP) bridge for Antigravity (`agy`).
//!
//! Spoken over stdin/stdout as newline-delimited JSON-RPC 2.0. Translates ACP session
//! lifecycle and prompt turns into Antigravity CLI invocations with `--output-format stream-json`.

use serde_json::{json, Value};
use std::io::{self, BufRead, BufReader, Write};
use std::process::{Command, Stdio};

fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    let mut current_cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let mut current_model = String::from("gemini-3.8-flash[high]");
    let mut session_counter = 0;

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let req: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let id = req.get("id").cloned();
        let method = req.get("method").and_then(Value::as_str).unwrap_or("");
        let params = req.get("params").cloned().unwrap_or(Value::Null);

        match method {
            "initialize" => {
                let res = json!({
                    "protocolVersion": 1,
                    "agentCapabilities": {
                        "modes": {
                            "availableModes": [
                                { "id": "default", "name": "Default" }
                            ]
                        },
                        "prompt": {
                            "image": true,
                            "embeddedContext": true
                        }
                    },
                    "agentInfo": {
                        "name": "antigravity",
                        "version": "1.3.2"
                    }
                });
                send_response(&mut out, id, Ok(res))?;
            }
            "session/new" => {
                session_counter += 1;
                let session_id = format!("agy-{session_counter}");
                if let Some(cwd_str) = params.get("cwd").and_then(Value::as_str) {
                    current_cwd = std::path::PathBuf::from(cwd_str);
                }

                let models = probe_models();
                let res = json!({
                    "sessionId": session_id,
                    "models": {
                        "availableModels": models
                    }
                });
                send_response(&mut out, id, Ok(res))?;
            }
            "session/set_model" => {
                if let Some(m) = params.get("modelId").and_then(Value::as_str) {
                    current_model = m.to_string();
                }
                send_response(&mut out, id, Ok(json!({})))?;
            }
            "session/set_config_option" => {
                if let Some(cfg) = params.get("configId").and_then(Value::as_str) {
                    if cfg == "model" {
                        if let Some(val) = params.get("value").and_then(Value::as_str) {
                            current_model = val.to_string();
                        }
                    }
                }
                send_response(&mut out, id, Ok(json!({})))?;
            }
            "session/load" => {
                send_response(&mut out, id, Ok(json!({})))?;
            }
            "session/prompt" => {
                let session_id = params.get("sessionId").and_then(Value::as_str).unwrap_or("agy-1");
                let prompt_text = extract_prompt(&params);

                let agy_bin = find_agy_binary();
                let mut cmd = Command::new(&agy_bin);
                // The prompt (context, memory, attachments) goes over stdin,
                // never argv: argv is readable by every local user via `ps`
                // and long prompts hit ARG_MAX.
                cmd.args(agy_args(&current_model))
                    .current_dir(&current_cwd)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped());

                match cmd.spawn() {
                    Ok(mut child) => {
                        // Write from a thread so a large prompt cannot deadlock
                        // against agy filling its stdout pipe; dropping the
                        // handle closes stdin, which ends the one-turn input.
                        let feeder = child.stdin.take().map(|mut stdin| {
                            let line = stdin_message(&prompt_text);
                            std::thread::spawn(move || {
                                let _ = stdin.write_all(line.as_bytes());
                            })
                        });
                        let mut sent_any = false;
                        if let Some(stdout_stream) = child.stdout.take() {
                            let reader = BufReader::new(stdout_stream);
                            for stream_line in reader.lines().flatten() {
                                if let Ok(event_json) = serde_json::from_str::<Value>(stream_line.trim()) {
                                    handle_agy_event(&mut out, session_id, &event_json, &mut sent_any)?;
                                }
                            }
                        }
                        let _ = child.wait();
                        if let Some(f) = feeder {
                            let _ = f.join();
                        }
                        if !sent_any {
                            let mut err_str = String::new();
                            if let Some(mut err_stream) = child.stderr.take() {
                                use std::io::Read;
                                let _ = err_stream.read_to_string(&mut err_str);
                            }
                            if !err_str.trim().is_empty() {
                                send_notification(
                                    &mut out,
                                    "session/update",
                                    json!({
                                        "sessionId": session_id,
                                        "update": {
                                            "sessionUpdate": "agent_message_chunk",
                                            "content": {
                                                "type": "text",
                                                "text": format!("Antigravity CLI error: {}", err_str.trim())
                                            }
                                        }
                                    }),
                                )?;
                            }
                        }
                        send_response(&mut out, id, Ok(json!({ "stopReason": "end_turn" })))?;
                    }
                    Err(e) => {
                        let err_msg = format!("Failed to spawn agy ({:?}): {e}", agy_bin);
                        send_response(&mut out, id, Err((-32603, err_msg)))?;
                    }
                }
            }
            _ => {
                if id.is_some() {
                    send_response(&mut out, id, Err((-32601, format!("Method not found: {method}"))))?;
                }
            }
        }
    }

    Ok(())
}

/// `agy` arguments for one print-mode turn. Deliberately has no prompt in it.
fn agy_args(current_model: &str) -> Vec<String> {
    let mut args: Vec<String> = vec!["--input-format".into(), "stream-json".into()];
    let (raw_base, effort_opt) = match current_model.split_once('[') {
        Some((b, rest)) => (b, rest.strip_suffix(']').unwrap_or(rest)),
        None => (current_model, ""),
    };
    let base = raw_base
        .trim_end_matches("-high")
        .trim_end_matches("-medium")
        .trim_end_matches("-low");

    if !effort_opt.is_empty() {
        args.extend(["--model".into(), base.into(), "--effort".into(), effort_opt.into()]);
    } else if raw_base != base {
        args.extend(["--model".into(), raw_base.into()]);
    } else {
        args.extend(["--model".into(), base.into()]);
    }
    args.extend(["--output-format".into(), "stream-json".into()]);
    args
}

/// The one NDJSON line `agy --input-format stream-json` reads for a turn
/// (shape measured against agy on 17 Oct 2026).
fn stdin_message(prompt: &str) -> String {
    let mut line = json!({ "event": "user", "message": { "content": prompt } }).to_string();
    line.push('\n');
    line
}

fn find_agy_binary() -> std::path::PathBuf {
    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join("agy");
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    if let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) {
        let candidates = [
            home.join(".local/bin/agy"),
            home.join(".cargo/bin/agy"),
            home.join("bin/agy"),
            std::path::PathBuf::from("/opt/homebrew/bin/agy"),
            std::path::PathBuf::from("/usr/local/bin/agy"),
        ];
        for c in candidates {
            if c.is_file() {
                return c;
            }
        }
    }
    std::path::PathBuf::from("agy")
}

fn normalize_model_id_and_label(raw_id: &str, raw_label: &str) -> (String, String) {
    for effort in ["high", "medium", "low", "xhigh", "max"] {
        let suffix = format!("-{effort}");
        if let Some(base) = raw_id.strip_suffix(&suffix) {
            let id = format!("{base}[{effort}]");
            let label = if raw_label.is_empty() {
                format!("{base} ({effort})")
            } else {
                raw_label.to_string()
            };
            return (id, label);
        }
    }
    let label = if raw_label.is_empty() { raw_id.to_string() } else { raw_label.to_string() };
    (raw_id.to_string(), label)
}

fn probe_models() -> Vec<Value> {
    let agy_bin = find_agy_binary();
    if let Ok(output) = Command::new(&agy_bin).arg("models").output() {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            let mut list = Vec::new();
            for line in text.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with('⠋') || trimmed.starts_with('⠙') {
                    continue;
                }
                let mut parts = trimmed.split_whitespace();
                if let Some(raw_id) = parts.next() {
                    let raw_label = parts.collect::<Vec<_>>().join(" ");
                    let (id, name) = normalize_model_id_and_label(raw_id, &raw_label);
                    list.push(json!({
                        "id": id,
                        "name": name,
                        "isCurrent": id == "gemini-3.8-flash[high]" || (list.is_empty() && id.contains("gemini"))
                    }));
                }
            }
            if !list.is_empty() {
                return list;
            }
        }
    }

    // Default fallback if agy models CLI is unreachable
    vec![
        json!({ "id": "gemini-3.8-flash[high]", "name": "Gemini 3.8 Flash (High)", "isCurrent": true }),
        json!({ "id": "gemini-3.8-flash[medium]", "name": "Gemini 3.8 Flash (Medium)", "isCurrent": false }),
        json!({ "id": "gemini-3.8-flash[low]", "name": "Gemini 3.8 Flash (Low)", "isCurrent": false }),
        json!({ "id": "gemini-3.7-flash[high]", "name": "Gemini 3.7 Flash (High)", "isCurrent": false }),
        json!({ "id": "gemini-3.7-flash[medium]", "name": "Gemini 3.7 Flash (Medium)", "isCurrent": false }),
        json!({ "id": "gemini-3.7-flash[low]", "name": "Gemini 3.7 Flash (Low)", "isCurrent": false }),
        json!({ "id": "gemini-3.1-pro[high]", "name": "Gemini 3.1 Pro (High)", "isCurrent": false }),
        json!({ "id": "gemini-3.1-pro[low]", "name": "Gemini 3.1 Pro (Low)", "isCurrent": false }),
        json!({ "id": "claude-opus-5-5[high]", "name": "Claude Opus 5.5 (High)", "isCurrent": false }),
        json!({ "id": "claude-opus-5-5[medium]", "name": "Claude Opus 5.5 (Medium)", "isCurrent": false }),
        json!({ "id": "claude-opus-5-5[low]", "name": "Claude Opus 5.5 (Low)", "isCurrent": false }),
        json!({ "id": "claude-sonnet-5-5[high]", "name": "Claude Sonnet 5.5 (High)", "isCurrent": false }),
        json!({ "id": "claude-sonnet-5-5[medium]", "name": "Claude Sonnet 5.5 (Medium)", "isCurrent": false }),
        json!({ "id": "claude-sonnet-5-5[low]", "name": "Claude Sonnet 5.5 (Low)", "isCurrent": false }),
        json!({ "id": "gpt-oss-120b[medium]", "name": "GPT-OSS 120B (Medium)", "isCurrent": false }),
    ]
}

fn extract_prompt(params: &Value) -> String {
    if let Some(s) = params.get("prompt").and_then(Value::as_str) {
        return s.to_string();
    }
    if let Some(arr) = params.get("prompt").and_then(Value::as_array) {
        let mut text = String::new();
        for block in arr {
            if let Some(t) = block.get("text").and_then(Value::as_str) {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(t);
            }
        }
        if !text.is_empty() {
            return text;
        }
    }
    String::from("Hello")
}

fn handle_agy_event(out: &mut io::StdoutLock, session_id: &str, ev: &Value, sent_any: &mut bool) -> anyhow::Result<()> {
    let event_type = ev.get("event").and_then(Value::as_str).unwrap_or("");
    match event_type {
        "step_update" => {
            if let Some(step) = ev.get("step_update") {
                if let Some(delta) = step.get("text_delta").and_then(Value::as_str) {
                    if !delta.is_empty() {
                        *sent_any = true;
                        send_notification(
                            out,
                            "session/update",
                            json!({
                                "sessionId": session_id,
                                "update": {
                                    "sessionUpdate": "agent_message_chunk",
                                    "content": {
                                        "type": "text",
                                        "text": delta
                                    }
                                }
                            }),
                        )?;
                    }
                }
                if let Some(thought) = step.get("thought").and_then(Value::as_str) {
                    if !thought.is_empty() {
                        send_notification(
                            out,
                            "session/update",
                            json!({
                                "sessionId": session_id,
                                "update": {
                                    "sessionUpdate": "agent_thought_chunk",
                                    "content": {
                                        "type": "text",
                                        "text": thought
                                    }
                                }
                            }),
                        )?;
                    }
                }
            }
        }
        "result" => {
            if let Some(res) = ev.get("result") {
                if !*sent_any {
                    if let Some(resp) = res.get("response").and_then(Value::as_str) {
                        if !resp.is_empty() {
                            *sent_any = true;
                            send_notification(
                                out,
                                "session/update",
                                json!({
                                    "sessionId": session_id,
                                    "update": {
                                        "sessionUpdate": "agent_message_chunk",
                                        "content": {
                                            "type": "text",
                                            "text": resp
                                        }
                                    }
                                }),
                            )?;
                        }
                    }
                }
                if let Some(usage) = res.get("usage") {
                    let in_tok = usage.get("input_tokens").and_then(Value::as_u64).unwrap_or(0);
                    let out_tok = usage.get("output_tokens").and_then(Value::as_u64).unwrap_or(0);
                    send_notification(
                        out,
                        "session/update",
                        json!({
                            "sessionId": session_id,
                            "update": {
                                "sessionUpdate": "usage_update",
                                "inputTokens": in_tok,
                                "outputTokens": out_tok
                            }
                        }),
                    )?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn send_response(
    out: &mut io::StdoutLock,
    id: Option<Value>,
    result: Result<Value, (i32, String)>,
) -> anyhow::Result<()> {
    let Some(id) = id else { return Ok(()); };
    let msg = match result {
        Ok(v) => json!({ "jsonrpc": "2.0", "id": id, "result": v }),
        Err((code, message)) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": code, "message": message }
        }),
    };
    writeln!(out, "{msg}")?;
    out.flush()?;
    Ok(())
}

fn send_notification(out: &mut io::StdoutLock, method: &str, params: Value) -> anyhow::Result<()> {
    let msg = json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params
    });
    writeln!(out, "{msg}")?;
    out.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P3-2: the prompt was passed as `-p <prompt>`, visible to every local
    /// user through `ps`.
    #[test]
    fn prompt_never_appears_in_argv() {
        let secret = "ข้อมูลลับ sk-test-123 context";
        for model in ["gemini-3.8-flash[high]", "gemini-pro-high", "x"] {
            let args = agy_args(model);
            assert!(!args.iter().any(|a| a.contains("sk-test-123")), "{args:?}");
            assert!(!args.iter().any(|a| a == "-p" || a == "--print"), "{args:?}");
            assert!(args.windows(2).any(|w| w == ["--input-format", "stream-json"]));
        }
        let line = stdin_message(secret);
        assert!(line.ends_with('\n') && !line[..line.len() - 1].contains('\n'));
        let v: Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(v["event"], "user");
        assert_eq!(v["message"]["content"], secret);
    }

    #[test]
    fn model_and_effort_are_split_as_before() {
        let a = agy_args("gemini-3.8-flash[high]");
        assert!(a.windows(4).any(|w| w == ["--model", "gemini-3.8-flash", "--effort", "high"]));
    }
}
