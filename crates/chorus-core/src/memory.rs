//! Durable facts (compaction rung L3, see `types::Rung`) — short, pinned or
//! frequently-used notes that survive far longer than raw events.

use crate::embedding::{self, Embedder};
use crate::{ChorusError, Result};
use rusqlite::{Connection, OptionalExtension};
use std::time::{SystemTime, UNIX_EPOCH};

/// Facts must stay short: the model never uses id/confidence/timestamp
/// metadata, and carrying it costs roughly 70% of a fact's tokens for
/// nothing. Longer text also defeats the point of a "durable fact" rung.
const MAX_FACT_LEN: usize = 140;

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone)]
pub struct Fact {
    pub id: i64,
    pub scope: i64,
    pub text: String,
    pub pinned: bool,
    pub hits: i64,
}

/// Add (or, on a duplicate `(scope, text)`, no-op onto) a fact. Rejects text
/// over 140 characters.
pub fn add(c: &Connection, scope: i64, text: &str) -> Result<i64> {
    if text.chars().count() > MAX_FACT_LEN {
        return Err(ChorusError::Invalid(format!(
            "fact text is {} chars, max is {MAX_FACT_LEN}",
            text.chars().count()
        )));
    }

    c.execute(
        "INSERT INTO fact(scope, text, conf, pinned, hits, created_ms)
         VALUES (?1, ?2, 1.0, 0, 0, ?3)
         ON CONFLICT(scope, text) DO UPDATE SET scope = excluded.scope",
        (scope, text, now_ms()),
    )?;

    let id: i64 = c.query_row(
        "SELECT id FROM fact WHERE scope = ?1 AND text = ?2",
        (scope, text),
        |r| r.get(0),
    )?;
    Ok(id)
}

/// The scopes a read of `scope` covers. Project (1) is what every reader asks
/// for, and it must also see global facts (0) and legacy session-scope rows (2:
/// a session has no id to scope by, so the scope was never meaningful) —
/// otherwise a fact an agent proposed with scope 0 or 2 is approved and then
/// never shown to anyone. Any other scope reads just itself.
fn read_scopes(scope: i64) -> (i64, i64, i64) {
    if scope == 1 {
        (0, 1, 2)
    } else {
        (scope, scope, scope)
    }
}

/// Drop later facts whose text equals an earlier one's: the same sentence can
/// exist in two scopes that one read now covers. Keeps the first (best-ranked).
fn dedup_text(facts: Vec<Fact>) -> Vec<Fact> {
    let mut seen = std::collections::HashSet::new();
    facts.into_iter().filter(|f| seen.insert(f.text.clone())).collect()
}

/// [`top`]'s ranking without the bookkeeping: read-only.
fn top_rows(c: &Connection, scope: i64, k: usize) -> Result<Vec<Fact>> {
    let mut stmt = c.prepare(
        "SELECT id, scope, text, pinned, hits FROM fact
         WHERE scope IN (?1, ?2, ?3)
         ORDER BY pinned DESC, hits DESC, created_ms DESC",
    )?;
    let (s0, s1, s2) = read_scopes(scope);
    let facts: Vec<Fact> = stmt
        .query_map((s0, s1, s2), |r| {
            Ok(Fact {
                id: r.get(0)?,
                scope: r.get(1)?,
                text: r.get(2)?,
                pinned: r.get::<_, i64>(3)? != 0,
                hits: r.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut facts = dedup_text(facts);
    facts.truncate(k);
    Ok(facts)
}

/// The top `k` facts for a scope, ordered pinned-first, then by hits, then
/// most-recently-created. Bumps `hits`/`last_used_ms` for every fact
/// returned.
pub fn top(c: &Connection, scope: i64, k: usize) -> Result<Vec<Fact>> {
    let facts = top_rows(c, scope, k)?;
    let now = now_ms();
    for f in &facts {
        c.execute(
            "UPDATE fact SET hits = hits + 1, last_used_ms = ?1 WHERE id = ?2",
            (now, f.id),
        )?;
    }

    // Reflect the bump in the returned values too.
    let facts = facts
        .into_iter()
        .map(|mut f| {
            f.hits += 1;
            f
        })
        .collect();

    Ok(facts)
}

/// Dimension of the hashed n-gram embedding below.
const EMBED_DIM: usize = 256;

/// A dependency-free text embedding: hashed character 2- and 3-grams, counted
/// and L2-normalised. It needs no model, runs in microseconds, and — because
/// it works on characters, not words — handles Thai (no spaces) and mixed
/// Thai/English the same way. It is *lexical* similarity, not meaning: "login"
/// finds "logging in" but not "authentication". A real embedding model can
/// replace this behind the same [`relevant`] call.
pub fn embed(text: &str) -> Vec<f32> {
    let chars: Vec<char> = text.to_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
    let mut v = vec![0f32; EMBED_DIM];
    for n in [2usize, 3] {
        for w in chars.windows(n) {
            let mut h: u64 = 0xcbf2_9ce4_8422_2325;
            for c in w {
                h = (h ^ *c as u64).wrapping_mul(0x100_0000_01b3);
            }
            v[(h % EMBED_DIM as u64) as usize] += 1.0;
        }
    }
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        v.iter_mut().for_each(|x| *x /= norm);
    }
    v
}

/// Facts for `scope` ranked by relevance to `query`: pinned facts always
/// come first, then the best matches above a similarity floor. Facts below
/// the floor are left out even when there is room (R2). An empty query is the
/// same as [`top`].
pub fn relevant(c: &Connection, scope: i64, query: &str, k: usize) -> Result<Vec<Fact>> {
    relevant_with(c, scope, query, k, None)
}

/// Cosine at or above which a new fact is the same fact as an existing one
/// (storage spec §6) and is merged into it rather than stored twice.
pub const DEDUP_COSINE: f32 = 0.92;

/// Lowest cosine a model embedding needs to count as "about the query".
const SEMANTIC_FLOOR: f32 = 0.30;

/// [`relevant`], with vector retrieval when `embedder` can embed the query and
/// at least one fact has a stored vector from the same model: those facts are
/// ranked by cosine (meaning, not just shared characters), pinned first and the
/// usual order filling what is left. Anything missing — no embedder, Ollama
/// down, no vectors yet — silently uses the n-gram ranking, so this never
/// fails a turn.
pub fn relevant_with(
    c: &Connection,
    scope: i64,
    query: &str,
    k: usize,
    embedder: Option<&dyn Embedder>,
) -> Result<Vec<Fact>> {
    let mut picked: Vec<Fact> = ranked_with(c, scope, query, k, embedder)?
        .into_iter()
        .map(|(f, _)| f)
        .collect();
    let now = now_ms();
    for f in &mut picked {
        c.execute("UPDATE fact SET hits = hits + 1, last_used_ms = ?1 WHERE id = ?2", (now, f.id))?;
        f.hits += 1;
    }
    Ok(picked)
}

/// [`relevant_with`]'s ranking with each fact's similarity to `query`, and
/// **no writes** — no `hits` bump. This is what the context inspector reads,
/// so looking at what the agent saw cannot change what it sees next. The
/// score is `None` when there was no query to score against (the plain
/// most-used order); pinned facts keep their computed score but are listed
/// first regardless of it.
pub fn ranked_with(
    c: &Connection,
    scope: i64,
    query: &str,
    k: usize,
    embedder: Option<&dyn Embedder>,
) -> Result<Vec<(Fact, Option<f32>)>> {
    if query.trim().is_empty() || k == 0 {
        return Ok(top_rows(c, scope, k)?.into_iter().map(|f| (f, None)).collect());
    }
    let mut stmt = c.prepare(
        "SELECT id, scope, text, pinned, hits FROM fact WHERE scope IN (?1, ?2, ?3)
         ORDER BY pinned DESC, hits DESC, created_ms DESC",
    )?;
    let (s0, s1, s2) = read_scopes(scope);
    let all: Vec<Fact> = stmt
        .query_map([s0, s1, s2], |r| {
            Ok(Fact {
                id: r.get(0)?,
                scope: r.get(1)?,
                text: r.get(2)?,
                pinned: r.get::<_, i64>(3)? != 0,
                hits: r.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let all = dedup_text(all);

    let semantic = embedder.and_then(|e| {
        let stored = stored_vectors(c, scope, &e.id()).ok().filter(|m| !m.is_empty())?;
        let q = e.embed(query)?;
        Some((q, stored))
    });
    let (floor, scored): (f32, Vec<(f32, Fact)>) = match &semantic {
        Some((q, stored)) => (
            SEMANTIC_FLOOR,
            all.into_iter()
                .map(|f| {
                    let s = stored.get(&f.id).map_or(0.0, |v| crate::VectorOps::cosine_similarity(q, v));
                    (s, f)
                })
                .collect(),
        ),
        None => {
            let q = embed(query);
            (
                lexical_floor(),
                all.into_iter()
                    .map(|f| (crate::VectorOps::cosine_similarity(&q, &embed(&f.text)), f))
                    .collect(),
            )
        }
    };
    let mut scored = scored;
    let mut picked: Vec<(Fact, Option<f32>)> = Vec::new();
    // Pinned first (already in hits order), then relevance, then the old order.
    for (s, f) in scored.iter().filter(|(_, f)| f.pinned) {
        picked.push((f.clone(), Some(*s)));
    }
    let mut hits: Vec<&(f32, Fact)> = scored.iter().filter(|(s, f)| !f.pinned && *s >= floor).collect();
    hits.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    for (s, f) in hits {
        picked.push((f.clone(), Some(*s)));
    }
    // With something to match against, a fact below the relevance floor is
    // not worth its tokens however much budget is left (R2). The old behaviour
    // — floor 0.18 and topping up with the most-used facts — stays one switch
    // away for comparison: `CHORUS_R2_MIN_RELEVANCE=off`.
    if !r2_on() {
        for (s, f) in scored.drain(..) {
            if !picked.iter().any(|(p, _)| p.id == f.id) {
                picked.push((f, Some(s)));
            }
        }
    }
    picked.truncate(k);
    Ok(picked)
}

/// Lowest hashed-n-gram cosine that counts as "about the query".
///
/// Measured on a six-fact store with the query "how do we deploy a release with
/// actions?": the fact about GitHub Actions scores 0.42, unrelated facts score
/// 0.18–0.30 (shared "th"/"ith"/"ac" bigrams are noise). The old floor of 0.18
/// let all of them in; 0.30 separates them there. Character n-grams are a weak
/// signal on short queries, so `CHORUS_MEMORY_MIN_RELEVANCE` overrides it.
const LEXICAL_FLOOR: f32 = 0.30;

fn lexical_floor() -> f32 {
    if !r2_on() {
        return 0.18;
    }
    std::env::var("CHORUS_MEMORY_MIN_RELEVANCE")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .filter(|v| (0.0..=1.0).contains(v))
        .unwrap_or(LEXICAL_FLOOR)
}

fn r2_on() -> bool {
    !std::env::var("CHORUS_R2_MIN_RELEVANCE").is_ok_and(|v| v == "off")
}

/// Stored vectors for `scope` from `model`, by fact id.
fn stored_vectors(c: &Connection, scope: i64, model: &str) -> Result<std::collections::HashMap<i64, Vec<f32>>> {
    let mut stmt = c.prepare(
        "SELECT e.fact, e.vec FROM fact_embedding e JOIN fact f ON f.id = e.fact
         WHERE f.scope IN (?1, ?2, ?3) AND e.model = ?4",
    )?;
    let (s0, s1, s2) = read_scopes(scope);
    let rows = stmt
        .query_map((s0, s1, s2, model), |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows.into_iter().map(|(id, b)| (id, embedding::decode(&b))).collect())
}

fn store_vector(c: &Connection, fact: i64, model: &str, v: &[f32]) -> Result<()> {
    c.execute(
        "INSERT INTO fact_embedding(fact, model, dim, vec) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(fact) DO UPDATE SET model = excluded.model, dim = excluded.dim, vec = excluded.vec",
        (fact, model, v.len() as i64, embedding::encode(v)),
    )?;
    Ok(())
}

/// What [`add_with`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Added {
    /// The fact now holding this text — the existing one when merged.
    pub id: i64,
    /// An existing fact was close enough (cosine ≥ [`DEDUP_COSINE`]) that no
    /// new row was made.
    pub merged: bool,
}

/// [`add`], plus — when `embedder` is available — near-duplicate merging and
/// storing the new fact's vector. The wording of the first-remembered fact
/// wins; the near-duplicate is dropped, not stored alongside.
pub fn add_with(c: &Connection, scope: i64, text: &str, embedder: Option<&dyn Embedder>) -> Result<Added> {
    let vector = embedder.and_then(|e| e.embed(text).map(|v| (e.id(), v)));
    if let Some((model, v)) = &vector {
        if text.chars().count() <= MAX_FACT_LEN {
            let stored = stored_vectors(c, scope, model)?;
            let best = stored
                .iter()
                .map(|(id, sv)| (*id, crate::VectorOps::cosine_similarity(v, sv)))
                .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            if let Some((id, sim)) = best {
                if sim >= DEDUP_COSINE {
                    return Ok(Added { id, merged: true });
                }
            }
        }
    }
    let id = add(c, scope, text)?;
    if let Some((model, v)) = vector {
        store_vector(c, id, &model, &v)?;
    }
    Ok(Added { id, merged: false })
}

/// Embed the facts that have no vector from `embedder`'s model yet (oldest
/// first, at most `limit`). Returns how many were stored; stops at the first
/// failure, since the next would fail too.
pub fn backfill(c: &Connection, embedder: &dyn Embedder, limit: usize) -> Result<usize> {
    let model = embedder.id();
    let mut stmt = c.prepare(
        "SELECT f.id, f.text FROM fact f
         LEFT JOIN fact_embedding e ON e.fact = f.id AND e.model = ?1
         WHERE e.fact IS NULL ORDER BY f.id ASC LIMIT ?2",
    )?;
    let todo = stmt
        .query_map((&model, limit as i64), |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut done = 0;
    for (id, text) in todo {
        let Some(v) = embedder.embed(&text) else { break };
        store_vector(c, id, &model, &v)?;
        done += 1;
    }
    Ok(done)
}

/// Render facts as the format sent to the model: plain markdown bullets and
/// nothing else. No ids, confidence, or timestamps — that metadata is never
/// used by the model and stripping it is the whole point.
pub fn render(facts: &[Fact]) -> String {
    facts
        .iter()
        .map(|f| format!("- {}", f.text))
        .collect::<Vec<_>>()
        .join("\n")
}

// ---- proposal queue (design §4: agents never write `fact` directly) -----

/// What a queued entry asks for. Everything waits for a person; the kind is
/// only what the review screen needs to say about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProposalKind {
    /// A fact to remember (what `memory_propose` and `chorus memory add` make).
    Fact,
    /// Drafted from the ledger after the fact (`lessons`): something that went
    /// wrong or was corrected, with the evidence attached.
    Lesson,
    /// "Stop remembering fact `replaces`": injected many times, never used, or
    /// pointing at files that are gone.
    Retire,
}

impl ProposalKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fact => "fact",
            Self::Lesson => "lesson",
            Self::Retire => "retire",
        }
    }
    fn parse(s: &str) -> Self {
        match s {
            "lesson" => Self::Lesson,
            "retire" => Self::Retire,
            _ => Self::Fact,
        }
    }
}

/// One fact an agent wants remembered, awaiting a human's review.
#[derive(Debug, Clone)]
pub struct Proposal {
    pub id: i64,
    pub scope: i64,
    pub text: String,
    pub src: Option<i64>,
    pub kind: ProposalKind,
    /// The existing fact this overwrites (`Fact`/`Lesson`) or deletes (`Retire`).
    pub replaces: Option<i64>,
    /// Where a lesson came from, e.g. `session 3 · seq 41`.
    pub evidence: Option<String>,
    /// Drafted from an imported session or web content ("อาจถูกวางยา"): shown
    /// with a warning and never accepted without a person.
    pub suspicious: bool,
}

/// A queue entry to insert; see [`enqueue`].
#[derive(Debug, Clone)]
pub struct NewProposal<'a> {
    pub scope: i64,
    pub text: &'a str,
    pub src: Option<i64>,
    pub kind: ProposalKind,
    pub replaces: Option<i64>,
    pub evidence: Option<&'a str>,
    pub suspicious: bool,
}

impl<'a> NewProposal<'a> {
    pub fn fact(scope: i64, text: &'a str) -> Self {
        Self { scope, text, src: None, kind: ProposalKind::Fact, replaces: None, evidence: None, suspicious: false }
    }
}

/// Queue a fact for review. Same 140-char limit as [`add`] — a proposal
/// that could never be approved shouldn't sit in the queue.
pub fn propose(c: &Connection, scope: i64, text: &str, src: Option<i64>) -> Result<i64> {
    enqueue(c, &NewProposal { src, ..NewProposal::fact(scope, text) })
}

/// Insert a queue entry exactly as described (no conflict detection).
pub fn enqueue(c: &Connection, p: &NewProposal<'_>) -> Result<i64> {
    if p.text.chars().count() > MAX_FACT_LEN {
        return Err(ChorusError::Invalid(format!(
            "proposed fact is {} chars, max is {MAX_FACT_LEN}",
            p.text.chars().count()
        )));
    }
    c.execute(
        "INSERT INTO fact_proposal(scope, text, src, created_ms, kind, replaces, evidence, suspicious)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        (p.scope, p.text, p.src, now_ms(), p.kind.as_str(), p.replaces, p.evidence, p.suspicious as i64),
    )?;
    Ok(c.last_insert_rowid())
}

/// What [`propose_with`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Proposed {
    /// Queued as a new entry (proposal id).
    Queued(i64),
    /// Queued as "replace fact `fact`": it is about the same thing as an
    /// existing fact but says something different.
    Replaces { proposal: i64, fact: i64 },
    /// Already remembered word for word (fact id), or already waiting in the
    /// queue (proposal id) — nothing was added.
    Known(i64),
    AlreadyQueued(i64),
}

/// Character-n-gram cosine at or above which two *different* sentences are
/// taken to be about the same thing when no embedding model is available.
/// "ใช้ pnpm ไม่ใช่ npm" vs "ใช้ yarn ไม่ใช่ npm" scores ≈0.69, "DB is Postgres 15"
/// vs "16" ≈0.92, and two different commands or unrelated notes stay at or
/// under 0.5 (measured; `tests/memory_learns.rs` pins the cases).
const REPLACE_NGRAM: f32 = 0.55;

/// The existing fact `text` is closest to (if close enough to be "the same
/// subject"), using stored vectors when `embedder` has them and the n-gram
/// embedding otherwise.
fn conflicting_fact(c: &Connection, scope: i64, text: &str, embedder: Option<&dyn Embedder>) -> Result<Option<i64>> {
    let (s0, s1, s2) = read_scopes(scope);
    let mut stmt = c.prepare("SELECT id, text FROM fact WHERE scope IN (?1, ?2, ?3) ORDER BY id")?;
    let facts: Vec<(i64, String)> = stmt
        .query_map([s0, s1, s2], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    if facts.is_empty() {
        return Ok(None);
    }
    if let Some(e) = embedder {
        if let (Ok(stored), Some(q)) = (stored_vectors(c, scope, &e.id()), e.embed(text)) {
            if !stored.is_empty() {
                let best = stored
                    .iter()
                    .map(|(id, v)| (*id, crate::VectorOps::cosine_similarity(&q, v)))
                    .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
                return Ok(best.filter(|(_, s)| *s >= DEDUP_COSINE).map(|(id, _)| id));
            }
        }
    }
    let q = embed(text);
    let best = facts
        .iter()
        .map(|(id, t)| (*id, crate::VectorOps::cosine_similarity(&q, &embed(t))))
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    Ok(best.filter(|(_, s)| *s >= REPLACE_NGRAM).map(|(id, _)| id))
}

/// What an *agent* uses to propose: like [`propose`], but a sentence that
/// conflicts with something already remembered becomes "replace #fact" instead
/// of a second row that contradicts the first, and a sentence already
/// remembered (or already waiting) adds nothing.
pub fn propose_with(
    c: &Connection,
    p: &NewProposal<'_>,
    embedder: Option<&dyn Embedder>,
) -> Result<Proposed> {
    if let Some(id) = c
        .query_row(
            "SELECT id FROM fact WHERE scope IN (0, 1, 2) AND text = ?1 LIMIT 1",
            [p.text],
            |r| r.get(0),
        )
        .optional()?
    {
        return Ok(Proposed::Known(id));
    }
    if let Some(id) = c
        .query_row(
            "SELECT id FROM fact_proposal WHERE text = ?1 AND scope = ?2 LIMIT 1",
            (p.text, p.scope),
            |r| r.get(0),
        )
        .optional()?
    {
        return Ok(Proposed::AlreadyQueued(id));
    }
    let target = if p.replaces.is_some() {
        p.replaces
    } else {
        conflicting_fact(c, p.scope, p.text, embedder)?
    };
    let proposal = enqueue(c, &NewProposal { replaces: target, ..p.clone() })?;
    Ok(match target {
        Some(fact) => Proposed::Replaces { proposal, fact },
        None => Proposed::Queued(proposal),
    })
}

/// Everything awaiting review, oldest first.
pub fn pending(c: &Connection) -> Result<Vec<Proposal>> {
    let mut stmt = c.prepare(
        "SELECT id, scope, text, src, kind, replaces, evidence, suspicious
         FROM fact_proposal ORDER BY id ASC",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Proposal {
                id: r.get(0)?,
                scope: r.get(1)?,
                text: r.get(2)?,
                src: r.get(3)?,
                kind: ProposalKind::parse(&r.get::<_, String>(4)?),
                replaces: r.get(5)?,
                evidence: r.get(6)?,
                suspicious: r.get::<_, i64>(7)? != 0,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

/// How many entries are waiting — the "รอคุณดู N ข้อ" badge.
pub fn pending_count(c: &Connection) -> Result<i64> {
    Ok(c.query_row("SELECT COUNT(*) FROM fact_proposal", [], |r| r.get(0))?)
}

/// Approve proposal `id`: move it into `fact` (via [`add`], so the length
/// check and dedup apply) and drop it from the queue. Returns the new fact
/// id. `Err` if there's no such proposal.
pub fn approve(c: &Connection, id: i64) -> Result<i64> {
    approve_with(c, id, None).map(|a| a.id)
}

/// [`approve`] through [`add_with`]: a proposal that restates an existing fact
/// is merged into it instead of becoming a second one. A "replace #id" entry
/// rewrites that fact in place; a "retire" entry deletes it.
pub fn approve_with(c: &Connection, id: i64, embedder: Option<&dyn Embedder>) -> Result<Added> {
    let row: Option<(i64, String, String, Option<i64>)> = c
        .query_row(
            "SELECT scope, text, kind, replaces FROM fact_proposal WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let (scope, text, kind, replaces) = row.ok_or_else(|| ChorusError::Invalid(format!("no proposal {id}")))?;
    let fact_exists = |fid: i64| -> Result<bool> {
        Ok(c.query_row("SELECT EXISTS(SELECT 1 FROM fact WHERE id = ?1)", [fid], |r| r.get(0))?)
    };
    let added = match (ProposalKind::parse(&kind), replaces) {
        (ProposalKind::Retire, Some(fid)) => {
            c.execute("DELETE FROM fact WHERE id = ?1", [fid])?;
            Added { id: fid, merged: false }
        }
        (ProposalKind::Retire, None) => return Err(ChorusError::Invalid(format!("retire proposal {id} names no fact"))),
        (_, Some(fid)) if fact_exists(fid)? => replace_text(c, fid, &text, embedder)?,
        // The fact it meant to replace is already gone: it is a plain fact now.
        _ => add_with(c, scope, &text, embedder)?,
    };
    c.execute("DELETE FROM fact_proposal WHERE id = ?1", [id])?;
    Ok(added)
}

/// Rewrite fact `id` to say `text`. Its counters start over — they measured
/// the old wording. If another fact already says exactly this, the old one is
/// dropped and that one wins.
fn replace_text(c: &Connection, id: i64, text: &str, embedder: Option<&dyn Embedder>) -> Result<Added> {
    if text.chars().count() > MAX_FACT_LEN {
        return Err(ChorusError::Invalid(format!("fact text is {} chars, max is {MAX_FACT_LEN}", text.chars().count())));
    }
    let scope: i64 = c.query_row("SELECT scope FROM fact WHERE id = ?1", [id], |r| r.get(0))?;
    let same: Option<i64> = c
        .query_row("SELECT id FROM fact WHERE scope = ?1 AND text = ?2 AND id != ?3", (scope, text, id), |r| r.get(0))
        .optional()?;
    if let Some(other) = same {
        c.execute("DELETE FROM fact WHERE id = ?1", [id])?;
        return Ok(Added { id: other, merged: true });
    }
    c.execute(
        "UPDATE fact SET text = ?1, injected = 0, referenced = 0, last_injected_ms = NULL WHERE id = ?2",
        (text, id),
    )?;
    c.execute("DELETE FROM fact_embedding WHERE fact = ?1", [id])?;
    if let Some(e) = embedder {
        if let Some(v) = e.embed(text) {
            store_vector(c, id, &e.id(), &v)?;
        }
    }
    Ok(Added { id, merged: false })
}

/// Drop proposal `id` without remembering it. Idempotent.
pub fn reject(c: &Connection, id: i64) -> Result<()> {
    c.execute("DELETE FROM fact_proposal WHERE id = ?1", [id])?;
    Ok(())
}

// ---- usage counters (schema v9) ---------------------------------------
//
// Written after a turn, never while its context is assembled (rule 7.2b):
// `assemble_context` only remembers which facts it used, and the turn's
// epilogue calls these.

/// These facts were put in front of a model once.
pub fn record_injected(c: &Connection, ids: &[i64]) -> Result<()> {
    let now = now_ms();
    for id in ids {
        c.execute("UPDATE fact SET injected = injected + 1, last_injected_ms = ?1 WHERE id = ?2", (now, id))?;
    }
    Ok(())
}

/// The agent's output touched these facts.
pub fn record_referenced(c: &Connection, ids: &[i64]) -> Result<()> {
    for id in ids {
        c.execute("UPDATE fact SET referenced = referenced + 1 WHERE id = ?1", [id])?;
    }
    Ok(())
}

/// `(id, text)` of the given facts that still exist.
pub fn fact_texts(c: &Connection, ids: &[i64]) -> Result<Vec<(i64, String)>> {
    let mut out = Vec::new();
    for id in ids {
        if let Some(t) = c.query_row("SELECT text FROM fact WHERE id = ?1", [id], |r| r.get::<_, String>(0)).optional()? {
            out.push((*id, t));
        }
    }
    Ok(out)
}

/// Words too common to show that an output "mentions" a fact.
const STOP: &[&str] = &[
    "the", "and", "for", "use", "with", "not", "are", "that", "this", "from", "into", "only",
    "always", "never", "when", "then", "than", "should", "must", "will", "have", "has", "was",
    "all", "any", "but", "you", "your", "our", "can", "via", "per", "its",
];

/// The terms of a fact an output can be checked against: Latin identifiers,
/// commands and paths (`pnpm`, `src/order/repo.ts`, `cargo-test`). Thai has
/// no word boundaries to split on, so a Thai-only fact has none of these and
/// [`mentions`] falls back to character n-grams.
pub fn key_terms(text: &str) -> Vec<String> {
    let mut terms: Vec<String> = Vec::new();
    let mut cur = String::new();
    let flush = |cur: &mut String, terms: &mut Vec<String>| {
        let t = cur.trim_matches(|c: char| "./-_".contains(c)).to_lowercase();
        cur.clear();
        if t.len() >= 3 && t.chars().any(|c| c.is_ascii_alphanumeric()) && !STOP.contains(&t.as_str()) && !terms.contains(&t) {
            terms.push(t);
        }
    };
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || "_./-".contains(ch) {
            cur.push(ch);
        } else {
            flush(&mut cur, &mut terms);
        }
    }
    flush(&mut cur, &mut terms);
    terms
}

fn thai_grams(text: &str) -> std::collections::HashSet<String> {
    let chars: Vec<char> = text.chars().filter(|c| !c.is_whitespace() && !c.is_ascii_punctuation()).collect();
    chars.windows(5).map(|w| w.iter().collect()).collect()
}

/// Does `output` mention this fact? Latin facts: a path or long identifier of
/// theirs appears, or at least half of their key terms do. Facts with no Latin terms: at least half of their 5-character
/// windows appear. Deliberately lenient about wording, strict about having
/// something concrete to match.
pub fn mentions(output: &str, fact_text: &str) -> bool {
    let out = output.to_lowercase();
    let terms = key_terms(fact_text);
    if !terms.is_empty() {
        let hits: Vec<&String> = terms.iter().filter(|t| out.contains(t.as_str())).collect();
        // A path or a long identifier is specific enough to prove it alone.
        let specific = |t: &&String| t.contains('/') || t.contains('.') || t.len() >= 10;
        return hits.iter().any(specific) || (!hits.is_empty() && hits.len() * 2 >= terms.len());
    }
    // Nothing Latin to look for and nothing Thai either: only stop words.
    if fact_text.is_ascii() {
        return false;
    }
    let grams = thai_grams(&fact_text.to_lowercase());
    if grams.is_empty() {
        return false;
    }
    let seen = thai_grams(&out);
    grams.iter().filter(|g| seen.contains(*g)).count() * 2 >= grams.len()
}

/// Which of `facts` the output mentions.
pub fn referenced_ids(output: &str, facts: &[(i64, String)]) -> Vec<i64> {
    facts.iter().filter(|(_, t)| mentions(output, t)).map(|(id, _)| *id).collect()
}

/// Injected this many times with no reference: not earning its tokens.
pub const RETIRE_AFTER_INJECTIONS: i64 = 20;

/// A path-looking token in `text` that does not exist under `root`.
fn dead_path(text: &str, root: &std::path::Path) -> Option<String> {
    for raw in text.split(|c: char| c.is_whitespace() || "`'\"(),;[]{}<>".contains(c)) {
        let tok = raw.trim_end_matches(['.', ':']);
        let name = tok.rsplit('/').next().unwrap_or("");
        if !tok.contains('/') || tok.contains("://") || tok.contains('*') || tok.starts_with('~') || tok.starts_with('@') {
            continue;
        }
        // A file name with an extension, e.g. `src/order/repo.ts` — not "and/or".
        if !name.contains('.') || name.starts_with('.') || name.ends_with('.') {
            continue;
        }
        let p = std::path::Path::new(tok);
        let full = if p.is_absolute() { p.to_path_buf() } else { root.join(p) };
        if !full.exists() {
            return Some(tok.to_string());
        }
    }
    None
}

/// Queue "retire" entries for facts that are no longer earning their place:
/// unpinned, injected at least [`RETIRE_AFTER_INJECTIONS`] times and never
/// referenced, or naming a file under `root` that no longer exists. At most
/// `cap` per call, never twice for the same fact. Returns the proposal ids.
pub fn suggest_retirements(c: &Connection, root: &std::path::Path, cap: usize) -> Result<Vec<i64>> {
    let mut stmt = c.prepare(
        "SELECT id, scope, text, injected, referenced FROM fact WHERE pinned = 0
           AND id NOT IN (SELECT replaces FROM fact_proposal WHERE replaces IS NOT NULL)
         ORDER BY injected DESC, id ASC",
    )?;
    let rows: Vec<(i64, i64, String, i64, i64)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut made = Vec::new();
    for (id, scope, text, injected, referenced) in rows {
        if made.len() >= cap {
            break;
        }
        let why = if injected >= RETIRE_AFTER_INJECTIONS && referenced == 0 {
            format!("ส่งให้โมเดล {injected} ครั้ง ไม่เคยถูกใช้")
        } else if let Some(path) = dead_path(&text, root) {
            format!("อ้างถึง {path} ซึ่งไม่มีแล้ว")
        } else {
            continue;
        };
        let shown: String = text.chars().take(100).collect();
        made.push(enqueue(
            c,
            &NewProposal {
                scope,
                text: &shown,
                src: None,
                kind: ProposalKind::Retire,
                replaces: Some(id),
                evidence: Some(&why),
                suspicious: false,
            },
        )?);
    }
    Ok(made)
}
