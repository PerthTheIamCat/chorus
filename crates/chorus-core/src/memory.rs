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

/// The top `k` facts for a scope, ordered pinned-first, then by hits, then
/// most-recently-created. Bumps `hits`/`last_used_ms` for every fact
/// returned.
pub fn top(c: &Connection, scope: i64, k: usize) -> Result<Vec<Fact>> {
    let mut stmt = c.prepare(
        "SELECT id, scope, text, pinned, hits FROM fact
         WHERE scope = ?1
         ORDER BY pinned DESC, hits DESC, created_ms DESC
         LIMIT ?2",
    )?;
    let facts: Vec<Fact> = stmt
        .query_map((scope, k as i64), |r| {
            Ok(Fact {
                id: r.get(0)?,
                scope: r.get(1)?,
                text: r.get(2)?,
                pinned: r.get::<_, i64>(3)? != 0,
                hits: r.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;

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
/// come first, then the best matches above a similarity floor, then — if
/// there is still room — the usual most-used order. An empty query is the
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
    if query.trim().is_empty() || k == 0 {
        return top(c, scope, k);
    }
    let mut stmt = c.prepare(
        "SELECT id, scope, text, pinned, hits FROM fact WHERE scope = ?1
         ORDER BY pinned DESC, hits DESC, created_ms DESC",
    )?;
    let all: Vec<Fact> = stmt
        .query_map([scope], |r| {
            Ok(Fact {
                id: r.get(0)?,
                scope: r.get(1)?,
                text: r.get(2)?,
                pinned: r.get::<_, i64>(3)? != 0,
                hits: r.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;

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
                0.18,
                all.into_iter()
                    .map(|f| (crate::VectorOps::cosine_similarity(&q, &embed(&f.text)), f))
                    .collect(),
            )
        }
    };
    let mut scored = scored;
    let mut picked: Vec<Fact> = Vec::new();
    // Pinned first (already in hits order), then relevance, then the old order.
    for (_, f) in scored.iter().filter(|(_, f)| f.pinned) {
        picked.push(f.clone());
    }
    let mut hits: Vec<&(f32, Fact)> = scored.iter().filter(|(s, f)| !f.pinned && *s >= floor).collect();
    hits.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    for (_, f) in hits {
        picked.push(f.clone());
    }
    for (_, f) in scored.drain(..) {
        if !picked.iter().any(|p| p.id == f.id) {
            picked.push(f);
        }
    }
    picked.truncate(k);
    let now = now_ms();
    for f in &mut picked {
        c.execute("UPDATE fact SET hits = hits + 1, last_used_ms = ?1 WHERE id = ?2", (now, f.id))?;
        f.hits += 1;
    }
    Ok(picked)
}

/// Stored vectors for `scope` from `model`, by fact id.
fn stored_vectors(c: &Connection, scope: i64, model: &str) -> Result<std::collections::HashMap<i64, Vec<f32>>> {
    let mut stmt = c.prepare(
        "SELECT e.fact, e.vec FROM fact_embedding e JOIN fact f ON f.id = e.fact
         WHERE f.scope = ?1 AND e.model = ?2",
    )?;
    let rows = stmt
        .query_map((scope, model), |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)))?
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

/// One fact an agent wants remembered, awaiting a human's review.
#[derive(Debug, Clone)]
pub struct Proposal {
    pub id: i64,
    pub scope: i64,
    pub text: String,
    pub src: Option<i64>,
}

/// Queue a fact for review. Same 140-char limit as [`add`] — a proposal
/// that could never be approved shouldn't sit in the queue.
pub fn propose(c: &Connection, scope: i64, text: &str, src: Option<i64>) -> Result<i64> {
    if text.chars().count() > MAX_FACT_LEN {
        return Err(ChorusError::Invalid(format!(
            "proposed fact is {} chars, max is {MAX_FACT_LEN}",
            text.chars().count()
        )));
    }
    c.execute(
        "INSERT INTO fact_proposal(scope, text, src, created_ms) VALUES (?1, ?2, ?3, ?4)",
        (scope, text, src, now_ms()),
    )?;
    Ok(c.last_insert_rowid())
}

/// Everything awaiting review, oldest first.
pub fn pending(c: &Connection) -> Result<Vec<Proposal>> {
    let mut stmt = c.prepare(
        "SELECT id, scope, text, src FROM fact_proposal ORDER BY id ASC",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Proposal {
                id: r.get(0)?,
                scope: r.get(1)?,
                text: r.get(2)?,
                src: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

/// Approve proposal `id`: move it into `fact` (via [`add`], so the length
/// check and dedup apply) and drop it from the queue. Returns the new fact
/// id. `Err` if there's no such proposal.
pub fn approve(c: &Connection, id: i64) -> Result<i64> {
    approve_with(c, id, None).map(|a| a.id)
}

/// [`approve`] through [`add_with`]: a proposal that restates an existing fact
/// is merged into it instead of becoming a second one.
pub fn approve_with(c: &Connection, id: i64, embedder: Option<&dyn Embedder>) -> Result<Added> {
    let row: Option<(i64, String)> = c
        .query_row(
            "SELECT scope, text FROM fact_proposal WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (scope, text) = row.ok_or_else(|| ChorusError::Invalid(format!("no proposal {id}")))?;
    let added = add_with(c, scope, &text, embedder)?;
    c.execute("DELETE FROM fact_proposal WHERE id = ?1", [id])?;
    Ok(added)
}

/// Drop proposal `id` without remembering it. Idempotent.
pub fn reject(c: &Connection, id: i64) -> Result<()> {
    c.execute("DELETE FROM fact_proposal WHERE id = ?1", [id])?;
    Ok(())
}
