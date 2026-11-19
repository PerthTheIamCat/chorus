//! `chorus_offer_variants`: an agent has made two to four versions of
//! something visual (screens, logos, layouts) and lets the person pick.
//!
//! Each variant is an image file inside the project or a dev-server URL on
//! this machine. The shell draws a choice card where the work stopped, with
//! pin comments on the images. What comes back — the pick and the comments —
//! is the tool result, so it lands in the ledger and the next agent sees it.
//!
//! The agent gets text only. The images are its own output, so sending one
//! back would cost tokens for nothing, and `promptCapabilities.image` (which
//! gates images *into* a prompt) never has to be consulted.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::agent_control::db_key;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Image,
    Url,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Variant {
    pub label: String,
    pub kind: Kind,
    /// Absolute path of the image (inside the project) or the `http(s)` URL.
    pub target: String,
    pub note: String,
}

/// What the person is shown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Offer {
    pub agent: String,
    pub title: String,
    pub variants: Vec<Variant>,
    pub recommended: Option<usize>,
}

/// A pin on a variant. `x`/`y` are 0…1 across the image; absent for a URL
/// variant or a comment on the whole thing.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Comment {
    pub variant: usize,
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
    pub text: String,
}

/// The person's answer.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Choice {
    /// Index of the picked variant; `None` = none of them.
    #[serde(default)]
    pub chosen: Option<usize>,
    #[serde(default)]
    pub comments: Vec<Comment>,
    /// Free text next to the pick ("none of these, try warmer colours").
    #[serde(default)]
    pub note: String,
}

/// Blocks until the person picks; `None` = dismissed. Called off the UI thread.
pub type Asker = Arc<dyn Fn(&Offer) -> Option<Choice> + Send + Sync>;

#[derive(Clone)]
struct Registered {
    asker: Asker,
    roots: Vec<PathBuf>,
}

static ASKERS: Mutex<Option<HashMap<(String, i64), Registered>>> = Mutex::new(None);

pub const MIN_VARIANTS: usize = 2;
pub const MAX_VARIANTS: usize = 4;
const MAX_TITLE: usize = 200;
const MAX_LABEL: usize = 80;
const MAX_NOTE: usize = 300;
const MAX_COMMENTS: usize = 20;
const MAX_COMMENT: usize = 500;
const IMAGE_EXT: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "heic", "svg"];

/// Who answers for `sess`, and the folders an image may live in (the working
/// directory first); `None` removes them.
pub fn set_asker(conn: &Connection, sess: i64, roots: Vec<PathBuf>, asker: Option<Asker>) {
    let mut guard = ASKERS.lock().unwrap_or_else(|p| p.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    let key = (db_key(conn), sess);
    match asker {
        Some(asker) => {
            map.insert(key, Registered { asker, roots });
        }
        None => {
            map.remove(&key);
        }
    }
}

/// The project's folders changed; keep the registration, update where images may be.
pub fn set_roots(conn: &Connection, sess: i64, roots: Vec<PathBuf>) {
    let mut guard = ASKERS.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(r) = guard
        .as_mut()
        .and_then(|m| m.get_mut(&(db_key(conn), sess)))
    {
        r.roots = roots;
    }
}

pub fn available(conn: &Connection, sess: i64) -> bool {
    registered(conn, sess).is_some()
}

fn registered(conn: &Connection, sess: i64) -> Option<Registered> {
    let guard = ASKERS.lock().unwrap_or_else(|p| p.into_inner());
    guard.as_ref()?.get(&(db_key(conn), sess)).cloned()
}

pub fn tool_schema() -> Value {
    json!({
        "name": "chorus_offer_variants",
        "description": "Offer the user two to four versions of something visual and let them \
choose (screens, logos, layouts, colour schemes). Each variant is an image file inside the \
project (give its path) or a URL of a dev server running on this machine (http://localhost…). \
The user sees them side by side, may pin comments on the images, and picks one or none. You \
get the choice and the comments back as text. Use it when taste decides, not when a requirement \
does; put your own pick in `recommended`.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "What is being chosen, one line" },
                "variants": {
                    "type": "array",
                    "minItems": MIN_VARIANTS,
                    "maxItems": MAX_VARIANTS,
                    "items": {
                        "type": "object",
                        "properties": {
                            "label": { "type": "string", "description": "Short name, e.g. 'Dense table'" },
                            "image": { "type": "string", "description": "Path of an image file in the project" },
                            "url": { "type": "string", "description": "http://localhost… URL of a running dev server" },
                            "note": { "type": "string", "description": "One line on what is different" }
                        },
                        "required": ["label"],
                        "additionalProperties": false
                    }
                },
                "recommended": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Index (from 0) of the variant you would pick"
                }
            },
            "required": ["title", "variants"],
            "additionalProperties": false
        }
    })
}

/// A URL the user's own machine serves: `http(s)` on a loopback host.
fn local_url(u: &str) -> bool {
    let Some(rest) = u
        .strip_prefix("http://")
        .or_else(|| u.strip_prefix("https://"))
    else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains('@') {
        return false;
    }
    let host = if let Some(v6) = authority.strip_prefix('[') {
        v6.split(']').next().unwrap_or("")
    } else {
        authority.split(':').next().unwrap_or("")
    };
    host == "localhost" || host == "127.0.0.1" || host == "::1" || host.ends_with(".localhost")
}

/// `p` (absolute, or relative to the first root) as a canonical path that
/// exists, is an image, and sits inside one of `roots`.
fn image_path(p: &str, roots: &[PathBuf]) -> Result<PathBuf, String> {
    let first = roots.first().ok_or("no project folder to look in")?;
    let raw = Path::new(p);
    let joined = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        first.join(raw)
    };
    let canon = joined
        .canonicalize()
        .map_err(|_| format!("image not found: {p}"))?;
    let ext_ok = canon
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMAGE_EXT.contains(&e.to_ascii_lowercase().as_str()));
    if !ext_ok || !canon.is_file() {
        return Err(format!("{p} is not an image file"));
    }
    let inside = roots.iter().any(|r| {
        r.canonicalize()
            .map(|r| canon.starts_with(r))
            .unwrap_or(false)
    });
    if !inside {
        return Err(format!("{p} is outside the project folders"));
    }
    Ok(canon)
}

fn trimmed(v: &Value, key: &str, max: usize) -> Result<String, String> {
    match v.get(key).filter(|x| !x.is_null()) {
        None => Ok(String::new()),
        Some(x) => {
            let s = x.as_str().ok_or(format!("{key} must be a string"))?.trim();
            if s.chars().count() > max {
                return Err(format!("{key} must be at most {max} characters"));
            }
            Ok(s.to_string())
        }
    }
}

/// Validate the arguments into what the person will see.
pub fn parse(args: &Value, roots: &[PathBuf]) -> Result<Offer, String> {
    let title = trimmed(args, "title", MAX_TITLE)?;
    if title.is_empty() {
        return Err("title is required".into());
    }
    let arr = args
        .get("variants")
        .and_then(Value::as_array)
        .ok_or("variants must be an array")?;
    if !(MIN_VARIANTS..=MAX_VARIANTS).contains(&arr.len()) {
        return Err(format!("offer {MIN_VARIANTS} to {MAX_VARIANTS} variants"));
    }
    let mut variants = Vec::new();
    for (i, v) in arr.iter().enumerate() {
        let label = trimmed(v, "label", MAX_LABEL)?;
        if label.is_empty() {
            return Err(format!("variant {i}: label is required"));
        }
        let note = trimmed(v, "note", MAX_NOTE)?;
        let image = trimmed(v, "image", 1_024)?;
        let url = trimmed(v, "url", 2_048)?;
        let (kind, target) = match (image.is_empty(), url.is_empty()) {
            (false, true) => (
                Kind::Image,
                image_path(&image, roots)
                    .map_err(|e| format!("variant {i}: {e}"))?
                    .to_string_lossy()
                    .into_owned(),
            ),
            (true, false) => {
                if !local_url(&url) {
                    return Err(format!(
                        "variant {i}: url must be http(s) on localhost / 127.0.0.1"
                    ));
                }
                (Kind::Url, url)
            }
            _ => return Err(format!("variant {i}: give exactly one of image or url")),
        };
        variants.push(Variant {
            label,
            kind,
            target,
            note,
        });
    }
    let recommended = match args.get("recommended").filter(|v| !v.is_null()) {
        None => None,
        Some(v) => {
            let i = v
                .as_u64()
                .ok_or("recommended must be a non-negative integer")? as usize;
            if i >= variants.len() {
                return Err("recommended must be the index of one of the variants".into());
            }
            Some(i)
        }
    };
    Ok(Offer {
        agent: String::new(),
        title,
        variants,
        recommended,
    })
}

/// The choice as the text the agent (and the ledger) get. Free text from the
/// person is masked like any message: it may carry a pasted key.
pub fn render_choice(offer: &Offer, choice: &Choice) -> String {
    let vault = crate::secrets::user_vault();
    let mask = |s: &str| vault.mask(s).text;
    let label = |i: usize| {
        offer
            .variants
            .get(i)
            .map_or_else(|| format!("variant {}", i + 1), |v| format!("{} (variant {})", v.label, i + 1))
    };
    let mut out = match choice.chosen.filter(|&i| i < offer.variants.len()) {
        Some(i) => format!("User chose {}.", label(i)),
        None => "User chose none of the variants.".to_string(),
    };
    let note: String = choice.note.trim().chars().take(MAX_COMMENT).collect();
    if !note.is_empty() {
        out.push_str(&format!(" Note: {}", mask(&note)));
    }
    let mut n = 0;
    for c in &choice.comments {
        let text: String = c.text.trim().chars().take(MAX_COMMENT).collect();
        if text.is_empty() || c.variant >= offer.variants.len() {
            continue;
        }
        if n == 0 {
            out.push_str("\nComments:");
        }
        if n == MAX_COMMENTS {
            out.push_str("\n- (more comments cut)");
            break;
        }
        n += 1;
        let at = match (c.x, c.y) {
            (Some(x), Some(y)) => format!(
                " at {}% across, {}% down",
                (x.clamp(0.0, 1.0) * 100.0).round(),
                (y.clamp(0.0, 1.0) * 100.0).round()
            ),
            _ => String::new(),
        };
        out.push_str(&format!("\n- on {}{at}: {}", label(c.variant), mask(&text)));
    }
    out
}

/// Handle one `tools/call`.
pub fn call(conn: &Connection, sess: i64, agent: &str, args: &Value) -> Result<Value, String> {
    let reg = registered(conn, sess)
        .ok_or("no one is available to choose right now; pick one yourself and say why")?;
    let mut offer = parse(args, &reg.roots)?;
    offer.agent = agent.to_string();
    let text = match (reg.asker)(&offer) {
        Some(choice) => render_choice(&offer, &choice),
        None => "The user closed the choice without picking. Do not offer again; pick the \
                 recommended one (or your best) and say what you assumed."
            .to_string(),
    };
    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> (tempfile::TempDir, Vec<PathBuf>) {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.png"), b"x").unwrap();
        std::fs::write(d.path().join("b.png"), b"x").unwrap();
        std::fs::write(d.path().join("notes.txt"), b"x").unwrap();
        let root = d.path().canonicalize().unwrap();
        (d, vec![root])
    }

    fn two(a: Value, b: Value) -> Value {
        json!({"title":"Pick a login","variants":[a, b]})
    }

    #[test]
    fn image_and_local_url_variants_parse() {
        let (_d, roots) = project();
        let o = parse(
            &two(
                json!({"label":"A","image":"a.png","note":"dense"}),
                json!({"label":"B","url":"http://localhost:5173/login"}),
            ),
            &roots,
        )
        .unwrap();
        assert_eq!(o.variants[0].kind, Kind::Image);
        assert!(o.variants[0].target.ends_with("a.png"));
        assert!(Path::new(&o.variants[0].target).is_absolute());
        assert_eq!(o.variants[1].kind, Kind::Url);
    }

    #[test]
    fn paths_and_urls_that_leave_the_project_are_refused() {
        let (_d, roots) = project();
        let other = tempfile::tempdir().unwrap();
        std::fs::write(other.path().join("x.png"), b"x").unwrap();
        let ok = json!({"label":"A","image":"a.png"});
        for bad in [
            json!({"label":"B","image": other.path().join("x.png").to_string_lossy()}),
            json!({"label":"B","image":"../../etc/passwd"}),
            json!({"label":"B","image":"notes.txt"}),
            json!({"label":"B","image":"missing.png"}),
            json!({"label":"B","url":"https://example.com"}),
            json!({"label":"B","url":"http://localhost@evil.com/"}),
            json!({"label":"B","url":"file:///etc/passwd"}),
            json!({"label":"B","image":"a.png","url":"http://localhost:1"}),
            json!({"label":"B"}),
        ] {
            assert!(parse(&two(ok.clone(), bad.clone()), &roots).is_err(), "{bad}");
        }
        for good in ["http://127.0.0.1:3000", "http://[::1]:8080/x", "https://app.localhost/"] {
            assert!(local_url(good), "{good}");
        }
    }

    #[test]
    fn two_to_four_variants_and_a_recommendation_that_exists() {
        let (_d, roots) = project();
        let v = |l: &str| json!({"label":l,"image":"a.png"});
        assert!(parse(&json!({"title":"t","variants":[v("A")]}), &roots).is_err());
        assert!(parse(&json!({"title":"t","variants":[v("A"),v("B"),v("C"),v("D"),v("E")]}), &roots).is_err());
        assert!(parse(&json!({"title":"t","variants":[v("A"),v("B")],"recommended":2}), &roots).is_err());
        let o = parse(&json!({"title":"t","variants":[v("A"),v("B")],"recommended":1}), &roots).unwrap();
        assert_eq!(o.recommended, Some(1));
        assert!(parse(&json!({"variants":[v("A"),v("B")]}), &roots).is_err(), "title is required");
    }

    #[test]
    fn the_choice_and_pin_comments_go_back_as_text() {
        std::env::set_var("CHORUS_SECRETS_BACKEND", "memory");
        let (_d, roots) = project();
        let o = parse(&two(json!({"label":"Dense","image":"a.png"}), json!({"label":"Airy","image":"b.png"})), &roots).unwrap();
        let c = Choice {
            chosen: Some(1),
            comments: vec![
                Comment { variant: 0, x: Some(0.424), y: Some(0.3), text: " button too small ".into() },
                Comment { variant: 9, x: None, y: None, text: "ignored: no such variant".into() },
                Comment { variant: 1, x: None, y: None, text: "   ".into() },
            ],
            note: "warmer please".into(),
        };
        let t = render_choice(&o, &c);
        assert!(t.starts_with("User chose Airy (variant 2)."), "{t}");
        assert!(t.contains("Note: warmer please"));
        assert!(t.contains("- on Dense (variant 1) at 42% across, 30% down: button too small"), "{t}");
        assert!(!t.contains("no such variant"));
        let none = render_choice(&o, &Choice { chosen: None, comments: vec![], note: String::new() });
        assert_eq!(none, "User chose none of the variants.");
    }

    #[test]
    fn call_needs_an_asker_and_survives_dismissal() {
        let (_d, roots) = project();
        let conn = Connection::open_in_memory().unwrap();
        let args = two(json!({"label":"A","image":"a.png"}), json!({"label":"B","image":"b.png"}));
        assert!(call(&conn, 71, "claude", &args).is_err());
        set_asker(&conn, 71, roots.clone(), Some(Arc::new(|_| None)));
        assert!(available(&conn, 71));
        let r = call(&conn, 71, "claude", &args).unwrap();
        assert!(r["content"][0]["text"].as_str().unwrap().contains("without picking"));
        set_asker(
            &conn,
            71,
            roots,
            Some(Arc::new(|o: &Offer| {
                assert_eq!(o.agent, "claude");
                Some(Choice { chosen: Some(0), comments: vec![], note: String::new() })
            })),
        );
        let r = call(&conn, 71, "claude", &args).unwrap();
        assert!(r["content"][0]["text"].as_str().unwrap().starts_with("User chose A"));
        set_asker(&conn, 71, vec![], None);
        assert!(!available(&conn, 71));
    }
}
