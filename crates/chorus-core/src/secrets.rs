//! Secrets: keep the real values of API keys, tokens and passwords out of
//! every prompt, the ledger and disk — and still let agents use them.
//!
//! Three layers, from the user's design (20 Oct 2026):
//!
//! 1. **Mask.** Text that passes through Chorus — what the user types, what
//!    agents send back, attachments — is scanned ([`find`]). Each secret is
//!    replaced by a name like `$CHORUS_SECRET_GITHUB_1`; the value goes to the
//!    login Keychain ([`Vault`]), and only a BLAKE3 fingerprint is written to
//!    `~/.chorus/secrets.json` so the same value always gets the same name.
//! 2. **Inject.** When an agent starts, every name its prompt mentions is set
//!    as an environment variable of the bridge process ([`Vault::resolve`]).
//!    `$CHORUS_SECRET_GITHUB_1` in a shell command then expands to the real
//!    value without the model ever seeing it.
//! 3. **Block.** The agent sandbox denies reading `.env` files and key files
//!    (chorus-acp `sandbox`); Chorus reads the project's `.env` itself
//!    ([`parse_dotenv`]) and hands its variables to the agent the same way.
//!
//! What this cannot do: an agent that runs `echo $CHORUS_SECRET_X` shows the
//! value to its own provider. The value is masked again before it reaches
//! the ledger, so it does not travel on to other agents or to disk.
//!
//! No regex dependency: the detector walks tokens and checks the published
//! shapes of each provider's keys, plus `NAME=value` where NAME says secret.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// Prefix of every placeholder Chorus writes.
pub const PREFIX: &str = "CHORUS_SECRET_";

/// One secret found in a text, by byte range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub start: usize,
    pub end: usize,
    /// Upper-case label used in the placeholder: `GITHUB`, `DB_PASSWORD`…
    pub kind: String,
}

fn is_tok(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-')
}

fn all(s: &str, f: impl Fn(char) -> bool) -> bool {
    s.chars().all(f)
}

/// (prefix, label, minimum length of the rest, allowed characters).
type Shape = (&'static str, &'static str, usize, fn(char) -> bool);

/// Provider key shapes. Lengths are the documented ones or a safe lower
/// bound, so ordinary words that merely start the same way (`sk-learn`) are
/// not taken for keys.
const SHAPES: &[Shape] = &[
    ("sk-ant-", "ANTHROPIC", 20, is_tok),
    ("sk-proj-", "OPENAI", 20, is_tok),
    ("sk-svcacct-", "OPENAI", 20, is_tok),
    ("sk-", "OPENAI", 32, is_tok),
    ("github_pat_", "GITHUB", 22, is_tok),
    ("ghp_", "GITHUB", 36, is_tok),
    ("gho_", "GITHUB", 36, is_tok),
    ("ghu_", "GITHUB", 36, is_tok),
    ("ghs_", "GITHUB", 36, is_tok),
    ("ghr_", "GITHUB", 36, is_tok),
    ("glpat-", "GITLAB", 20, is_tok),
    ("xoxb-", "SLACK", 10, is_tok),
    ("xoxp-", "SLACK", 10, is_tok),
    ("xoxa-", "SLACK", 10, is_tok),
    ("xoxs-", "SLACK", 10, is_tok),
    ("AIza", "GOOGLE", 35, is_tok),
    ("sk_live_", "STRIPE", 16, is_tok),
    ("rk_live_", "STRIPE", 16, is_tok),
    ("hf_", "HUGGINGFACE", 30, |c| c.is_ascii_alphanumeric()),
    ("npm_", "NPM", 36, |c| c.is_ascii_alphanumeric()),
];

/// Variable names that say "the value is a secret".
const SECRET_WORDS: &[&str] = &[
    "SECRET",
    "TOKEN",
    "PASSWORD",
    "PASSWD",
    "PASSPHRASE",
    "API_KEY",
    "APIKEY",
    "PRIVATE_KEY",
    "ACCESS_KEY",
    "CREDENTIAL",
    "DSN",
];

/// A plain word after `Name:` is prose ("Token: expired"), not a secret.
/// Real values have a digit, a symbol, or case changes past the first letter.
fn looks_random(v: &str) -> bool {
    v.chars().any(|c| c.is_ascii_digit())
        || v.chars().any(|c| !c.is_ascii_alphanumeric())
        || (v.len() >= 12 && v.chars().skip(1).any(|c| c.is_ascii_uppercase()))
}

/// Values that look assigned but are obviously not real secrets.
fn is_placeholder_value(v: &str) -> bool {
    let l = v.to_ascii_lowercase();
    v.len() < 8
        || v.starts_with('$')
        || v.starts_with('<')
        || v.starts_with("${")
        || v.contains(PREFIX)
        || l.chars().all(|c| c == 'x' || c == '*' || c == '.')
        || [
            "changeme",
            "change_me",
            "your_",
            "your-",
            "example",
            "placeholder",
            "dummy",
            "todo",
            "none",
            "null",
            "true",
            "false",
        ]
        .iter()
        .any(|p| l.contains(p))
}

/// Every secret in `text`, sorted, non-overlapping.
pub fn find(text: &str) -> Vec<Found> {
    let mut out: Vec<Found> = Vec::new();
    find_private_keys(text, &mut out);
    find_shapes(text, &mut out);
    find_jwts(text, &mut out);
    find_assignments(text, &mut out);
    out.sort_by_key(|f| (f.start, std::cmp::Reverse(f.end)));
    let mut kept: Vec<Found> = Vec::new();
    for f in out {
        if kept.last().is_some_and(|k| f.start < k.end) {
            continue;
        }
        kept.push(f);
    }
    kept
}

fn find_private_keys(text: &str, out: &mut Vec<Found>) {
    let mut from = 0;
    while let Some(i) = text[from..].find("-----BEGIN ") {
        let start = from + i;
        let header_end = match text[start..].find('\n') {
            Some(n) => start + n,
            None => break,
        };
        if !text[start..header_end].contains("PRIVATE KEY-----") {
            from = header_end;
            continue;
        }
        match text[header_end..].find("-----END ") {
            Some(e) => {
                let end_line = header_end + e;
                let end = text[end_line..]
                    .find("-----\n")
                    .map(|n| end_line + n + 5)
                    .unwrap_or_else(|| {
                        text[end_line..]
                            .find("KEY-----")
                            .map_or(text.len(), |n| end_line + n + 8)
                    });
                out.push(Found {
                    start,
                    end,
                    kind: "PRIVATE_KEY".into(),
                });
                from = end;
            }
            None => break,
        }
    }
}

/// Byte ranges of maximal runs of key characters (`[A-Za-z0-9_-]`).
fn tokens(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut v = Vec::new();
    let mut start: Option<usize> = None;
    for (i, c) in text.char_indices() {
        let t = is_tok(c);
        match (t, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                v.push((s, &text[s..i]));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        v.push((s, &text[s..]));
    }
    v.into_iter()
}

fn find_shapes(text: &str, out: &mut Vec<Found>) {
    for (pos, tok) in tokens(text) {
        if tok.len() == 20
            && (tok.starts_with("AKIA") || tok.starts_with("ASIA"))
            && all(&tok[4..], |c| c.is_ascii_uppercase() || c.is_ascii_digit())
        {
            out.push(Found {
                start: pos,
                end: pos + 20,
                kind: "AWS".into(),
            });
            continue;
        }
        for (prefix, kind, min, ok) in SHAPES {
            if let Some(rest) = tok.strip_prefix(prefix) {
                if rest.len() >= *min && all(rest, *ok) && rest.chars().any(|c| c.is_ascii_digit())
                {
                    out.push(Found {
                        start: pos,
                        end: pos + tok.len(),
                        kind: (*kind).into(),
                    });
                }
                break;
            }
        }
    }
}

fn find_jwts(text: &str, out: &mut Vec<Found>) {
    let b64 = |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '-');
    let mut from = 0;
    while let Some(i) = text[from..].find("eyJ") {
        let start = from + i;
        let end = text[start..]
            .find(|c: char| !(b64(c) || c == '.'))
            .map_or(text.len(), |n| start + n);
        let parts: Vec<&str> = text[start..end].split('.').collect();
        if parts.len() == 3 && parts.iter().all(|p| p.len() >= 10) {
            out.push(Found {
                start,
                end,
                kind: "JWT".into(),
            });
        }
        from = end.max(start + 3);
    }
}

/// `NAME=value`, `NAME: value`, `"name": "value"` where NAME says secret.
fn find_assignments(text: &str, out: &mut Vec<Found>) {
    for line_start in std::iter::once(0).chain(text.match_indices('\n').map(|(i, _)| i + 1)) {
        let line_end = text[line_start..]
            .find('\n')
            .map_or(text.len(), |n| line_start + n);
        let line = &text[line_start..line_end];
        let Some(sep) = line.find(['=', ':']) else {
            continue;
        };
        let name = line[..sep]
            .trim()
            .trim_start_matches("export ")
            .trim_matches(|c| c == '"' || c == '\'')
            .trim();
        if name.is_empty()
            || name.len() > 64
            || !all(name, |c| {
                c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.'
            })
        {
            continue;
        }
        let upper = name.to_ascii_uppercase().replace(['-', '.'], "_");
        if !SECRET_WORDS.iter().any(|w| upper.contains(w)) {
            continue;
        }
        let raw = &line[sep + 1..];
        let lead = raw.len() - raw.trim_start().len();
        let mut v = raw.trim();
        let mut v_off = sep + 1 + lead;
        v = v.trim_end_matches(',');
        let mut quoted = false;
        if let Some(q) = v.chars().next().filter(|c| *c == '"' || *c == '\'') {
            if let Some(close) = v[1..].find(q) {
                v = &v[1..1 + close];
                v_off += 1;
                quoted = true;
            }
        }
        // Unquoted code on the right-hand side is a lookup, not a value:
        // `api_key = os.environ['X']`, `token = get_token()`, `pass: cfg.db.pass`.
        let looks_like_code = !quoted
            && (v.contains(['(', ')', '[', ']', '{', '}', '\'', '"'])
                || (all(v, |c| c.is_ascii_alphabetic() || c == '_' || c == '.')
                    && v.contains('.')));
        if looks_like_code
            || v.contains(char::is_whitespace)
            || is_placeholder_value(v)
            || !looks_random(v)
        {
            continue;
        }
        let start = line_start + v_off;
        out.push(Found {
            start,
            end: start + v.len(),
            kind: label(&upper),
        });
    }
}

/// `DB_PASSWORD` → `DB_PASSWORD`; anything odd → `GENERIC`.
fn label(name: &str) -> String {
    let l: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    if l.is_empty() {
        "GENERIC".into()
    } else {
        l
    }
}

/// Placeholder names a text mentions (`CHORUS_SECRET_…`), in order, deduped.
pub fn referenced(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (i, _) in text.match_indices(PREFIX) {
        let end = text[i..]
            .find(|c: char| !(c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
            .map_or(text.len(), |n| i + n);
        let name = &text[i..end];
        if name.len() > PREFIX.len() && !out.iter().any(|o| o == name) {
            out.push(name.to_string());
        }
    }
    out
}

/// `KEY=value` pairs of a dotenv file: comments, `export`, quotes handled.
pub fn parse_dotenv(content: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let k = k.trim();
        if k.is_empty()
            || !all(k, |c| c.is_ascii_alphanumeric() || c == '_')
            || k.starts_with(|c: char| c.is_ascii_digit())
        {
            continue;
        }
        let v = v.trim();
        let v = if (v.starts_with('"') && v.ends_with('"') && v.len() >= 2)
            || (v.starts_with('\'') && v.ends_with('\'') && v.len() >= 2)
        {
            &v[1..v.len() - 1]
        } else {
            v.split(" #").next().unwrap_or(v).trim()
        };
        out.push((k.to_string(), v.to_string()));
    }
    out
}

// ---- vault ------------------------------------------------------------------

/// Where values live. Names are placeholders without the `$`.
pub trait Backend: Send + Sync {
    fn put(&self, name: &str, value: &str) -> Result<(), String>;
    fn get(&self, name: &str) -> Result<Option<String>, String>;
    fn delete(&self, name: &str) -> Result<(), String>;
}

/// Process memory only: tests, and platforms without a Keychain. Names
/// outlive nothing here, so injection works within one run only.
#[derive(Default)]
pub struct Memory(Mutex<BTreeMap<String, String>>);

impl Backend for Memory {
    fn put(&self, name: &str, value: &str) -> Result<(), String> {
        self.0
            .lock()
            .map_err(|_| "poisoned")?
            .insert(name.into(), value.into());
        Ok(())
    }
    fn get(&self, name: &str) -> Result<Option<String>, String> {
        Ok(self.0.lock().map_err(|_| "poisoned")?.get(name).cloned())
    }
    fn delete(&self, name: &str) -> Result<(), String> {
        self.0.lock().map_err(|_| "poisoned")?.remove(name);
        Ok(())
    }
}

/// The macOS login Keychain (or `keychain`, for tests), through the
/// system's `security` tool. Values are hex-encoded so no quoting rule of
/// `security -i` can split them, and written over stdin, never argv — any
/// local user can read another process's argv with `ps`.
pub struct Keychain {
    pub keychain: Option<PathBuf>,
}

pub const KEYCHAIN_SERVICE: &str = "Chorus secrets";

fn hex(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<String> {
    let s = s.trim();
    if s.len() % 2 != 0 {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect();
    String::from_utf8(bytes?).ok()
}

impl Keychain {
    fn tail(&self) -> String {
        self.keychain
            .as_ref()
            .map(|k| format!(" \"{}\"", k.display()))
            .unwrap_or_default()
    }
}

impl Backend for Keychain {
    fn put(&self, name: &str, value: &str) -> Result<(), String> {
        use std::io::Write;
        if !all(name, |c| c.is_ascii_alphanumeric() || c == '_') {
            return Err("bad secret name".into());
        }
        let mut child = std::process::Command::new("/usr/bin/security")
            .arg("-i")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("security: {e}"))?;
        let line = format!(
            "add-generic-password -U -s \"{KEYCHAIN_SERVICE}\" -a {name} -w {}{}\n",
            hex(value),
            self.tail()
        );
        child
            .stdin
            .take()
            .ok_or("security stdin")?
            .write_all(line.as_bytes())
            .map_err(|e| e.to_string())?;
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        let err = String::from_utf8_lossy(&out.stderr);
        if out.status.success() && !err.contains("error") {
            Ok(())
        } else {
            Err(format!("Keychain refused the secret: {}", err.trim()))
        }
    }

    fn get(&self, name: &str) -> Result<Option<String>, String> {
        let mut cmd = std::process::Command::new("/usr/bin/security");
        cmd.args([
            "find-generic-password",
            "-s",
            KEYCHAIN_SERVICE,
            "-a",
            name,
            "-w",
        ]);
        if let Some(k) = &self.keychain {
            cmd.arg(k);
        }
        let out = cmd.output().map_err(|e| format!("security: {e}"))?;
        if !out.status.success() {
            return Ok(None); // 44 = not found
        }
        Ok(unhex(&String::from_utf8_lossy(&out.stdout)))
    }

    fn delete(&self, name: &str) -> Result<(), String> {
        let mut cmd = std::process::Command::new("/usr/bin/security");
        cmd.args([
            "delete-generic-password",
            "-s",
            KEYCHAIN_SERVICE,
            "-a",
            name,
        ]);
        if let Some(k) = &self.keychain {
            cmd.arg(k);
        }
        cmd.stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        cmd.status().map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// One remembered secret — never its value.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub kind: String,
    /// BLAKE3 of the value: the same value keeps its name across sessions.
    pub fingerprint: String,
}

/// The result of masking a text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Masked {
    pub text: String,
    /// Names used in `text`, new or remembered.
    pub names: Vec<String>,
    /// Values that could not be stored (no Keychain): still masked, but
    /// cannot be injected later.
    pub unstored: usize,
}

pub struct Vault {
    backend: Box<dyn Backend>,
    index_path: Option<PathBuf>,
    index: Mutex<Vec<Entry>>,
    /// Values seen this run, longest first, so text that repeats a value
    /// verbatim (an agent echoing an injected variable) is masked even when
    /// no pattern would recognise it.
    known: Mutex<Vec<(String, String)>>,
    /// Values the backend refused (locked Keychain): still usable for the
    /// rest of this run, just not remembered across runs.
    fallback: Memory,
}

impl Vault {
    pub fn new(backend: Box<dyn Backend>, index_path: Option<PathBuf>) -> Self {
        let index = index_path
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Vault {
            backend,
            index_path,
            index: Mutex::new(index),
            known: Mutex::new(Vec::new()),
            fallback: Memory::default(),
        }
    }

    pub fn in_memory() -> Self {
        Self::new(Box::<Memory>::default(), None)
    }

    fn save_index(&self, index: &[Entry]) {
        let Some(p) = &self.index_path else { return };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(bytes) = serde_json::to_vec_pretty(index) {
            let tmp = p.with_extension("json.tmp");
            if std::fs::write(&tmp, bytes).is_ok() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
                }
                let _ = std::fs::rename(&tmp, p);
            }
        }
    }

    fn remember(&self, name: &str, value: &str) {
        if let Ok(mut k) = self.known.lock() {
            if !k.iter().any(|(_, v)| v == value) {
                k.push((name.to_string(), value.to_string()));
                k.sort_by_key(|(_, v)| std::cmp::Reverse(v.len()));
            }
        }
    }

    /// Put in the backend; on refusal keep it for this run. `false` = not
    /// persisted.
    fn store(&self, name: &str, value: &str) -> bool {
        if self.backend.put(name, value).is_ok() {
            return true;
        }
        let _ = self.fallback.put(name, value);
        false
    }

    /// The name for `value`, allocating and storing it the first time.
    /// The bool is `false` when the backend refused the value (see `store`).
    pub fn name_for(&self, kind: &str, value: &str) -> (String, bool) {
        let fp = blake3::hash(value.as_bytes()).to_hex().to_string();
        let mut index = self.index.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(e) = index.iter().find(|e| e.fingerprint == fp) {
            let name = e.name.clone();
            drop(index);
            // Re-store if the backend lost it (Keychain reset, other Mac).
            let stored = matches!(self.backend.get(&name), Ok(Some(_))) || self.store(&name, value);
            self.remember(&name, value);
            return (name, stored);
        }
        let base = format!("{PREFIX}{kind}");
        let mut n = 1;
        let name = loop {
            let candidate = format!("{base}_{n}");
            if !index.iter().any(|e| e.name == candidate) {
                break candidate;
            }
            n += 1;
        };
        let stored = self.store(&name, value);
        index.push(Entry {
            name: name.clone(),
            kind: kind.to_string(),
            fingerprint: fp,
        });
        self.save_index(&index);
        drop(index);
        self.remember(&name, value);
        (name, stored)
    }

    /// Replace every secret in `text` with `$NAME`.
    pub fn mask(&self, text: &str) -> Masked {
        let mut names = Vec::new();
        let mut unstored = 0;
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        for f in find(text) {
            let value = &text[f.start..f.end];
            let (name, stored) = self.name_for(&f.kind, value);
            if !stored {
                unstored += 1;
            }
            out.push_str(&text[last..f.start]);
            out.push('$');
            out.push_str(&name);
            if !names.contains(&name) {
                names.push(name);
            }
            last = f.end;
        }
        out.push_str(&text[last..]);
        // Values injected or masked earlier this run, repeated verbatim.
        let known = self.known.lock().map(|k| k.clone()).unwrap_or_default();
        for (name, value) in known {
            if value.len() >= 8 && out.contains(&value) {
                out = out.replace(&value, &format!("${name}"));
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        }
        Masked {
            text: out,
            names,
            unstored,
        }
    }

    /// `(name, value)` for each name the backend still has — the environment
    /// for an agent whose prompt mentions them.
    pub fn resolve(&self, names: &[String]) -> Vec<(String, String)> {
        names
            .iter()
            .filter_map(|n| {
                let v = match self.backend.get(n) {
                    Ok(Some(v)) => v,
                    _ => self.fallback.get(n).ok().flatten()?,
                };
                self.remember(n, &v);
                Some((n.clone(), v))
            })
            .collect()
    }

    /// Everything remembered, for Settings. Names and kinds only.
    pub fn list(&self) -> Vec<Entry> {
        self.index.lock().map(|i| i.clone()).unwrap_or_default()
    }

    /// Forget one secret: Keychain item, index entry, in-memory copy.
    pub fn forget(&self, name: &str) -> Result<(), String> {
        self.backend.delete(name)?;
        let _ = self.fallback.delete(name);
        let mut index = self.index.lock().unwrap_or_else(|p| p.into_inner());
        index.retain(|e| e.name != name);
        self.save_index(&index);
        if let Ok(mut k) = self.known.lock() {
            k.retain(|(n, _)| n != name);
        }
        Ok(())
    }
}

/// The user's vault: the login Keychain on macOS (`~/.chorus/secrets.json`
/// for the index), process memory elsewhere or with
/// `CHORUS_SECRETS_BACKEND=memory` (tests).
pub fn user_vault() -> &'static Vault {
    // Checked on every call, not once: tests set it before their first
    // secret, and must never write test values into a developer's Keychain.
    static MEMORY: OnceLock<Vault> = OnceLock::new();
    static KEYCHAIN: OnceLock<Vault> = OnceLock::new();
    let memory = std::env::var("CHORUS_SECRETS_BACKEND").is_ok_and(|v| v == "memory");
    if !cfg!(target_os = "macos") || memory {
        return MEMORY.get_or_init(Vault::in_memory);
    }
    KEYCHAIN.get_or_init(|| {
        let index =
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".chorus").join("secrets.json"));
        Vault::new(Box::new(Keychain { keychain: None }), index)
    })
}

/// Variables a project `.env` must not set on the agent's process: its own
/// auth and runtime. A project's `ANTHROPIC_API_KEY` would silently switch
/// the Claude bridge from the user's login to API billing. These still
/// reach the agent under their `CHORUS_SECRET_…` name.
fn reserved_env(name: &str) -> bool {
    const EXACT: &[&str] = &[
        "PATH",
        "HOME",
        "SHELL",
        "USER",
        "LOGNAME",
        "TMPDIR",
        "PWD",
        "NODE_OPTIONS",
        "GOOGLE_API_KEY",
        "GOOGLE_APPLICATION_CREDENTIALS",
    ];
    const PREFIXES: &[&str] = &[
        "ANTHROPIC_",
        "CLAUDE",
        "OPENAI_",
        "CODEX_",
        "GEMINI_",
        "CHORUS_",
        "DYLD_",
        "LD_",
        "NPM_CONFIG_",
    ];
    EXACT.contains(&name) || PREFIXES.iter().any(|p| name.starts_with(p))
}

/// The project's `.env` (then `.env.local`), read by Chorus outside the
/// sandbox so the agent never has to open it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ProjectEnv {
    /// Environment for the agent process: each variable under its own name
    /// (unless reserved) and each secret also under its placeholder.
    pub env: Vec<(String, String)>,
    /// Variable names the agent may use — never values.
    pub names: Vec<String>,
}

const DOTENV_MAX_BYTES: u64 = 64 * 1024;

pub fn project_env(cwd: &std::path::Path, vault: &Vault) -> ProjectEnv {
    let mut vars: Vec<(String, String)> = Vec::new();
    for f in [".env", ".env.local"] {
        let p = cwd.join(f);
        let ok = std::fs::metadata(&p).is_ok_and(|m| m.is_file() && m.len() <= DOTENV_MAX_BYTES);
        let Some(content) = ok.then(|| std::fs::read_to_string(&p).ok()).flatten() else {
            continue;
        };
        for (k, v) in parse_dotenv(&content) {
            vars.retain(|(old, _)| old != &k);
            vars.push((k, v));
        }
    }
    let mut out = ProjectEnv::default();
    for (k, v) in vars {
        let upper = k.to_ascii_uppercase();
        // A remembered value is replaced wherever it appears later, so a
        // plain word (`SECRET_MODE=production`) must not become one.
        let named_secret =
            SECRET_WORDS.iter().any(|w| upper.contains(w)) && v.len() >= 8 && looks_random(&v);
        let secret = named_secret || !find(&format!("{k}={v}")).is_empty() || !find(&v).is_empty();
        if secret {
            let (name, _) = vault.name_for(&label(&upper), &v);
            out.env.push((name.clone(), v.clone()));
            out.names.push(format!("{k} (also ${name})"));
        } else {
            out.names.push(k.clone());
        }
        if !reserved_env(&k) {
            out.env.push((k, v));
        }
    }
    out
}

/// What the agent is told about the project `.env`. Names only.
pub fn project_env_guidance(names: &[String]) -> String {
    if names.is_empty() {
        return String::new();
    }
    format!(
        "\n\n(Chorus: the project's .env is not readable from your sandbox. Its variables are already \
         set in your environment: {}. Use them as environment variables; do not try to read .env \
         or print their values.)\n",
        names.join(", ")
    )
}

/// One line for the agent when its prompt carries placeholders.
pub const AGENT_GUIDANCE: &str = "\n\n(Chorus: names like $CHORUS_SECRET_GITHUB_1 stand for \
secrets the user shared; their real values are set as environment variables in your shell. \
Use them as $NAME in commands — to put one in a file, write it from the shell, e.g. \
printf 'KEY=%s\\n' \"$CHORUS_SECRET_X\" >> .env — and never print, echo or ask for a value.)\n";

#[cfg(test)]
mod tests {
    use super::*;

    // Built at runtime: a literal in the source would look like a leaked key
    // to GitHub push protection and to any scanner, which is the point.
    fn fake(prefix: &str, n: usize) -> String {
        let body: String = "a1B2c3D4e5F6g7H8i9J0".chars().cycle().take(n).collect();
        format!("{prefix}{body}")
    }

    #[test]
    fn finds_provider_keys_by_shape() {
        for (key, kind) in [
            (fake("sk-ant-api03-", 40), "ANTHROPIC"),
            (fake("sk-proj-", 40), "OPENAI"),
            (fake("ghp_", 36), "GITHUB"),
            (fake("github_pat_", 40), "GITHUB"),
            (fake("xoxb-", 30), "SLACK"),
            (fake("AIza", 35), "GOOGLE"),
            (fake("sk_live_", 24), "STRIPE"),
            (fake("glpat-", 20), "GITLAB"),
            (format!("AKIA{}", "ABCDEFGH23456789"), "AWS"),
        ] {
            let text = format!("use {key} please");
            let f = find(&text);
            assert_eq!(f.len(), 1, "{key}");
            assert_eq!(&text[f[0].start..f[0].end], key);
            assert_eq!(f[0].kind, kind, "{key}");
        }
    }

    #[test]
    fn ordinary_words_are_not_keys() {
        for t in [
            "pip install sk-learn",
            "the ghp_ prefix",
            "AKIA is a prefix",
            "token: <your token here>",
            "PASSWORD=changeme",
            "api_key = os.environ['X']",
            "AUTH_URL=",
            "password: ${DB_PASSWORD}",
            "Author: PerthTheIamCat",
            "Auth: completed",
            "Token: expired",
            "PASSED: all",
        ] {
            assert!(find(t).is_empty(), "{t}: {:?}", find(t));
        }
    }

    #[test]
    fn assignments_and_blocks() {
        let text = "DB_PASSWORD=hunter2hunter2\nexport STRIPE_SECRET=\"abc123def456\"\n  \"client_secret\": \"q9w8e7r6t5y4\",\nNAME=chorus\n";
        let found = find(text);
        let got: Vec<(&str, &str)> = found
            .iter()
            .map(|f| (&text[f.start..f.end], f.kind.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("hunter2hunter2", "DB_PASSWORD"),
                ("abc123def456", "STRIPE_SECRET"),
                ("q9w8e7r6t5y4", "CLIENT_SECRET")
            ]
        );

        let pem = format!("key:\n-----BEGIN OPENSSH PRIVATE KEY-----\n{}\n-----END OPENSSH PRIVATE KEY-----\nafter", "A".repeat(64));
        let f = find(&pem);
        assert_eq!(f.len(), 1);
        assert!(
            pem[f[0].start..f[0].end].starts_with("-----BEGIN")
                && pem[f[0].start..f[0].end].ends_with("-----")
        );
        assert!(pem[f[0].end..].starts_with("\nafter"));

        let jwt = format!(
            "Bearer eyJ{}.{}.{}",
            "h".repeat(20),
            "p".repeat(30),
            "s".repeat(25)
        );
        assert_eq!(find(&jwt)[0].kind, "JWT");
    }

    #[test]
    fn mask_is_stable_and_resolves_back() {
        let v = Vault::in_memory();
        let key = fake("ghp_", 36);
        let a = v.mask(&format!("push with {key}"));
        assert_eq!(a.text, "push with $CHORUS_SECRET_GITHUB_1");
        assert_eq!(a.unstored, 0);
        // Same value, same name; a different one gets the next number.
        let b = v.mask(&format!("again {key} and {}", fake("ghp_", 40)));
        assert_eq!(
            b.text,
            "again $CHORUS_SECRET_GITHUB_1 and $CHORUS_SECRET_GITHUB_2"
        );
        assert_eq!(
            referenced(&b.text),
            ["CHORUS_SECRET_GITHUB_1", "CHORUS_SECRET_GITHUB_2"]
        );
        assert_eq!(
            v.resolve(&referenced(&b.text))[0],
            ("CHORUS_SECRET_GITHUB_1".to_string(), key.clone())
        );
        // An agent echoing the value back in prose is masked too.
        assert_eq!(
            v.mask(&format!("token is {key}.")).text,
            "token is $CHORUS_SECRET_GITHUB_1."
        );
        // The index never holds a value.
        let listed = serde_json::to_string(&v.list()).unwrap();
        assert!(!listed.contains(&key));
    }

    #[test]
    fn values_without_a_pattern_are_masked_once_known() {
        let v = Vault::in_memory();
        // A .env value (a URL with a password) that no shape matches.
        let url = "postgres://app:s3cr3t@db.internal/app";
        let (name, _) = v.name_for("DATABASE_URL", url);
        assert_eq!(name, "CHORUS_SECRET_DATABASE_URL_1");
        assert_eq!(
            v.mask(&format!("conn {url} failed")).text,
            format!("conn ${name} failed")
        );
    }

    #[test]
    fn forget_removes_value_and_name() {
        let v = Vault::in_memory();
        let m = v.mask(&fake("sk-proj-", 40));
        let name = m.names[0].clone();
        v.forget(&name).unwrap();
        assert!(v.list().is_empty());
        assert!(v.resolve(&[name]).is_empty());
    }

    #[test]
    fn dotenv_parses_like_dotenv() {
        let env = "# c\nexport A=1\nB=\"two words\"\nC='x=y'\nD=plain # comment\n1BAD=no\nE\n";
        assert_eq!(
            parse_dotenv(env),
            [("A", "1"), ("B", "two words"), ("C", "x=y"), ("D", "plain")]
                .map(|(k, v)| (k.to_string(), v.to_string()))
        );
    }

    #[test]
    fn project_env_hides_secrets_but_keeps_plain_settings() {
        let d = tempfile::tempdir().unwrap();
        let key = fake("sk-proj-", 40);
        std::fs::write(
            d.path().join(".env"),
            format!("PORT=3000\nOPENAI_API_KEY={key}\nDB_PASSWORD=hunter2hunter2\nSECRET_MODE=production\n"),
        )
        .unwrap();
        std::fs::write(d.path().join(".env.local"), "PORT=4000\n").unwrap();
        let v = Vault::in_memory();
        let pe = project_env(d.path(), &v);
        let get = |k: &str| pe.env.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("PORT"), Some("4000"), ".env.local wins");
        assert_eq!(get("DB_PASSWORD"), Some("hunter2hunter2"));
        // Reserved: would change how the bridge itself authenticates.
        assert_eq!(get("OPENAI_API_KEY"), None);
        assert_eq!(get("CHORUS_SECRET_OPENAI_API_KEY_1"), Some(key.as_str()));
        let told = project_env_guidance(&pe.names);
        assert!(told.contains("PORT") && told.contains("$CHORUS_SECRET_DB_PASSWORD_1"));
        assert!(!told.contains(&key) && !told.contains("hunter2"));
        assert_eq!(v.mask("deploy to production").text, "deploy to production");
        // And an agent echoing a value back gets it masked.
        assert_eq!(
            v.mask("pw hunter2hunter2").text,
            "pw $CHORUS_SECRET_DB_PASSWORD_1"
        );
    }

    struct Refuses;
    impl Backend for Refuses {
        fn put(&self, _: &str, _: &str) -> Result<(), String> {
            Err("locked".into())
        }
        fn get(&self, _: &str) -> Result<Option<String>, String> {
            Ok(None)
        }
        fn delete(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn a_locked_keychain_still_masks_and_still_injects_this_run() {
        let v = Vault::new(Box::new(Refuses), None);
        let key = fake("ghp_", 36);
        let m = v.mask(&key);
        assert_eq!(
            (m.text.as_str(), m.unstored),
            ("$CHORUS_SECRET_GITHUB_1", 1)
        );
        assert_eq!(v.resolve(&m.names), [(m.names[0].clone(), key)]);
    }

    #[test]
    fn hex_round_trips_anything() {
        for s in ["", "abc", "ไทย\n\"quote\" $x", "-----BEGIN\nKEY"] {
            assert_eq!(unhex(&hex(s)).as_deref(), Some(s));
        }
    }

    /// The real Keychain, in a throwaway keychain file so CI and the user's
    /// login keychain are never touched.
    #[cfg(target_os = "macos")]
    #[test]
    fn keychain_backend_round_trips_through_security() {
        let dir = tempfile::tempdir().unwrap();
        let kc = dir.path().join("t.keychain-db");
        let run = |args: &[&str]| {
            std::process::Command::new("/usr/bin/security")
                .args(args)
                .status()
                .unwrap()
        };
        assert!(run(&["create-keychain", "-p", "pw", kc.to_str().unwrap()]).success());
        assert!(run(&["unlock-keychain", "-p", "pw", kc.to_str().unwrap()]).success());
        let k = Keychain {
            keychain: Some(kc.clone()),
        };
        let value = format!("{}\nwith \"quotes\" and ไทย", fake("ghp_", 36));
        k.put("CHORUS_SECRET_TEST_1", &value).unwrap();
        assert_eq!(
            k.get("CHORUS_SECRET_TEST_1").unwrap().as_deref(),
            Some(value.as_str())
        );
        k.put("CHORUS_SECRET_TEST_1", "replaced").unwrap();
        assert_eq!(
            k.get("CHORUS_SECRET_TEST_1").unwrap().as_deref(),
            Some("replaced")
        );
        k.delete("CHORUS_SECRET_TEST_1").unwrap();
        assert_eq!(k.get("CHORUS_SECRET_TEST_1").unwrap(), None);
        let _ = run(&["delete-keychain", kc.to_str().unwrap()]);
    }
}
