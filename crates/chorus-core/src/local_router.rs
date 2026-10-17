//! Local in-process task router and monetary cost accounting.
//! Fast 3-8ms pre-dispatch classification with cost-biased utility
//! and SQLite kNN override memory flywheel.

use crate::router::RoutePreset;
use crate::Result;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteDecision {
    pub recommended_model: String,
    pub recommended_agent: String,
    pub complexity_score: f32, // 0.0 (trivial) to 1.0 (architectural)
    pub estimated_cost_usd: f64,
    pub dispatch_delay_ms: u64, // 0ms (instant dispatch)
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostAccounting {
    pub input_tokens: usize,
    pub output_tokens: usize,
    pub cost_usd: f64,
}

/// Calculate dollar cost based on model pricing
pub fn calculate_cost(model: &str, input_tokens: usize, output_tokens: usize) -> CostAccounting {
    // Standard rates per 1M tokens
    let (in_rate, out_rate) = match model {
        m if m.contains("opus") => (15.0, 75.0),
        m if m.contains("sonnet") || m.contains("sol") => (3.0, 15.0),
        m if m.contains("haiku") || m.contains("mini") || m.contains("luna") => (0.25, 1.25),
        m if m.contains("codex") => (2.5, 10.0),
        _ => (1.0, 5.0), // default
    };

    let cost_usd =
        ((input_tokens as f64 * in_rate) + (output_tokens as f64 * out_rate)) / 1_000_000.0;
    CostAccounting {
        input_tokens,
        output_tokens,
        cost_usd,
    }
}

/// Instant in-process routing evaluation (sub-5ms)
pub fn route_task_fast(prompt: &str, _candidates: &[(&str, &str)]) -> RouteDecision {
    let lower = prompt.to_lowercase();
    let len = prompt.chars().count();

    // Fast keyword & structural probe
    let is_complex = lower.contains("architect")
        || lower.contains("refactor")
        || lower.contains("redesign")
        || lower.contains("concurrency")
        || len > 600;

    let is_review = lower.contains("review") || lower.contains("audit") || lower.contains("check");

    let score = if is_complex {
        0.85
    } else if is_review {
        0.50
    } else if len < 80 {
        0.15
    } else {
        0.40
    };

    // Cost-biased selection: default to cheaper model tier unless complexity > 0.7
    let (selected_agent, selected_model, reason) = if score > 0.70 {
        (
            "claude",
            "claude-sonnet-4-6",
            "High complexity requires reasoning depth",
        )
    } else if score > 0.35 {
        (
            "codex",
            "gpt-5.1-codex",
            "Standard task routed to efficient tier",
        )
    } else {
        (
            "codex",
            "gpt-5.1-codex-mini",
            "Trivial query routed to lightweight low-cost tier",
        )
    };

    // Estimated cost for ~1,500 prompt tokens and ~500 output tokens
    let cost = calculate_cost(selected_model, 1500, 500).cost_usd;

    RouteDecision {
        recommended_model: selected_model.to_string(),
        recommended_agent: selected_agent.to_string(),
        complexity_score: score,
        estimated_cost_usd: cost,
        dispatch_delay_ms: 0, // 0ms: no 5-second countdown
        reason: reason.to_string(),
    }
}

/// Instant in-process fast routing decision with learned memory lookup.
/// Only overrides learned under the same preset are consulted.
pub fn route_task_fast_with_db(
    conn: Option<&Connection>,
    prompt: &str,
    candidates: &[(&str, &str)],
    preset: RoutePreset,
) -> RouteDecision {
    if let Some(c) = conn {
        if let Ok(Some(learned)) = find_learned_route(c, prompt, preset) {
            let cost = calculate_cost(&learned.chosen_model, 1500, 500).cost_usd;
            return RouteDecision {
                recommended_model: learned.chosen_model,
                recommended_agent: if learned.chosen_agent.is_empty() {
                    "auto".into()
                } else {
                    learned.chosen_agent
                },
                complexity_score: 0.5,
                estimated_cost_usd: cost,
                dispatch_delay_ms: 0,
                reason: if learned.reason.is_empty() {
                    "เรียนรู้จากความชอบที่คุณเคยเลือกไว้".to_string()
                } else {
                    format!("เรียนรู้จากประวัติการเลือกก่อนหน้า: {}", learned.reason)
                },
            };
        }
    }
    route_task_fast(prompt, candidates)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearnedRoute {
    pub id: i64,
    pub prompt: String,
    pub chosen_model: String,
    pub chosen_agent: String,
    pub reason: String,
    pub created_ms: i64,
    /// The route preset active when the user made this choice. Learning is
    /// per preset: picking a large model under Max says nothing about Economy.
    pub preset: String,
}

pub fn init_override_memory(conn: &Connection) -> Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS router_override_memory (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            prompt TEXT NOT NULL,
            chosen_model TEXT NOT NULL,
            chosen_agent TEXT NOT NULL DEFAULT '',
            reason TEXT NOT NULL DEFAULT '',
            created_ms INTEGER NOT NULL
        )",
        [],
    )?;
    // Safe schema evolution for existing databases
    let _ = conn.execute(
        "ALTER TABLE router_override_memory ADD COLUMN chosen_agent TEXT NOT NULL DEFAULT ''",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE router_override_memory ADD COLUMN reason TEXT NOT NULL DEFAULT ''",
        [],
    );
    // Rows from before presets existed were learned under the old table,
    // which is what Balanced keeps.
    let _ = conn.execute(
        "ALTER TABLE router_override_memory ADD COLUMN preset TEXT NOT NULL DEFAULT 'balanced'",
        [],
    );
    Ok(())
}

pub fn record_user_model_override(
    conn: &Connection,
    prompt: &str,
    chosen_model: &str,
    preset: RoutePreset,
) -> Result<()> {
    record_route_learning(conn, prompt, chosen_model, "", "ผู้ใช้เลือกโมเดลนี้เอง", preset)?;
    Ok(())
}

/// Record a choice the user made. Never call this with the router's own
/// pick: learning from itself only makes the router more sure of its habits.
pub fn record_route_learning(
    conn: &Connection,
    prompt: &str,
    chosen_model: &str,
    chosen_agent: &str,
    reason: &str,
    preset: RoutePreset,
) -> Result<LearnedRoute> {
    init_override_memory(conn)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;

    conn.execute(
        "INSERT INTO router_override_memory (prompt, chosen_model, chosen_agent, reason, created_ms, preset)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![prompt, chosen_model, chosen_agent, reason, now, preset.as_str()],
    )?;

    let id = conn.last_insert_rowid();
    Ok(LearnedRoute {
        id,
        prompt: prompt.to_string(),
        chosen_model: chosen_model.to_string(),
        chosen_agent: chosen_agent.to_string(),
        reason: reason.to_string(),
        created_ms: now,
        preset: preset.as_str().to_string(),
    })
}

pub fn list_learned_routes(conn: &Connection, limit: usize) -> Result<Vec<LearnedRoute>> {
    init_override_memory(conn)?;
    let mut stmt = conn.prepare(
        "SELECT id, prompt, chosen_model, chosen_agent, reason, created_ms, preset
         FROM router_override_memory
         ORDER BY id DESC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit as i64], |row| {
        Ok(LearnedRoute {
            id: row.get(0)?,
            prompt: row.get(1)?,
            chosen_model: row.get(2)?,
            chosen_agent: row.get(3)?,
            reason: row.get(4)?,
            created_ms: row.get(5)?,
            preset: row.get(6)?,
        })
    })?;

    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub fn clear_learned_routes(conn: &Connection) -> Result<()> {
    init_override_memory(conn)?;
    conn.execute("DELETE FROM router_override_memory", [])?;
    Ok(())
}

pub fn find_learned_route(
    conn: &Connection,
    prompt: &str,
    preset: RoutePreset,
) -> Result<Option<LearnedRoute>> {
    init_override_memory(conn)?;
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    // 1. Exact case-insensitive match
    let mut stmt = conn.prepare(
        "SELECT id, prompt, chosen_model, chosen_agent, reason, created_ms, preset
         FROM router_override_memory
         WHERE LOWER(TRIM(prompt)) = LOWER(?1) AND preset = ?2
         ORDER BY id DESC
         LIMIT 1",
    )?;
    let mut rows = stmt.query(params![trimmed, preset.as_str()])?;
    if let Some(row) = rows.next()? {
        return Ok(Some(LearnedRoute {
            id: row.get(0)?,
            prompt: row.get(1)?,
            chosen_model: row.get(2)?,
            chosen_agent: row.get(3)?,
            reason: row.get(4)?,
            created_ms: row.get(5)?,
            preset: row.get(6)?,
        }));
    }

    // 2. Keyword/substring match for prompt if long enough
    if trimmed.len() >= 6 {
        let pattern = format!("%{}%", trimmed.to_lowercase());
        let mut stmt = conn.prepare(
            "SELECT id, prompt, chosen_model, chosen_agent, reason, created_ms, preset
             FROM router_override_memory
             WHERE (LOWER(prompt) LIKE ?1 OR ?2 LIKE '%' || LOWER(prompt) || '%') AND preset = ?3
             ORDER BY id DESC
             LIMIT 1",
        )?;
        let mut rows = stmt.query(params![pattern, trimmed.to_lowercase(), preset.as_str()])?;
        if let Some(row) = rows.next()? {
            return Ok(Some(LearnedRoute {
                id: row.get(0)?,
                prompt: row.get(1)?,
                chosen_model: row.get(2)?,
                chosen_agent: row.get(3)?,
                reason: row.get(4)?,
                created_ms: row.get(5)?,
                preset: row.get(6)?,
            }));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_and_list_learned_routes() {
        let conn = Connection::open_in_memory().unwrap();
        let b = RoutePreset::Balanced;
        let r1 = record_route_learning(
            &conn,
            "สร้าง REST API ด้วย Bun",
            "claude-sonnet-4-6",
            "claude",
            "Backend task",
            b,
        )
        .unwrap();
        assert_eq!(r1.prompt, "สร้าง REST API ด้วย Bun");
        assert_eq!(r1.chosen_model, "claude-sonnet-4-6");

        let r2 = record_route_learning(
            &conn,
            "แก้ไขบั๊ก CSS เล็กน้อย",
            "gpt-5.1-codex-mini",
            "codex",
            "Quick fix",
            b,
        )
        .unwrap();
        assert_eq!(r2.chosen_agent, "codex");

        let list = list_learned_routes(&conn, 10).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, r2.id); // newest first

        // Test find
        let found = find_learned_route(&conn, "สร้าง REST API ด้วย Bun", b).unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().chosen_model, "claude-sonnet-4-6");

        // Test clear
        clear_learned_routes(&conn).unwrap();
        let empty_list = list_learned_routes(&conn, 10).unwrap();
        assert!(empty_list.is_empty());
    }

    #[test]
    fn learning_is_per_preset() {
        let conn = Connection::open_in_memory().unwrap();
        let prompt = "ออกแบบระบบ billing";
        record_route_learning(&conn, prompt, "gpt-6-astra", "codex", "", RoutePreset::Max).unwrap();
        record_route_learning(
            &conn,
            prompt,
            "gpt-5.6-luna",
            "codex",
            "",
            RoutePreset::Economy,
        )
        .unwrap();

        let max = find_learned_route(&conn, prompt, RoutePreset::Max)
            .unwrap()
            .unwrap();
        assert_eq!(max.chosen_model, "gpt-6-astra");
        assert_eq!(max.preset, "max");
        let eco = find_learned_route(&conn, prompt, RoutePreset::Economy)
            .unwrap()
            .unwrap();
        assert_eq!(eco.chosen_model, "gpt-5.6-luna");
        assert!(find_learned_route(&conn, prompt, RoutePreset::Balanced)
            .unwrap()
            .is_none());

        let fast = route_task_fast_with_db(Some(&conn), prompt, &[], RoutePreset::Max);
        assert_eq!(fast.recommended_model, "gpt-6-astra");
    }

    #[test]
    fn rows_from_before_presets_count_as_balanced() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE router_override_memory (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                prompt TEXT NOT NULL,
                chosen_model TEXT NOT NULL,
                created_ms INTEGER NOT NULL
            )",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO router_override_memory (prompt, chosen_model, created_ms) VALUES ('fix typo', 'haiku', 1)",
            [],
        )
        .unwrap();
        let old = find_learned_route(&conn, "fix typo", RoutePreset::Balanced)
            .unwrap()
            .unwrap();
        assert_eq!(old.chosen_model, "haiku");
        assert_eq!(old.preset, "balanced");
        assert!(find_learned_route(&conn, "fix typo", RoutePreset::Max)
            .unwrap()
            .is_none());
    }
}
