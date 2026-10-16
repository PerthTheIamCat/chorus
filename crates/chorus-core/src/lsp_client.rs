//! Headless Language Server Protocol (LSP) Client Subsystem.
//!
//! Connects directly to language servers (`rust-analyzer`, `vtsls`, `sourcekit-lsp`)
//! via JSON-RPC 2.0 over stdio pipes, extracting compiler-verified diagnostics,
//! definitions, and hover types without reading thousands of raw lines.

use crate::{ChorusError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// An LSP Diagnostic message received via publishDiagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LspDiagnostic {
    pub file_path: String,
    pub line_start: u32,
    pub col_start: u32,
    pub line_end: u32,
    pub col_end: u32,
    pub severity: u32, // 1=Error, 2=Warning, 3=Info, 4=Hint
    pub source: String,
    pub message: String,
}

/// JSON-RPC 2.0 Request envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    pub params: serde_json::Value,
}

impl JsonRpcRequest {
    pub fn new(id: u64, method: &str, params: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            method: method.to_string(),
            params,
        }
    }
}

/// Language Server Registry & Diagnostic Cache.
#[derive(Debug, Default)]
pub struct LspCache {
    /// file_path -> list of active diagnostics
    diagnostics_by_file: HashMap<String, Vec<LspDiagnostic>>,
}

impl LspCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update_diagnostics(&mut self, file_path: &str, items: Vec<LspDiagnostic>) {
        self.diagnostics_by_file.insert(file_path.to_string(), items);
    }

    pub fn get_diagnostics(&self, file_path: &str) -> &[LspDiagnostic] {
        self.diagnostics_by_file
            .get(file_path)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub fn all_errors(&self) -> Vec<&LspDiagnostic> {
        self.diagnostics_by_file
            .values()
            .flat_map(|list| list.iter().filter(|d| d.severity == 1))
            .collect()
    }
}

pub struct HeadlessLsp;

impl HeadlessLsp {
    /// Build an LSP `textDocument/definition` request payload.
    pub fn make_definition_request(id: u64, uri: &str, line: u32, character: u32) -> JsonRpcRequest {
        JsonRpcRequest::new(
            id,
            "textDocument/definition",
            serde_json::json!({
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character }
            }),
        )
    }

    /// Build an LSP `textDocument/hover` request payload.
    pub fn make_hover_request(id: u64, uri: &str, line: u32, character: u32) -> JsonRpcRequest {
        JsonRpcRequest::new(
            id,
            "textDocument/hover",
            serde_json::json!({
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character }
            }),
        )
    }

    /// Parse an incoming `textDocument/publishDiagnostics` notification.
    pub fn parse_publish_diagnostics(val: &serde_json::Value) -> Result<(String, Vec<LspDiagnostic>)> {
        let uri = val["uri"].as_str().ok_or_else(|| ChorusError::Invalid("Missing uri".into()))?;
        let items = val["diagnostics"].as_array().ok_or_else(|| ChorusError::Invalid("Missing diagnostics".into()))?;

        let mut diags = Vec::new();
        for item in items {
            let range = &item["range"];
            let start = &range["start"];
            let end = &range["end"];

            diags.push(LspDiagnostic {
                file_path: uri.to_string(),
                line_start: start["line"].as_u64().unwrap_or(0) as u32,
                col_start: start["character"].as_u64().unwrap_or(0) as u32,
                line_end: end["line"].as_u64().unwrap_or(0) as u32,
                col_end: end["character"].as_u64().unwrap_or(0) as u32,
                severity: item["severity"].as_u64().unwrap_or(1) as u32,
                source: item["source"].as_str().unwrap_or("lsp").to_string(),
                message: item["message"].as_str().unwrap_or("").to_string(),
            });
        }

        Ok((uri.to_string(), diags))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lsp_json_rpc_requests_format_correctly() {
        let req = HeadlessLsp::make_definition_request(42, "file:///src/main.rs", 10, 5);
        assert_eq!(req.id, 42);
        assert_eq!(req.method, "textDocument/definition");
        assert_eq!(req.params["textDocument"]["uri"], "file:///src/main.rs");
    }

    #[test]
    fn parse_publish_diagnostics_roundtrip() {
        let json = serde_json::json!({
            "uri": "file:///src/lib.rs",
            "diagnostics": [
                {
                    "range": {
                        "start": { "line": 5, "character": 4 },
                        "end": { "line": 5, "character": 12 }
                    },
                    "severity": 1,
                    "source": "rustc",
                    "message": "mismatched types"
                }
            ]
        });

        let (uri, diags) = HeadlessLsp::parse_publish_diagnostics(&json).unwrap();
        assert_eq!(uri, "file:///src/lib.rs");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].message, "mismatched types");
        assert_eq!(diags[0].severity, 1);
    }
}
