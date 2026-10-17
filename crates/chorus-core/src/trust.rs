//! Per-project consent for running the project's own code.
//!
//! `cargo check` compiles and runs `build.rs` and proc-macros and honours
//! `.cargo/config.toml`'s `rustc-wrapper`; `.chorus/hooks/*` are scripts; the
//! linters load project plugins. Any of those can be planted by a cloned repo
//! or written by a sandboxed agent during a turn. Running them automatically
//! after the turn turned "writes stay in the workspace" into "code runs as the
//! user, outside Seatbelt".
//!
//! So nothing here runs unless the user trusted the project, and even then it
//! runs through a [`Confine`] that wraps the command in the turn's Seatbelt
//! profile. Seatbelt only limits writes — a hostile `build.rs` can still read
//! and exfiltrate — which is why trust is required as well, not instead.
//!
//! The trust list lives in the user's Chorus home (`$CHORUS_HOME` or
//! `~/.chorus/trusted-projects.json`), never in the workspace: a repo cannot
//! ship its own consent, and the sandbox does not allow agents to write there.

use crate::{ChorusError, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Wraps an argv so the process it starts is confined by the OS sandbox, or
/// says why that cannot be done. `cwd` is the workspace the profile is built for.
pub type Confine<'a> = dyn Fn(&[String], &Path) -> std::result::Result<Vec<String>, String> + 'a;

/// The user's list of trusted project roots.
#[derive(Debug, Clone)]
pub struct TrustStore {
    path: PathBuf,
}

impl TrustStore {
    /// `$CHORUS_HOME/trusted-projects.json`, or `~/.chorus/trusted-projects.json`.
    pub fn default_location() -> Self {
        let home = std::env::var_os("CHORUS_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".chorus")))
            .unwrap_or_else(|| PathBuf::from(".chorus"));
        Self::at(home.join("trusted-projects.json"))
    }

    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    fn key(workspace: &Path) -> Option<String> {
        workspace
            .canonicalize()
            .ok()
            .and_then(|p| p.to_str().map(str::to_string))
    }

    fn load(&self) -> Vec<String> {
        std::fs::read(&self.path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    /// Whether the user trusted `workspace`. Unknown, unreadable or missing = no.
    pub fn is_trusted(&self, workspace: &Path) -> bool {
        match Self::key(workspace) {
            Some(k) => self.load().contains(&k),
            None => false,
        }
    }

    /// Record the user's decision for `workspace`.
    pub fn set_trusted(&self, workspace: &Path, trusted: bool) -> Result<()> {
        let key = Self::key(workspace)
            .ok_or_else(|| ChorusError::Invalid(format!("cannot resolve {}", workspace.display())))?;
        let mut list = self.load();
        list.retain(|k| k != &key);
        if trusted {
            list.push(key);
            list.sort();
        }
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&list)?)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    /// Consent to run project code in `workspace`, or an error when the user
    /// has not trusted it. The only way to obtain a [`Consent`].
    pub fn consent<'a>(&self, workspace: &Path, confine: &'a Confine<'a>) -> Result<Consent<'a>> {
        if !self.is_trusted(workspace) {
            return Err(ChorusError::Invalid(format!(
                "project not trusted: {} — turn on \"trust this project\" to run checks or hooks",
                workspace.display()
            )));
        }
        Ok(Consent { workspace: workspace.to_path_buf(), confine })
    }
}

/// Proof that the user trusted this workspace, plus the sandbox to run in.
pub struct Consent<'a> {
    workspace: PathBuf,
    confine: &'a Confine<'a>,
}

impl Consent<'_> {
    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    /// A command for `argv`, confined to the sandbox and started in the
    /// workspace. Fails closed: no sandbox, no command.
    pub fn command(&self, argv: &[String]) -> Result<Command> {
        let wrapped = (self.confine)(argv, &self.workspace)
            .map_err(|why| ChorusError::Invalid(format!("refusing to run unsandboxed: {why}")))?;
        let (prog, args) = wrapped
            .split_first()
            .ok_or_else(|| ChorusError::Invalid("empty command".into()))?;
        let mut cmd = Command::new(prog);
        cmd.args(args).current_dir(&self.workspace);
        Ok(cmd)
    }
}

/// Resolve `tool` on `PATH`, skipping relative entries so a repo cannot put
/// its own `biome` ahead of the user's through `PATH=.:…` or `node_modules/.bin`.
pub fn which(tool: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(tool))
        .find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(argv: &[String], _: &Path) -> std::result::Result<Vec<String>, String> {
        Ok(argv.to_vec())
    }

    #[test]
    fn projects_are_untrusted_until_the_user_says_so() {
        let home = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let store = TrustStore::at(home.path().join("trusted-projects.json"));
        assert!(!store.is_trusted(ws.path()));
        assert!(store.consent(ws.path(), &identity).is_err());

        store.set_trusted(ws.path(), true).unwrap();
        assert!(store.is_trusted(ws.path()));
        assert!(store.consent(ws.path(), &identity).is_ok());

        store.set_trusted(ws.path(), false).unwrap();
        assert!(!store.is_trusted(ws.path()));
    }

    #[test]
    fn consent_refuses_when_the_sandbox_is_unavailable() {
        let home = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let store = TrustStore::at(home.path().join("t.json"));
        store.set_trusted(ws.path(), true).unwrap();
        let no_sandbox = |_: &[String], _: &Path| Err("no sandbox-exec".to_string());
        let consent = store.consent(ws.path(), &no_sandbox).unwrap();
        assert!(consent.command(&["true".into()]).is_err());
    }
}
