//! `antigravity-acp` — Lightweight Agent Client Protocol (ACP) bridge for Antigravity (`agy`).
//!
//! Spoken over stdin/stdout as newline-delimited JSON-RPC 2.0. Translates ACP session
//! lifecycle and prompt turns into Antigravity CLI invocations with `--output-format stream-json`.

use serde_json::{json, Value};
use std::io::{self, BufRead, BufReader, Write};
use chorus_acp::agy::{canonical_model_id, context_window, split_model};
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
                if let Some(cur) = models
                    .iter()
                    .find(|m| m["isCurrent"].as_bool() == Some(true))
                    .and_then(|m| m["id"].as_str())
                {
                    current_model = cur.to_string();
                }
                let res = json!({
                    "sessionId": session_id,
                    "models": {
                        "availableModels": models,
                        "currentModelId": current_model
                    }
                });
                send_response(&mut out, id, Ok(res))?;
            }
            "session/set_model" => {
                if let Some(m) = params.get("modelId").and_then(Value::as_str) {
                    current_model = canonical_model_id(m);
                }
                send_response(&mut out, id, Ok(json!({ "models": { "currentModelId": current_model } })))?;
            }
            "session/set_config_option" => {
                if let Some(cfg) = params.get("configId").and_then(Value::as_str) {
                    if cfg == "model" {
                        if let Some(val) = params.get("value").and_then(Value::as_str) {
                            current_model = canonical_model_id(val);
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
                        let mut turn = Turn::new(&current_model);
                        if let Some(stdout_stream) = child.stdout.take() {
                            let reader = BufReader::new(stdout_stream);
                            for stream_line in reader.lines().map_while(Result::ok) {
                                match serde_json::from_str::<Value>(stream_line.trim()) {
                                    Ok(event_json) => {
                                        for update in turn.on_event(&event_json) {
                                            send_notification(
                                                &mut out,
                                                "session/update",
                                                json!({ "sessionId": session_id, "update": update }),
                                            )?;
                                        }
                                    }
                                    // agy prints a bad `--model` and similar
                                    // refusals as plain text on stdout.
                                    Err(_) => turn.note_plain(&stream_line),
                                }
                            }
                        }
                        let sent_any = turn.sent_any;
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
                            if let Some(failure) = turn.failure(&err_str) {
                                // An error reply, not a chat message: the
                                // relay classifies it (quota → failover).
                                send_response(&mut out, id, Err((-32603, failure)))?;
                                continue;
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
/// Model and effort go separately (`--model gemini-4-argon --effort high`):
/// measured 18 Oct 2026, agy reports the bare id back in `init.model` and
/// the thinking-token count follows `--effort`.
fn agy_args(current_model: &str) -> Vec<String> {
    let mut args: Vec<String> = vec!["--input-format".into(), "stream-json".into()];
    let (base, effort) = split_model(current_model);
    if !base.is_empty() {
        args.extend(["--model".into(), base.into()]);
    }
    if let Some(e) = effort {
        args.extend(["--effort".into(), e.into()]);
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
    let id = canonical_model_id(raw_id);
    let label = if !raw_label.is_empty() {
        raw_label.to_string()
    } else {
        match split_model(raw_id) {
            (base, Some(e)) => format!("{base} ({e})"),
            (base, None) => base.to_string(),
        }
    };
    (id, label)
}

fn probe_models() -> Vec<Value> {
    let agy_bin = find_agy_binary();
    if let Ok(output) = Command::new(&agy_bin).arg("models").output() {
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            let mut list = Vec::new();
            for line in text.lines() {
                let trimmed = line.trim();
                // Rows are `id<TAB>label`; anything else is a spinner or
                // "Fetching available models...".
                if trimmed.is_empty() || !line.contains('\t') {
                    continue;
                }
                let mut parts = trimmed.split_whitespace();
                if let Some(raw_id) = parts.next() {
                    let raw_label = parts.collect::<Vec<_>>().join(" ");
                    let (id, name) = normalize_model_id_and_label(raw_id, &raw_label);
                    if list.iter().any(|m: &Value| m["id"] == id.as_str()) {
                        continue;
                    }
                    list.push(json!({ "id": id, "name": name, "isCurrent": false }));
                }
            }
            if !list.is_empty() {
                // agy names no default; keep the old one if it is offered,
                // else the first Gemini, else the first row.
                let pick = list
                    .iter()
                    .position(|m| m["id"] == "gemini-3.8-flash[high]")
                    .or_else(|| list.iter().position(|m| m["id"].as_str().is_some_and(|i| i.starts_with("gemini"))))
                    .unwrap_or(0);
                list[pick]["isCurrent"] = json!(true);
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

/// One `agy` print-mode turn, folded into ACP `session/update`s.
struct Turn {
    /// Context window of the model this turn runs on, when known.
    window: Option<u64>,
    sent_any: bool,
    /// `result.status` other than SUCCESS, with whatever agy said about it.
    failed: Option<String>,
    plain: Vec<String>,
}

impl Turn {
    fn new(model: &str) -> Self {
        Self {
            window: context_window(model),
            sent_any: false,
            failed: None,
            plain: Vec::new(),
        }
    }

    fn note_plain(&mut self, line: &str) {
        let t = line.trim();
        if !t.is_empty() && self.plain.len() < 40 {
            self.plain.push(t.to_string());
        }
    }

    /// The error to answer the prompt with, if the turn failed and nothing
    /// reached the user.
    fn failure(&self, stderr: &str) -> Option<String> {
        if self.sent_any {
            return None;
        }
        let status = self.failed.as_ref()?;
        let mut detail: Vec<&str> = self.plain.iter().map(String::as_str).collect();
        if !stderr.trim().is_empty() {
            detail.push(stderr.trim());
        }
        Some(if detail.is_empty() {
            format!("Antigravity CLI error: turn ended with status {status}")
        } else {
            format!("Antigravity CLI error: {}", detail.join("\n"))
        })
    }

    fn on_event(&mut self, ev: &Value) -> Vec<Value> {
        let mut out = Vec::new();
        match ev.get("event").and_then(Value::as_str).unwrap_or("") {
            "step_update" => {
                let Some(step) = ev.get("step_update") else { return out };
                if let Some(delta) = step.get("text_delta").and_then(Value::as_str) {
                    if !delta.is_empty() {
                        self.sent_any = true;
                        out.push(chunk("agent_message_chunk", delta));
                    }
                }
                if let Some(thought) = step.get("thought").and_then(Value::as_str) {
                    if !thought.is_empty() {
                        out.push(chunk("agent_thought_chunk", thought));
                    }
                }
                // A finished step carries the usage so far, so the meter
                // moves mid-turn instead of only at the end.
                if let Some(u) = step.get("usage").and_then(|u| self.usage(u)) {
                    out.push(u);
                }
            }
            "result" => {
                let Some(res) = ev.get("result") else { return out };
                if !self.sent_any {
                    if let Some(resp) = res.get("response").and_then(Value::as_str) {
                        if !resp.is_empty() {
                            self.sent_any = true;
                            out.push(chunk("agent_message_chunk", resp));
                        }
                    }
                }
                if let Some(status) = res.get("status").and_then(Value::as_str) {
                    if status != "SUCCESS" {
                        self.failed = Some(status.to_string());
                    }
                }
                if let Some(u) = res.get("usage").and_then(|u| self.usage(u)) {
                    out.push(u);
                }
            }
            _ => {}
        }
        out
    }

    /// ACP `usage_update` from agy's token counts. ACP reports context
    /// pressure (`used` of `size`), not an in/out split: what the next turn
    /// starts from is this turn's input plus what the model wrote. agy has
    /// no window field, so `size` is sent only when known.
    fn usage(&self, u: &Value) -> Option<Value> {
        let input = u.get("input_tokens").and_then(Value::as_u64).unwrap_or(0);
        let output = u.get("output_tokens").and_then(Value::as_u64).unwrap_or(0);
        if input == 0 && output == 0 {
            return None;
        }
        let mut v = json!({ "sessionUpdate": "usage_update", "used": input + output });
        if let Some(w) = self.window {
            v["size"] = json!(w);
        }
        Some(v)
    }
}

fn chunk(kind: &str, text: &str) -> Value {
    json!({ "sessionUpdate": kind, "content": { "type": "text", "text": text } })
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

    /// The installed bridge cached `agy models`' own ids (`-high` suffix),
    /// which the picker could not split, and passed them through whole.
    #[test]
    fn agy_list_ids_become_model_and_effort() {
        let a = agy_args("gemini-4-argon-medium");
        assert!(a.windows(4).any(|w| w == ["--model", "gemini-4-argon", "--effort", "medium"]), "{a:?}");
        let a = agy_args("claude-opus-5-5[xhigh]");
        assert!(a.windows(4).any(|w| w == ["--model", "claude-opus-5-5", "--effort", "xhigh"]), "{a:?}");
        let a = agy_args("gemini-4-argon");
        assert!(a.windows(2).any(|w| w == ["--model", "gemini-4-argon"]));
        assert!(!a.iter().any(|x| x == "--effort"));
        assert_eq!(canonical_model_id("gpt-oss-120b-medium"), "gpt-oss-120b[medium]");
        assert_eq!(canonical_model_id("gemini-4-argon[high]"), "gemini-4-argon[high]");
        assert_eq!(canonical_model_id("claude-opus-5-5"), "claude-opus-5-5");
        let (id, label) = normalize_model_id_and_label("gemini-3.8-flash-low", "Gemini 3.8 Flash (Low)");
        assert_eq!((id.as_str(), label.as_str()), ("gemini-3.8-flash[low]", "Gemini 3.8 Flash (Low)"));
    }

    /// Real agy 1.4.2 stream (18 Oct 2026, trimmed): usage arrives on the
    /// finished step and again on `result`, as `input_tokens`/`output_tokens`.
    /// The bridge used to forward those names, which Chorus does not read,
    /// so the context meter stayed at 0%.
    #[test]
    fn agy_usage_becomes_used_and_size() {
        std::env::remove_var("CHORUS_AGY_CONTEXT_WINDOW");
        let mut t = Turn::new("gemini-3.8-flash[low]");
        let lines = [
            r#"{"event":"step_update","step_update":{"step_index":1,"state":"ACTIVE","step_type":"agent_response","text_delta":"ok"}}"#,
            r#"{"event":"step_update","step_update":{"step_index":1,"state":"DONE","step_type":"agent_response","text_delta":"\n","usage":{"input_tokens":15206,"output_tokens":1,"thinking_tokens":0,"cache_read_tokens":0,"total_tokens":15207}}}"#,
            r#"{"event":"result","result":{"status":"SUCCESS","response":"ok\n","usage":{"input_tokens":15206,"output_tokens":1,"thinking_tokens":0,"cache_read_tokens":0,"total_tokens":15207}}}"#,
        ];
        let updates: Vec<Value> = lines
            .iter()
            .flat_map(|l| t.on_event(&serde_json::from_str(l).unwrap()))
            .collect();
        let usage: Vec<&Value> = updates.iter().filter(|u| u["sessionUpdate"] == "usage_update").collect();
        assert_eq!(usage.len(), 2);
        assert_eq!(usage[1]["used"], 15207);
        assert_eq!(usage[1]["size"], 1_048_576);
        assert!(usage[1].get("inputTokens").is_none());
        assert!(t.failure("").is_none());

        // No known window: `used` alone, never a made-up size.
        let mut t = Turn::new("claude-sonnet-5-5[low]");
        let u = t.on_event(&serde_json::from_str(lines[2]).unwrap());
        let u = u.iter().find(|u| u["sessionUpdate"] == "usage_update").unwrap();
        assert_eq!(u["used"], 15207);
        assert!(u.get("size").is_none());
    }

    #[test]
    fn a_refused_model_is_an_error_not_silence() {
        let mut t = Turn::new("nope[high]");
        t.note_plain(r#"error: invalid model selection (--model "nope" --effort "high"): model nope is not recognized"#);
        t.on_event(&serde_json::from_str(r#"{"event":"result","result":{"status":"ERROR","usage":{"input_tokens":0,"output_tokens":0}}}"#).unwrap());
        let e = t.failure("").unwrap();
        assert!(e.contains("invalid model selection"), "{e}");
    }
}
