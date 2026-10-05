//! Curated, user-level integration installs and per-project enablement.
//!
//! This module deliberately owns only Chorus's data and launch paths. It never
//! runs a vendor `init` command or writes another tool's global configuration.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use thiserror::Error;

pub const STATE_FILE: &str = ".chorus/integrations.json";
pub const TOOLS_DIR: &str = ".chorus/tools";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Mcp,
    Cli,
    Skill,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallMethod {
    Homebrew,
    Npm,
    Uv,
    CargoGit,
    Bundled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Recipe {
    pub method: InstallMethod,
    pub package: &'static str,
    pub health_args: &'static [&'static str],
    pub executable: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Integration {
    pub id: &'static str,
    pub title: &'static str,
    pub source: &'static str,
    pub maintainer: &'static str,
    pub license: &'static str,
    pub reviewed_version: &'static str,
    pub modes: &'static [Mode],
    pub required_runtimes: &'static [&'static str],
    pub actions: &'static [&'static str],
    pub project_access: &'static [&'static str],
    pub network: &'static str,
    pub secret_names: &'static [&'static str],
    pub recipe: Recipe,
}

const GRAFT_MODES: &[Mode] = &[Mode::Mcp, Mode::Cli];
const MCP_ONLY: &[Mode] = &[Mode::Mcp];
const CLI_ONLY: &[Mode] = &[Mode::Cli];
const SKILL_ONLY: &[Mode] = &[Mode::Skill];

const CHORUS_REVIEW_SKILL: &str = r#"---
name: chorus-review
description: Review code changes when asked to review a diff, patch, or implementation. Inspect the changed code and its nearby call sites; report only actionable correctness, security, or regression risks, ordered by severity, with file and line references.
---

# Review code changes

1. Read the changed code and enough surrounding implementation to understand its contract.
2. Trace affected callers and data flows; check error paths, compatibility, and boundary conditions.
3. Report concrete findings first, ordered by severity. Include a file and line for each finding.
4. Keep the review focused. If no actionable issue is found, say so and mention what remains unverified.
"#;

/// Versions pinned from first-party release/package pages reviewed 2026-10-05.
/// Package managers verify package integrity while resolving their lock data.
pub static CATALOG: &[Integration] = &[
    Integration { id:"graft", title:"Graft", source:"https://github.com/trailhq/Graft", maintainer:"TrailHQ", license:"MIT", reviewed_version:"0.18.0", modes:GRAFT_MODES, required_runtimes:&["Node.js", "npm"], actions:&["Read project source to build a code graph", "Expose graft ask/grep and MCP graph tools"], project_access:&["Read project files", "Write regenerable graft/ cache when CLI is used"], network:"May contact the configured model provider for deep summaries; structural graph remains local", secret_names:&["GRAFT_API_KEY", "OPENROUTER_API_KEY"], recipe:Recipe{method:InstallMethod::Npm,package:"@nanonets/graft",health_args:&["--version"],executable:"graft"} },
    Integration { id:"serena", title:"Serena", source:"https://github.com/oraios/serena", maintainer:"Serena contributors", license:"GPL-3.0-or-later", reviewed_version:"1.7.0", modes:MCP_ONLY, required_runtimes:&["uv", "Python 3.13"], actions:&["Semantic source retrieval and editing over MCP"], project_access:&["Read and edit project files when an agent invokes tools"], network:"Local MCP process; language-server behavior depends on configured backend", secret_names:&[], recipe:Recipe{method:InstallMethod::Uv,package:"serena-agent",health_args:&["--version"],executable:"serena"} },
    Integration { id:"rtk", title:"RTK", source:"https://github.com/rtk-ai/rtk", maintainer:"RTK contributors", license:"Apache-2.0", reviewed_version:"0.51.0", modes:CLI_ONLY, required_runtimes:&["Homebrew"], actions:&["Run explicitly as rtk <command> to filter command output"], project_access:&["Commands invoked by the agent retain their normal filesystem access"], network:"No network needed for local command filtering", secret_names:&[], recipe:Recipe{method:InstallMethod::Homebrew,package:"rtk-ai/tap/rtk",health_args:&["--version"],executable:"rtk"} },
    Integration { id:"headroom", title:"Headroom", source:"https://github.com/headroomlabs-ai/headroom", maintainer:"Headroom Labs", license:"Apache-2.0", reviewed_version:"0.39.1", modes:GRAFT_MODES, required_runtimes:&["uv", "Python 3.11+"], actions:&["MCP compression, retrieval and stats tools", "Explicit CLI commands where supported"], project_access:&["Receives content the agent explicitly submits to its tools"], network:"MCP mode is local; remote proxy and request-wrapper modes are excluded", secret_names:&[], recipe:Recipe{method:InstallMethod::Uv,package:"headroom-ai",health_args:&["--version"],executable:"headroom"} },
    Integration { id:"chorus-review", title:"Chorus Review", source:"https://github.com/PerthTheIamCat/chorus", maintainer:"Chorus", license:"MIT", reviewed_version:"1.0.0", modes:SKILL_ONLY, required_runtimes:&[], actions:&["Review code changes for actionable correctness and regression risks"], project_access:&["Reads changed code and nearby call sites when invoked"], network:"No network access is built into this skill", secret_names:&[], recipe:Recipe{method:InstallMethod::Bundled,package:"chorus-review",health_args:&[],executable:"SKILL.md"} },
];

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CatalogState {
    pub schema_version: u32,
    #[serde(default)]
    pub installed: BTreeMap<String, InstallRecord>,
    /// Canonical project path -> catalog IDs. This file lives in the user's home,
    /// so a checked-in workspace cannot grant itself integration access.
    #[serde(default)]
    pub projects: BTreeMap<String, BTreeSet<String>>,
    #[serde(default)]
    pub history: Vec<UpdateRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallRecord {
    pub id: String,
    pub version: String,
    pub source: String,
    pub license: String,
    pub method: InstallMethod,
    pub runtime_path: PathBuf,
    /// Registry-provided integrity identifier for the installed top-level
    /// artifact, when exposed by the package manager.
    pub artifact_integrity: Option<String>,
    pub owned_files: Vec<PathBuf>,
    pub health: Health,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Health {
    pub healthy: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateRecord {
    pub id: String,
    pub from: Option<String>,
    pub to: String,
    pub outcome: String,
}

#[derive(Debug, Error)]
pub enum CatalogError {
    #[error("unknown integration: {0}")]
    Unknown(String),
    #[error("no home directory is available")]
    NoHome,
    #[error("required installer is unavailable: {0}")]
    MissingRuntime(String),
    #[error("integration install failed: {0}")]
    Install(String),
    #[error("integration health check failed: {0}")]
    Health(String),
    #[error("catalog state error: {0}")]
    Io(#[from] std::io::Error),
    #[error("catalog state is invalid: {0}")]
    Json(#[from] serde_json::Error),
}

pub fn catalog() -> &'static [Integration] {
    CATALOG
}
pub fn get(id: &str) -> Option<&'static Integration> {
    CATALOG.iter().find(|x| x.id == id)
}

pub fn state_path() -> Result<PathBuf, CatalogError> {
    Ok(home()?.join(STATE_FILE))
}
pub fn tools_path() -> Result<PathBuf, CatalogError> {
    Ok(home()?.join(TOOLS_DIR))
}

fn home() -> Result<PathBuf, CatalogError> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or(CatalogError::NoHome)
}

pub fn load_state() -> Result<CatalogState, CatalogError> {
    let path = state_path()?;
    match std::fs::read(path) {
        Ok(data) => Ok(serde_json::from_slice(&data)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(CatalogState {
            schema_version: 1,
            ..Default::default()
        }),
        Err(e) => Err(e.into()),
    }
}

pub fn save_state(state: &CatalogState) -> Result<(), CatalogError> {
    let path = state_path()?;
    let dir = path.parent().ok_or(CatalogError::NoHome)?;
    std::fs::create_dir_all(dir)?;
    let temp = dir.join("integrations.json.tmp");
    std::fs::write(&temp, serde_json::to_vec_pretty(state)?)?;
    std::fs::rename(temp, path)?;
    Ok(())
}

fn runner(method: InstallMethod) -> &'static str {
    match method {
        InstallMethod::Homebrew => "brew",
        InstallMethod::Npm => "npm",
        InstallMethod::Uv => "uv",
        InstallMethod::CargoGit => "cargo",
        InstallMethod::Bundled => "",
    }
}

const INSTALLERS: &[&str] = &["brew", "uv", "npm", "cargo", "git"];

/// Report the missing package installer needed by a catalog entry, if any.
/// The same resolver is used by installation preflight and child processes.
pub fn missing_installer(entry: &Integration) -> Option<&'static str> {
    if entry.recipe.method != InstallMethod::Homebrew && entry.recipe.method != InstallMethod::Bundled {
        return Some("Homebrew formula");
    }
    let name = runner(entry.recipe.method);
    (!name.is_empty() && installer_path(name).is_none()).then_some(name)
}

fn search_path() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut seen = BTreeSet::new();
    if let Some(current) = std::env::var_os("PATH") {
        for path in std::env::split_paths(&current) {
            // Empty/relative entries would make installer resolution depend on
            // the process working directory. Keep absolute PATH order intact.
            if path.is_absolute() && seen.insert(path.clone()) {
                paths.push(path);
            }
        }
    }
    let mut standard = vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ];
    if let Ok(home) = home() {
        standard.push(home.join(".cargo/bin"));
        standard.push(home.join(".local/bin"));
    }
    for path in standard {
        if seen.insert(path.clone()) {
            paths.push(path);
        }
    }
    paths
}

fn child_path() -> OsString {
    std::env::join_paths(search_path())
        .unwrap_or_else(|_| std::env::var_os("PATH").unwrap_or_default())
}

fn installer_path(name: &str) -> Option<PathBuf> {
    if !INSTALLERS.contains(&name) {
        return None;
    }
    search_path()
        .into_iter()
        .map(|directory| directory.join(name))
        .find(|candidate| is_executable_file(candidate))
}

fn is_executable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn installer_command(name: &str) -> Result<Command, CatalogError> {
    let executable = installer_path(name)
        .ok_or_else(|| CatalogError::MissingRuntime(name.to_owned()))?;
    let mut command = Command::new(executable);
    command.env("PATH", child_path());
    Ok(command)
}

/// Read Homebrew's installed keg rather than inferring installation from PATH
/// or Chorus's own state. The opt path follows the active keg across upgrades.
pub fn homebrew_installed(entry: &Integration) -> Result<Option<InstallRecord>, CatalogError> {
    if entry.recipe.method != InstallMethod::Homebrew {
        return Ok(None);
    }
    let output = installer_command("brew")?
        .args(["list", "--versions", "--formula", entry.recipe.package])
        .output()?;
    if !output.status.success() || output.stdout.is_empty() {
        return Ok(None);
    }
    let version = String::from_utf8_lossy(&output.stdout)
        .split_whitespace().nth(1).map(str::to_owned)
        .ok_or_else(|| CatalogError::Install("Homebrew returned no installed version".into()))?;
    let prefix = installer_command("brew")?
        .args(["--prefix", entry.recipe.package]).output()?;
    if !prefix.status.success() {
        return Err(CatalogError::Install("Homebrew could not resolve the installed prefix".into()));
    }
    let executable = PathBuf::from(String::from_utf8_lossy(&prefix.stdout).trim())
        .join("bin").join(entry.recipe.executable);
    let health = match Command::new(&executable).args(entry.recipe.health_args).output() {
        Ok(output) if output.status.success() => Health {
            healthy: true,
            detail: String::from_utf8_lossy(&output.stdout).trim().to_owned(),
        },
        Ok(output) => Health { healthy: false, detail: format!("version check exited with {}", output.status) },
        Err(error) => Health { healthy: false, detail: error.to_string() },
    };
    Ok(Some(InstallRecord {
        id: entry.id.into(), version, source: entry.source.into(), license: entry.license.into(),
        method: InstallMethod::Homebrew, runtime_path: executable,
        artifact_integrity: None, owned_files: vec![], health,
    }))
}

fn install_uv_locked(entry: &Integration, root: &Path) -> Result<(PathBuf, String), CatalogError> {
    let python = installer_command("uv")?
        .args(["--no-config", "python", "find", "3.13"])
        .env_remove("UV_INDEX_URL")
        .env_remove("UV_EXTRA_INDEX_URL")
        .env_remove("UV_DEFAULT_INDEX")
        .output()?;
    if !python.status.success() {
        return Err(CatalogError::MissingRuntime(
            "Python 3.13; recovery: install it with `uv python install 3.13`".into(),
        ));
    }
    let input = root.join("requirements.in");
    let lock = root.join("requirements.lock");
    let venv = root.join("venv");
    std::fs::write(
        &input,
        format!("{}=={}\n", entry.recipe.package, entry.reviewed_version),
    )?;
    let compiled = installer_command("uv")?
        .args([
            "--no-config",
            "pip",
            "compile",
            "--generate-hashes",
            "--only-binary",
            ":all:",
            "--default-index",
            "https://pypi.org/simple",
            "--output-file",
        ])
        .arg(&lock)
        .arg(&input)
        .env_remove("UV_INDEX_URL")
        .env_remove("UV_EXTRA_INDEX_URL")
        .env_remove("UV_DEFAULT_INDEX")
        .output()?;
    if !compiled.status.success() {
        return Err(CatalogError::Install(format!(
            "uv could not create the pinned PyPI hash lock (exit {})",
            compiled.status
        )));
    }
    let created = installer_command("uv")?
        .args(["--no-config", "venv", "--python", "3.13"])
        .arg(&venv)
        .env_remove("UV_INDEX_URL")
        .env_remove("UV_EXTRA_INDEX_URL")
        .env_remove("UV_DEFAULT_INDEX")
        .output()?;
    if !created.status.success() {
        return Err(CatalogError::MissingRuntime(
            "Python 3.13 or uv-managed download unavailable".into(),
        ));
    }
    let python = venv.join("bin/python");
    let installed = installer_command("uv")?
        .args([
            "--no-config",
            "pip",
            "install",
            "--only-binary",
            ":all:",
            "--default-index",
            "https://pypi.org/simple",
            "--python",
        ])
        .arg(&python)
        .args(["--require-hashes", "-r"])
        .arg(&lock)
        .env_remove("UV_INDEX_URL")
        .env_remove("UV_EXTRA_INDEX_URL")
        .env_remove("UV_DEFAULT_INDEX")
        .output()?;
    if !installed.status.success() {
        return Err(CatalogError::Install(format!(
            "uv hash-locked install failed (exit {})",
            installed.status
        )));
    }
    let binary = venv.join("bin").join(entry.recipe.executable);
    if !binary.is_file() {
        return Err(CatalogError::Health(
            "uv installed the pinned packages but no expected executable was created".into(),
        ));
    }
    let lock_bytes = std::fs::read(&lock)?;
    let digest = blake3::hash(&lock_bytes).to_hex().to_string();
    Ok((binary, format!("uv-requirements-lock-blake3:{digest}")))
}

fn approved_git_commit(entry: &Integration) -> Result<String, CatalogError> {
    let tag = format!("v{}", entry.reviewed_version);
    let peeled = format!("refs/tags/{tag}^{{}}");
    let refs = installer_command("git")?
        .args([
            "ls-remote",
            entry.recipe.package,
            &format!("refs/tags/{tag}"),
            &peeled,
        ])
        .output()?;
    if !refs.status.success() {
        return Err(CatalogError::Install(
            "could not resolve the approved RTK tag over TLS".into(),
        ));
    }
    let rows = String::from_utf8_lossy(&refs.stdout);
    let chosen = rows
        .lines()
        .filter_map(|row| row.split_once('\t'))
        .find(|(_, reference)| *reference == peeled)
        .or_else(|| {
            rows.lines()
                .filter_map(|row| row.split_once('\t'))
                .find(|(_, reference)| *reference == format!("refs/tags/{tag}"))
        })
        .map(|(sha, _)| sha.to_ascii_lowercase())
        .filter(|sha| {
            (sha.len() == 40 || sha.len() == 64) && sha.bytes().all(|b| b.is_ascii_hexdigit())
        })
        .ok_or_else(|| {
            CatalogError::Install(
                "GitHub did not return a valid commit for the approved RTK tag".into(),
            )
        })?;
    Ok(chosen)
}

fn run_install(command: &mut Command) -> Result<(), CatalogError> {
    let output = command.output()?;
    if output.status.success() {
        Ok(())
    } else {
        let operation = command.get_args().map(|arg| arg.to_string_lossy())
            .collect::<Vec<_>>().join(" ");
        let message = if command.get_program().to_string_lossy().ends_with("/brew") {
            format!("Homebrew ทำงานไม่สำเร็จ ({}): brew {operation}", output.status)
        } else {
            format!("installer exited with {}", output.status)
        };
        Err(CatalogError::Install(message))
    }
}

/// Install from the approved Homebrew formula, or copy a bundled skill.
/// Legacy package-manager records remain readable and removable, but no new
/// npm, uv or Cargo installation is started by the catalog.
pub fn install(id: &str) -> Result<InstallRecord, CatalogError> {
    let entry = get(id).ok_or_else(|| CatalogError::Unknown(id.into()))?;
    let mut state = load_state()?;
    let old = state.installed.get(id).map(|x| x.version.clone());
    if entry.recipe.method == InstallMethod::Homebrew {
        if let Some(record) = homebrew_installed(entry)? {
            state.installed.insert(id.into(), record.clone());
            save_state(&state)?;
            return Ok(record);
        }
        let mut command = installer_command("brew")?;
        command.args(["install", "--formula", entry.recipe.package]);
        run_install(&mut command)?;
        let record = homebrew_installed(entry)?
            .ok_or_else(|| CatalogError::Health("Homebrew finished but the formula is not installed".into()))?;
        if !record.health.healthy {
            return Err(CatalogError::Health(record.health.detail));
        }
        state.installed.insert(id.into(), record.clone());
        state.history.push(UpdateRecord { id: id.into(), from: old, to: record.version.clone(), outcome: "healthy".into() });
        save_state(&state)?;
        return Ok(record);
    }
    if entry.recipe.method != InstallMethod::Bundled {
        return Err(CatalogError::Install(format!(
            "{} has no approved Homebrew formula yet", entry.title
        )));
    }
    if let Some(record) = state
        .installed
        .get(id)
        .filter(|r| r.version == entry.reviewed_version)
    {
        return Ok(record.clone());
    }
    let root = tools_path()?.join(id).join(entry.reviewed_version);
    if root.exists() {
        return Err(CatalogError::Install(format!(
            "managed version directory already exists: {}",
            root.display()
        )));
    }
    std::fs::create_dir_all(&root)?;
    if let Some(bin) = missing_installer(entry) {
        let _ = std::fs::remove_dir_all(&root);
        return Err(CatalogError::MissingRuntime(bin.into()));
    }
    let mut artifact_integrity = None;
    let bundled_skill = if entry.recipe.method == InstallMethod::Bundled {
        let path = root
            .join("skills")
            .join(entry.id)
            .join(entry.recipe.executable);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, CHORUS_REVIEW_SKILL)?;
        let digest = blake3::hash(CHORUS_REVIEW_SKILL.as_bytes())
            .to_hex()
            .to_string();
        artifact_integrity = Some(format!("blake3:{digest}"));
        Some(path)
    } else {
        None
    };
    let uv_executable = if entry.recipe.method == InstallMethod::Uv {
        let result = install_uv_locked(entry, &root);
        match result {
            Ok((path, lock_digest)) => {
                artifact_integrity = Some(lock_digest);
                Some(path)
            }
            Err(error) => {
                let _ = std::fs::remove_dir_all(&root);
                return Err(error);
            }
        }
    } else {
        None
    };
    let git_commit = if entry.recipe.method == InstallMethod::CargoGit {
        match approved_git_commit(entry) {
            Ok(commit) => {
                artifact_integrity = Some(format!("git-commit:{commit}"));
                Some(commit)
            }
            Err(error) => {
                let _ = std::fs::remove_dir_all(&root);
                return Err(error);
            }
        }
    } else {
        None
    };
    let npm_integrity = if entry.recipe.method == InstallMethod::Npm {
        let metadata = match installer_command("npm").and_then(|mut command| {
            command
            .args([
                "--userconfig=/dev/null",
                "--registry=https://registry.npmjs.org",
                "view",
                &format!("{}@{}", entry.recipe.package, entry.reviewed_version),
                "dist.integrity",
                "--json",
            ])
            .output()
            .map_err(CatalogError::Io)
        })
        {
            Ok(output) => output,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&root);
                return Err(CatalogError::Install(format!(
                    "cannot query npm integrity metadata: {e}"
                )));
            }
        };
        if !metadata.status.success() {
            let _ = std::fs::remove_dir_all(&root);
            return Err(CatalogError::Install(
                "npm did not provide integrity metadata over its configured registry connection"
                    .into(),
            ));
        }
        let integrity = String::from_utf8_lossy(&metadata.stdout)
            .trim()
            .trim_matches('"')
            .to_string();
        if !integrity.starts_with("sha512-") {
            let _ = std::fs::remove_dir_all(&root);
            return Err(CatalogError::Install(
                "npm registry returned no SHA-512 integrity value".into(),
            ));
        }
        Some(integrity)
    } else {
        None
    };
    let install_result = match entry.recipe.method {
        InstallMethod::Homebrew => unreachable!("handled before managed installs"),
        InstallMethod::Npm => run_install(
            installer_command("npm")?
                .args([
                    "--userconfig=/dev/null",
                    "--registry=https://registry.npmjs.org",
                    "install",
                    "--prefix",
                ])
                .arg(&root)
                .arg(format!(
                    "{}@{}",
                    entry.recipe.package, entry.reviewed_version
                )),
        ),
        InstallMethod::Uv => Ok(()),
        InstallMethod::CargoGit => run_install(
            installer_command("cargo")?
                .args([
                    "install",
                    "--git",
                    entry.recipe.package,
                    "--rev",
                    git_commit.as_deref().unwrap_or_default(),
                    "--locked",
                    "--root",
                ])
                .arg(&root),
        ),
        InstallMethod::Bundled => Ok(()),
    };
    if let Err(error) = install_result {
        let _ = std::fs::remove_dir_all(&root);
        return Err(error);
    }
    if let Some(expected) = npm_integrity.as_deref() {
        let lock = root.join("package-lock.json");
        let lock_data = std::fs::read(&lock)
            .and_then(|b| serde_json::from_slice::<Value>(&b).map_err(std::io::Error::other));
        let lock_integrity = lock_data.ok().and_then(|v| {
            v.get("packages")
                .and_then(Value::as_object)
                .and_then(|p| p.get(&format!("node_modules/{}", entry.recipe.package)))
                .and_then(|v| v.get("integrity"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
        if lock_integrity.as_deref() != Some(expected) {
            let _ = std::fs::remove_dir_all(&root);
            return Err(CatalogError::Install(
                "installed npm lock integrity did not match the registry metadata".into(),
            ));
        }
    }
    let installed_executable = match entry.recipe.method {
        InstallMethod::Homebrew => unreachable!("handled before managed installs"),
        InstallMethod::Npm => root
            .join("node_modules")
            .join(".bin")
            .join(entry.recipe.executable),
        InstallMethod::Uv => uv_executable.ok_or_else(|| {
            CatalogError::Install("managed Python executable was not created".into())
        })?,
        InstallMethod::CargoGit => root.join("bin").join(entry.recipe.executable),
        InstallMethod::Bundled => bundled_skill
            .ok_or_else(|| CatalogError::Install("bundled skill file was not created".into()))?,
    };
    let health = if entry.recipe.method == InstallMethod::Bundled {
        match std::fs::read_to_string(&installed_executable) {
            Ok(content) if content == CHORUS_REVIEW_SKILL => Health {
                healthy: true,
                detail: "bundled SKILL.md digest verified".into(),
            },
            Ok(_) => {
                let _ = std::fs::remove_dir_all(&root);
                return Err(CatalogError::Health(
                    "bundled skill content did not match its compiled digest source".into(),
                ));
            }
            Err(e) => {
                let _ = std::fs::remove_dir_all(&root);
                return Err(CatalogError::Health(e.to_string()));
            }
        }
    } else {
        let check = Command::new(&installed_executable)
            .env("PATH", child_path())
            .args(entry.recipe.health_args)
            .output();
        match check {
            Ok(o) if o.status.success() => Health {
                healthy: true,
                detail: String::from_utf8_lossy(&o.stdout).trim().to_string(),
            },
            Ok(o) => {
                let status = o.status.to_string();
                let _ = std::fs::remove_dir_all(&root);
                return Err(CatalogError::Health(format!(
                    "version check exited with {status}"
                )));
            }
            Err(e) => {
                let _ = std::fs::remove_dir_all(&root);
                return Err(CatalogError::Health(e.to_string()));
            }
        }
    };
    let record = InstallRecord {
        id: id.into(),
        version: entry.reviewed_version.into(),
        source: entry.source.into(),
        license: entry.license.into(),
        method: entry.recipe.method,
        runtime_path: installed_executable,
        artifact_integrity: npm_integrity.or(artifact_integrity),
        owned_files: vec![root.clone()],
        health,
    };
    state.installed.insert(id.into(), record.clone());
    state.history.push(UpdateRecord {
        id: id.into(),
        from: old,
        to: entry.reviewed_version.into(),
        outcome: "healthy".into(),
    });
    if let Err(e) = save_state(&state) {
        let _ = std::fs::remove_dir_all(root);
        return Err(e);
    }
    // Keep the old version: it may be executing in an already-running session.
    Ok(record)
}

pub fn update(id: &str) -> Result<InstallRecord, CatalogError> {
    let entry = get(id).ok_or_else(|| CatalogError::Unknown(id.into()))?;
    if entry.recipe.method == InstallMethod::Homebrew {
        if homebrew_installed(entry)?.is_none() {
            return install(id);
        }
        let old = load_state()?.installed.get(id).map(|record| record.version.clone());
        run_install(installer_command("brew")?.args(["upgrade", "--formula", entry.recipe.package]))?;
        let record = homebrew_installed(entry)?
            .ok_or_else(|| CatalogError::Health("Homebrew finished but the formula is not installed".into()))?;
        if !record.health.healthy {
            return Err(CatalogError::Health(record.health.detail));
        }
        let mut state = load_state()?;
        state.installed.insert(id.into(), record.clone());
        state.history.push(UpdateRecord { id: id.into(), from: old, to: record.version.clone(), outcome: "healthy".into() });
        save_state(&state)?;
        return Ok(record);
    }
    install(id)
}

pub fn set_enabled(project: &Path, id: &str, enabled: bool) -> Result<(), CatalogError> {
    let entry = get(id).ok_or_else(|| CatalogError::Unknown(id.into()))?;
    let key = project_key(project)?;
    let mut state = load_state()?;
    let available = if entry.recipe.method == InstallMethod::Homebrew {
        homebrew_installed(entry)?.is_some()
    } else {
        state.installed.contains_key(id)
    };
    if enabled && !available {
        return Err(CatalogError::Install(format!("{id} is not installed")));
    }
    let ids = state.projects.entry(key.clone()).or_default();
    if enabled {
        ids.insert(id.into());
    } else {
        ids.remove(id);
    }
    if ids.is_empty() {
        state.projects.remove(&key);
    }
    save_state(&state)
}

pub fn enabled_for(project: &Path) -> Result<Vec<&'static Integration>, CatalogError> {
    let key = project_key(project)?;
    let state = load_state()?;
    Ok(state
        .projects
        .get(&key)
        .into_iter()
        .flatten()
        .filter_map(|id| get(id))
        .filter(|entry| entry.recipe.method != InstallMethod::Homebrew
            || homebrew_installed(entry).ok().flatten().is_some())
        .collect())
}

fn project_key(path: &Path) -> Result<String, CatalogError> {
    Ok(std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned())
}

pub fn installed() -> Result<Vec<InstallRecord>, CatalogError> {
    let mut state = load_state()?;
    for entry in CATALOG.iter().filter(|entry| entry.recipe.method == InstallMethod::Homebrew) {
        if let Some(record) = homebrew_installed(entry).ok().flatten() {
            state.installed.insert(entry.id.into(), record);
        } else {
            state.installed.remove(entry.id);
        }
    }
    Ok(state.installed.into_values().collect())
}

/// Remove a Homebrew formula, or a legacy Chorus-owned install. The caller
/// presents a destructive confirmation because Homebrew packages are global.
pub fn uninstall(id: &str) -> Result<(), CatalogError> {
    let mut state = load_state()?;
    let entry = get(id).ok_or_else(|| CatalogError::Unknown(id.into()))?;
    if entry.recipe.method == InstallMethod::Homebrew {
        if homebrew_installed(entry)?.is_some() {
            run_install(installer_command("brew")?
                .args(["uninstall", "--formula", entry.recipe.package]))?;
        }
        if let Some(record) = state.installed.get(id).filter(|record| record.method != InstallMethod::Homebrew) {
            remove_owned_files(record)?;
        }
        state.installed.remove(id);
        for ids in state.projects.values_mut() { ids.remove(id); }
        state.projects.retain(|_, ids| !ids.is_empty());
        return save_state(&state);
    }
    if let Some(record) = state.installed.remove(id) {
        remove_owned_files(&record)?;
    }
    for ids in state.projects.values_mut() { ids.remove(id); }
    state.projects.retain(|_, ids| !ids.is_empty());
    save_state(&state)?;
    Ok(())
}

fn remove_owned_files(record: &InstallRecord) -> Result<(), CatalogError> {
    let managed_root = match std::fs::canonicalize(tools_path()?) {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let mut owned_paths = Vec::new();
    for path in &record.owned_files {
        let canonical = match std::fs::canonicalize(path) {
            Ok(path) => path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if !canonical.starts_with(&managed_root) || canonical == managed_root {
            return Err(CatalogError::Install(format!(
                "refusing to remove path outside Chorus tools: {}", path.display()
            )));
        }
        owned_paths.push(canonical);
    }
    for path in owned_paths {
        std::fs::remove_dir_all(path)?;
    }
    Ok(())
}

/// Build CLI search paths and mode-specific launch projections for a project.
pub fn cli_paths_for(project: &Path) -> Result<Vec<PathBuf>, CatalogError> {
    let state = load_state()?;
    Ok(enabled_for(project)?
        .into_iter()
        .filter(|e| e.modes.contains(&Mode::Cli))
        .filter_map(|e| {
            homebrew_installed(e).ok().flatten().or_else(|| state.installed.get(e.id).cloned()).map(|r| {
                r.runtime_path
                    .parent()
                    .unwrap_or(Path::new(""))
                    .to_path_buf()
            })
        })
        .collect())
}

/// RTK is explicitly invoked and never rewrites shell commands. This instruction
/// can be appended to the agent's project instructions by the caller.
pub fn project_instructions(project: &Path) -> Result<Vec<String>, CatalogError> {
    let state = load_state()?;
    let mut instructions = Vec::new();
    for entry in enabled_for(project)? {
        if entry.id == "rtk" {
            instructions.push("Use the enabled `rtk` executable explicitly as `rtk <command>` when filtered command output is useful.".to_string());
        } else if entry.modes.contains(&Mode::Skill) {
            if let Some(record) = state.installed.get(entry.id) {
                instructions.push(format!(
                    "For tasks covered by {}, read the enabled skill at `{}` before acting.",
                    entry.title,
                    record.runtime_path.display()
                ));
            }
        }
    }
    Ok(instructions)
}

/// MCP projection. `mcp_config` performs the merge and returns name collisions;
/// caller can surface them rather than silently shadowing user entries.
pub fn project_mcp(project: &Path) -> Result<Vec<Value>, CatalogError> {
    let state = load_state()?;
    Ok(enabled_for(project)?
        .into_iter()
        .filter(|e| e.modes.contains(&Mode::Mcp))
        .filter_map(|e| homebrew_installed(e).ok().flatten().or_else(|| state.installed.get(e.id).cloned()).map(|r| server_for(e, &r, project)))
        .collect())
}

fn server_for(entry: &Integration, record: &InstallRecord, project: &Path) -> Value {
    let path = record.runtime_path.to_string_lossy();
    match entry.id {
        "graft" => serde_json::json!({"name":"graft","command":path,"args":["mcp"],"env":[]}),
        "serena" => {
            serde_json::json!({"name":"serena","command":path,"args":["start-mcp-server","--context=ide","--project",project.to_string_lossy()],"env":[]})
        }
        "headroom" => serde_json::json!({"name":"headroom","command":path,"args":["mcp"],"env":[]}),
        _ => serde_json::json!({}),
    }
}
