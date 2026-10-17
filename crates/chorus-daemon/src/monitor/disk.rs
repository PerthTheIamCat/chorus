//! Chorus's own disk use, by category and per project (research 02 §7).
//!
//! Sizes are allocated bytes (`st_blocks × 512`, what `du` reports), each
//! (dev, inode) counted once so hard links are not double-counted. Computed
//! on demand when the window opens, never on the 1 s tick.

use serde::Serialize;
use std::collections::HashSet;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize)]
pub struct Categories {
    /// `store.db` + `-wal` + `-shm` (ledger, blobs, FTS, summaries).
    pub ledger: u64,
    /// Shadow checkpoint repos (`.chorus/ckpt`).
    pub checkpoints: u64,
    /// Agent worktrees (`.chorus/worktrees`): full checkouts, not shared.
    pub worktrees: u64,
    pub models: u64,
    /// Per-project memory stores.
    pub memory: u64,
    pub cache: u64,
    /// Anything else under a workspace's `.chorus/`.
    pub other: u64,
}

impl Categories {
    pub fn total(&self) -> u64 {
        self.ledger
            + self.checkpoints
            + self.worktrees
            + self.models
            + self.memory
            + self.cache
            + self.other
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TableSize {
    pub name: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectUsage {
    pub path: String,
    pub ledger: u64,
    pub checkpoints: u64,
    pub worktrees: u64,
    pub other: u64,
    pub total: u64,
    /// `dbstat` per table, largest first. Empty when there is no store.
    pub tables: Vec<TableSize>,
    /// Free pages inside `store.db`: what `VACUUM` would hand back.
    pub freelist_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiskReport {
    pub categories: Categories,
    pub total: u64,
    pub projects: Vec<ProjectUsage>,
    pub elapsed_ms: f64,
}

pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// `~/Library/Application Support/Chorus`.
pub fn app_support() -> Option<PathBuf> {
    home().map(|h| h.join("Library/Application Support/Chorus"))
}

pub fn models_dir() -> PathBuf {
    chorus_core::model_manager::ModelManager::default_models_dir()
}

/// Cache directories Chorus writes and can rebuild on demand.
pub fn cache_dirs() -> Vec<PathBuf> {
    home()
        .map(|h| vec![h.join(".cache/chorus")])
        .unwrap_or_default()
}

/// Allocated size of `path` (file or tree), counting each inode once.
pub fn du(path: &Path, seen: &mut HashSet<(u64, u64)>) -> u64 {
    let Ok(md) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if !seen.insert((md.dev(), md.ino())) {
        return 0;
    }
    let mut n = md.blocks() * 512;
    if md.is_dir() {
        if let Ok(rd) = std::fs::read_dir(path) {
            for e in rd.flatten() {
                n += du(&e.path(), seen);
            }
        }
    }
    n
}

pub fn du1(path: &Path) -> u64 {
    du(path, &mut HashSet::new())
}

fn store_files(ws: &Path) -> [PathBuf; 3] {
    let db = ws.join(".chorus/store.db");
    [
        db.clone(),
        PathBuf::from(format!("{}-wal", db.display())),
        PathBuf::from(format!("{}-shm", db.display())),
    ]
}

/// `dbstat` breakdown and freelist of a store, read-only. The bundled SQLite
/// is built with `SQLITE_ENABLE_DBSTAT_VTAB`.
pub fn store_tables(db: &Path) -> (Vec<TableSize>, u64) {
    use rusqlite::{Connection, OpenFlags};
    if !db.exists() {
        return (Vec::new(), 0);
    }
    let Ok(c) = Connection::open_with_flags(
        db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return (Vec::new(), 0);
    };
    let _ = c.busy_timeout(std::time::Duration::from_millis(500));
    let mut tables: Vec<TableSize> = c
        .prepare("SELECT name, SUM(pgsize) FROM dbstat GROUP BY name ORDER BY 2 DESC")
        .and_then(|mut s| {
            s.query_map([], |r| {
                Ok(TableSize {
                    name: r.get(0)?,
                    bytes: r.get::<_, i64>(1)?.max(0) as u64,
                })
            })
            .map(|rows| rows.flatten().collect())
        })
        .unwrap_or_default();
    // The largest dozen; FTS shadow tables and indexes make the tail long.
    tables.truncate(12);
    let free = c
        .query_row("PRAGMA freelist_count", [], |r| r.get::<_, i64>(0))
        .unwrap_or(0)
        .max(0) as u64;
    let page = c
        .query_row("PRAGMA page_size", [], |r| r.get::<_, i64>(0))
        .unwrap_or(4096)
        .max(0) as u64;
    (tables, free * page)
}

/// Per-project usage of one workspace folder.
pub fn project_usage(ws: &Path, seen: &mut HashSet<(u64, u64)>) -> ProjectUsage {
    let ledger: u64 = store_files(ws).iter().map(|p| du(p, seen)).sum();
    let checkpoints = du(&ws.join(".chorus/ckpt"), seen);
    let worktrees = du(&ws.join(".chorus/worktrees"), seen);
    // The rest of `.chorus/` (skills, agents, mcp.json …). `du` skips what
    // `seen` already counted, so this is only the remainder.
    let other = du(&ws.join(".chorus"), seen);
    let (tables, freelist_bytes) = store_tables(&ws.join(".chorus/store.db"));
    ProjectUsage {
        path: ws.display().to_string(),
        ledger,
        checkpoints,
        worktrees,
        other,
        total: ledger + checkpoints + worktrees + other,
        tables,
        freelist_bytes,
    }
}

pub fn report(workspaces: &[String]) -> DiskReport {
    let t0 = std::time::Instant::now();
    let mut seen = HashSet::new();
    let mut cats = Categories::default();
    let mut projects = Vec::new();
    let mut uniq: Vec<PathBuf> = Vec::new();
    for w in workspaces {
        let p = Path::new(w)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(w));
        if !uniq.contains(&p) && p.join(".chorus").exists() {
            uniq.push(p);
        }
    }
    for ws in &uniq {
        let u = project_usage(ws, &mut seen);
        cats.ledger += u.ledger;
        cats.checkpoints += u.checkpoints;
        cats.worktrees += u.worktrees;
        cats.other += u.other;
        projects.push(u);
    }
    projects.sort_by(|a, b| b.total.cmp(&a.total));
    cats.models = du(&models_dir(), &mut seen);
    if let Some(dir) = app_support() {
        cats.memory = du(&dir.join("projects"), &mut seen);
    }
    cats.cache = cache_dirs().iter().map(|d| du(d, &mut seen)).sum();
    DiskReport {
        total: cats.total(),
        categories: cats,
        projects,
        elapsed_ms: (t0.elapsed().as_secs_f64() * 1000.0 * 10.0).round() / 10.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hard_links_count_once() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a");
        std::fs::write(&a, vec![7u8; 64 * 1024]).unwrap();
        std::fs::hard_link(&a, d.path().join("b")).unwrap();
        let mut seen = HashSet::new();
        let both = du(d.path(), &mut seen);
        let one = du1(&a);
        assert!(one >= 64 * 1024);
        assert!(both < 2 * one, "hard link double-counted: {both} vs {one}");
    }

    #[test]
    fn categories_split_a_workspace() {
        let d = tempfile::tempdir().unwrap();
        let ws = d.path();
        std::fs::create_dir_all(ws.join(".chorus/ckpt")).unwrap();
        std::fs::create_dir_all(ws.join(".chorus/worktrees/x")).unwrap();
        std::fs::write(ws.join(".chorus/ckpt/obj"), vec![1u8; 32 * 1024]).unwrap();
        std::fs::write(ws.join(".chorus/worktrees/x/f"), vec![1u8; 48 * 1024]).unwrap();
        {
            let c = rusqlite::Connection::open(ws.join(".chorus/store.db")).unwrap();
            c.execute_batch("CREATE TABLE t(x); INSERT INTO t VALUES (zeroblob(20000));")
                .unwrap();
        }
        let r = report(&[ws.display().to_string()]);
        let p = &r.projects[0];
        assert!(
            p.checkpoints >= 32 * 1024 && p.checkpoints < 48 * 1024,
            "{p:?}"
        );
        assert!(p.worktrees >= 48 * 1024, "{p:?}");
        assert!(p.ledger >= 20_000, "{p:?}");
        assert!(
            p.tables.iter().any(|t| t.name == "t"),
            "dbstat: {:?}",
            p.tables
        );
        assert_eq!(p.total, p.ledger + p.checkpoints + p.worktrees + p.other);
    }
}
