//! Release & Update Verification Engine for Chorus.
//!
//! Checks for newer releases, parses Semantic Versioning, compares versions,
//! and provides release notes and download links for the desktop harness.

use crate::{ChorusError, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateCheckResult {
    pub current_version: String,
    pub latest_version: String,
    pub has_update: bool,
    pub release_notes: Option<String>,
    pub download_url: Option<String>,
    pub published_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SemVer {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl SemVer {
    pub fn parse(s: &str) -> Option<Self> {
        let clean = s.trim().trim_start_matches('v').trim_start_matches('V');
        let parts: Vec<&str> = clean.split('.').collect();
        if parts.is_empty() {
            return None;
        }

        let parse_part = |p: &str| -> u64 {
            p.chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse::<u64>()
                .unwrap_or(0)
        };

        let major = parts.first().map(|p| parse_part(p)).unwrap_or(0);
        let minor = parts.get(1).map(|p| parse_part(p)).unwrap_or(0);
        let patch = parts.get(2).map(|p| parse_part(p)).unwrap_or(0);

        Some(Self {
            major,
            minor,
            patch,
        })
    }
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    body: Option<String>,
    published_at: Option<String>,
    html_url: Option<String>,
}

/// Parse a GitHub release response JSON and compare against the current app version.
pub fn evaluate_update(current_version: &str, json_response: &str) -> Result<UpdateCheckResult> {
    let release: GitHubRelease = serde_json::from_str(json_response).map_err(|e| {
        ChorusError::Invalid(format!("failed to parse GitHub release response: {e}"))
    })?;

    let cur = SemVer::parse(current_version).unwrap_or(SemVer {
        major: 0,
        minor: 1,
        patch: 0,
    });
    let latest = SemVer::parse(&release.tag_name).unwrap_or(cur);

    let has_update = latest > cur;

    Ok(UpdateCheckResult {
        current_version: current_version.to_string(),
        latest_version: release.tag_name,
        has_update,
        release_notes: release.body,
        download_url: release.html_url,
        published_at: release.published_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_parsing_and_comparison() {
        let v1 = SemVer::parse("0.1.0").unwrap();
        let v2 = SemVer::parse("v0.2.0").unwrap();
        let v3 = SemVer::parse("0.2.1-beta").unwrap();
        let v4 = SemVer::parse("v1.0.0").unwrap();

        assert!(v2 > v1);
        assert!(v3 > v2);
        assert!(v4 > v3);
        assert_eq!(v1, SemVer { major: 0, minor: 1, patch: 0 });
    }

    #[test]
    fn evaluate_update_detects_newer_version() {
        let json = r#"{
            "tag_name": "v0.2.0",
            "body": "- Added Multi-Agent Swarm\n- Added Metal MLX runtime",
            "published_at": "2026-10-10T12:00:00Z",
            "html_url": "https://github.com/chorus-ai/chorus/releases/tag/v0.2.0"
        }"#;

        let result = evaluate_update("0.1.0", json).unwrap();
        assert!(result.has_update);
        assert_eq!(result.latest_version, "v0.2.0");
        assert!(result.release_notes.unwrap().contains("Multi-Agent Swarm"));
        assert!(result.download_url.unwrap().contains("releases/tag/v0.2.0"));
    }

    #[test]
    fn evaluate_update_identifies_up_to_date() {
        let json = r#"{
            "tag_name": "v0.1.0",
            "body": "Initial release",
            "published_at": "2026-10-09T10:00:00Z",
            "html_url": "https://github.com/chorus-ai/chorus/releases/tag/v0.1.0"
        }"#;

        let result = evaluate_update("0.1.0", json).unwrap();
        assert!(!result.has_update);
    }
}
