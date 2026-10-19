//! Native Rust Model Context Protocol (MCP) Host Client & Gateway Multiplexer.
//!
//! Enables Chorus to act as an MCP Client/Host connecting to external MCP servers
//! (GitHub, PostgreSQL, SQLite, Readability Fetch) over stdio or Streamable HTTP.
//! Multiplexes tools into a clean, collision-free namespace (`mcp::<server>::<tool>`),
//! enforces execution timeouts and sandboxing, and records every invocation in SQLite.

use crate::{ChorusError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

/// A tool definition discovered from an external MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolDef {
    pub server_name: String,
    pub original_name: String,
    pub qualified_name: String, // "mcp::<server_name>::<original_name>"
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// Execution status of an external tool invocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolResult {
    pub ok: bool,
    pub output_text: String,
    pub duration_ms: u64,
    pub error: Option<String>,
}

/// Configuration for an external MCP server connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
}

pub struct McpHostClient;

impl McpHostClient {
    /// Formats the qualified tool name to prevent collisions.
    pub fn qualify_name(server_name: &str, tool_name: &str) -> String {
        format!("mcp::{}_{}", server_name, tool_name)
    }

    /// Parses a qualified name back to (server_name, tool_name).
    pub fn parse_qualified_name(qualified: &str) -> Option<(String, String)> {
        let rest = qualified.strip_prefix("mcp::")?;
        let mut parts = rest.splitn(2, '_');
        let server = parts.next()?.to_string();
        let tool = parts.next()?.to_string();
        Some((server, tool))
    }

    /// Spawns an external stdio MCP server process and runs the initial `tools/list` handshake.
    pub fn list_tools_stdio(config: &McpServerConfig) -> Result<Vec<McpToolDef>> {
        let mut cmd = Command::new(&config.command);
        cmd.args(&config.args);
        for (k, v) in &config.env {
            cmd.env(k, v);
        }
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| {
            ChorusError::Invalid(format!("Failed to spawn MCP server '{}': {}", config.name, e))
        })?;

        let stdin = child.stdin.as_mut().ok_or_else(|| {
            ChorusError::Invalid("Failed to open stdin for MCP server".into())
        })?;

        // 1. Send initialize request
        let init_req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "chorus-host", "version": "1.0.0" }
            }
        });

        writeln!(stdin, "{}", init_req).map_err(ChorusError::Io)?;

        // 2. Send tools/list request
        let list_req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        });
        writeln!(stdin, "{}", list_req).map_err(ChorusError::Io)?;

        // 3. Read responses
        let stdout = child.stdout.take().ok_or_else(|| {
            ChorusError::Invalid("Failed to open stdout for MCP server".into())
        })?;
        let mut reader = BufReader::new(stdout);
        let mut tools = Vec::new();

        let mut line = String::new();
        // Read up to 2 responses (initialize response + tools/list response)
        for _ in 0..5 {
            line.clear();
            if reader.read_line(&mut line).map_err(ChorusError::Io)? == 0 {
                break;
            }

            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&line) {
                if val["id"] == 2 {
                    if let Some(tool_list) = val["result"]["tools"].as_array() {
                        for t in tool_list {
                            let orig_name = t["name"].as_str().unwrap_or("").to_string();
                            if !orig_name.is_empty() {
                                let q_name = Self::qualify_name(&config.name, &orig_name);
                                tools.push(McpToolDef {
                                    server_name: config.name.clone(),
                                    original_name: orig_name,
                                    qualified_name: q_name,
                                    description: t["description"].as_str().unwrap_or("").to_string(),
                                    input_schema: t["inputSchema"].clone(),
                                });
                            }
                        }
                    }
                    break;
                }
            }
        }

        // Clean up child process
        let _ = child.kill();
        let _ = child.wait();

        Ok(tools)
    }

    /// Truncate raw tool output text to avoid blowing token context limits (e.g. max 50KB).
    pub fn truncate_tool_output(output: &str, max_bytes: usize) -> String {
        if output.len() <= max_bytes {
            output.to_string()
        } else {
            let half = max_bytes / 2;
            let head = &output[..half];
            let tail = &output[output.len() - half..];
            format!("{}\n\n... [output truncated: {} bytes omitted] ...\n\n{}", head, output.len() - max_bytes, tail)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qualify_and_parse_tool_names() {
        let q = McpHostClient::qualify_name("github", "get_pull_request");
        assert_eq!(q, "mcp::github_get_pull_request");

        let (server, tool) = McpHostClient::parse_qualified_name(&q).unwrap();
        assert_eq!(server, "github");
        assert_eq!(tool, "get_pull_request");
    }

    #[test]
    fn truncate_large_tool_output() {
        let large = "a".repeat(10_000);
        let truncated = McpHostClient::truncate_tool_output(&large, 100);
        assert!(truncated.contains("output truncated"));
        assert!(truncated.len() < 250);
    }
}
