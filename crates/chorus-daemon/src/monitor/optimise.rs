//! Manual storage clean-up (ticket 14 §7). Every action has a `preview` that
//! estimates what it frees, and an `apply` the user confirms. Nothing here is
//! scheduled. Destructive actions take the ids the user picked from the
//! preview's list and re-check each one before touching it.

use super::disk::{self, du1};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Action {
    /// Delete blobs nothing references (non-destructive: no one can read
    /// them). Frees pages inside `store.db`; VACUUM returns them to disk.
    BlobGc { workspace: String },
    /// Rebuild `store.db` without free pages. Only while no turn runs.
    Vacuum { workspace: String },
    /// Pack the shadow repo's loose objects (it runs with `gc.auto=0`).
    PackCheckpoints { workspace: String },
    /// Drop checkpoints older than `days`, keeping anything `protect` names
    /// (the open session's base, `review_base`) and the current `HEAD`.
    PruneCheckpoints {
        workspace: String,
        days: u32,
        #[serde(default)]
        protect: Vec<String>,
    },
    /// Remove merged, discarded or orphaned agent worktrees. `ids` = paths
    /// from the preview list; empty means "just list".
    PruneWorktrees {
        workspace: String,
        #[serde(default)]
        ids: Vec<String>,
    },
    /// Ask Ollama to drop loaded models from memory (`keep_alive: 0`).
    UnloadModel,
    /// Empty `~/.cache/chorus` (model lists; rebuilt on demand).
    ClearCaches,
    /// Delete model files from Chorus's models dir. `ids` = file names.
    DeleteModels {
        #[serde(default)]
        ids: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct Item {
    pub id: String,
    pub label: String,
    pub bytes: u64,
    /// `merged`, `orphaned`, `dirty`, `active`, `ready`, `stray`…
    pub kind: String,
    /// The user may pick this one.
    pub selectable: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Preview {
    /// Estimated bytes freed if applied (to everything selectable).
    pub bytes: u64,
    pub destructive: bool,
    /// Can run right now (e.g. VACUUM is refused while a turn runs).
    pub allowed: bool,
    pub note: String,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Applied {
    /// Measured, not estimated: size before minus size after.
    pub freed_bytes: u64,
    pub message: String,
}

type R<T> = Result<T, String>;

fn ws_path(w: &str) -> R<PathBuf> {
    let p = Path::new(w)
        .canonicalize()
        .map_err(|e| format!("{w}: {e}"))?;
    if !p.join(".chorus").is_dir() {
        return Err(format!("{w}: ไม่ใช่ workspace ของ Chorus"));
    }
    Ok(p)
}

fn store_db(ws: &Path) -> PathBuf {
    ws.join(".chorus/store.db")
}

fn store_size(ws: &Path) -> u64 {
    let db = store_db(ws);
    du1(&db)
        + du1(Path::new(&format!("{}-wal", db.display())))
        + du1(Path::new(&format!("{}-shm", db.display())))
}

fn open_db(ws: &Path) -> R<rusqlite::Connection> {
    let c = rusqlite::Connection::open_with_flags(
        store_db(ws),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| e.to_string())?;
    c.busy_timeout(std::time::Duration::from_secs(3))
        .map_err(|e| e.to_string())?;
    Ok(c)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Blobs written in the last hour may belong to an event that is about to be
/// appended (put → append is two statements); leave them alone.
fn gc_cutoff() -> i64 {
    now_ms() - 3_600_000
}

pub fn preview(a: &Action) -> R<Preview> {
    match a {
        Action::BlobGc { workspace } => {
            let ws = ws_path(workspace)?;
            let c = open_db(&ws)?;
            let (n, bytes) =
                chorus_core::blob::gc_preview(&c, gc_cutoff()).map_err(|e| e.to_string())?;
            Ok(simple(
                bytes,
                false,
                true,
                format!("{n} blob ที่ไม่มีใครอ้างถึง · พื้นที่คืนใน store.db (กด VACUUM เพื่อคืนให้ดิสก์)"),
            ))
        }
        Action::Vacuum { workspace } => {
            let ws = ws_path(workspace)?;
            let (_, free) = disk::store_tables(&store_db(&ws));
            let busy = chorus_acp::turn_running();
            let note = if busy {
                "มีเทิร์นกำลังรันอยู่ — รอให้จบก่อน".to_string()
            } else {
                "เขียน store.db ใหม่โดยไม่มีหน้าว่าง".into()
            };
            Ok(simple(free, false, !busy, note))
        }
        Action::PackCheckpoints { workspace } => {
            let ws = ws_path(workspace)?;
            let loose = loose_bytes(&ws)?;
            // Packing delta-compresses; typically well over half of the
            // loose size comes back. Report the loose size as the ceiling.
            Ok(simple(
                loose,
                false,
                true,
                "รวม object ที่กระจายอยู่เป็น pack (ไม่ลบ checkpoint ใด)".into(),
            ))
        }
        Action::PruneCheckpoints {
            workspace,
            days,
            protect,
        } => {
            let ws = ws_path(workspace)?;
            let plan = ckpt::plan(&ws, *days, protect)?;
            Ok(Preview {
                bytes: plan.estimated_bytes,
                destructive: true,
                allowed: !plan.drop.is_empty(),
                note: format!(
                    "ลบ checkpoint ที่เก่ากว่า {days} วัน {} อัน · เก็บ {} อัน (รวม review_base และ base ของ session ที่เปิดอยู่)",
                    plan.drop.len(),
                    plan.keep.len()
                ),
                items: Vec::new(),
            })
        }
        Action::PruneWorktrees { workspace, .. } => {
            let ws = ws_path(workspace)?;
            let items = worktree_items(&ws);
            let bytes = items.iter().filter(|i| i.selectable).map(|i| i.bytes).sum();
            Ok(Preview {
                bytes,
                destructive: true,
                allowed: true,
                note: "เลือก worktree ที่จะลบ".into(),
                items,
            })
        }
        Action::UnloadModel => {
            let loaded = ollama::loaded().unwrap_or_default();
            let bytes = loaded.iter().map(|(_, b)| *b).sum();
            let items = loaded
                .into_iter()
                .map(|(name, bytes)| Item {
                    id: name.clone(),
                    label: name,
                    bytes,
                    kind: "loaded".into(),
                    selectable: true,
                })
                .collect::<Vec<_>>();
            let note = if items.is_empty() {
                "ไม่มีโมเดลโหลดอยู่ใน Ollama".into()
            } else {
                "เอาโมเดลออกจาก RAM (ไฟล์ยังอยู่)".into()
            };
            Ok(Preview {
                bytes,
                destructive: false,
                allowed: !items.is_empty(),
                note,
                items,
            })
        }
        Action::ClearCaches => {
            let bytes = disk::cache_dirs().iter().map(|d| du1(d)).sum();
            Ok(simple(
                bytes,
                false,
                bytes > 0,
                "~/.cache/chorus (สร้างใหม่ได้เองเมื่อต้องใช้)".into(),
            ))
        }
        Action::DeleteModels { .. } => {
            let items = model_items();
            Ok(Preview {
                bytes: 0,
                destructive: true,
                allowed: !items.is_empty(),
                note: "เลือกโมเดลที่จะลบ".into(),
                items,
            })
        }
    }
}

fn simple(bytes: u64, destructive: bool, allowed: bool, note: String) -> Preview {
    Preview {
        bytes,
        destructive,
        allowed,
        note,
        items: Vec::new(),
    }
}

pub fn apply(a: &Action) -> R<Applied> {
    match a {
        Action::BlobGc { workspace } => {
            let ws = ws_path(workspace)?;
            let c = open_db(&ws)?;
            let (_, bytes) =
                chorus_core::blob::gc_preview(&c, gc_cutoff()).map_err(|e| e.to_string())?;
            let n = chorus_core::blob::gc(&c, gc_cutoff()).map_err(|e| e.to_string())?;
            // Pages go to the freelist; the file only shrinks after VACUUM.
            Ok(Applied {
                freed_bytes: bytes,
                message: format!("ลบ {n} blob · คืน {} ใน store.db", human(bytes)),
            })
        }
        Action::Vacuum { workspace } => {
            if chorus_acp::turn_running() {
                return Err("มีเทิร์นกำลังรันอยู่ — VACUUM ได้เฉพาะตอนว่าง".into());
            }
            let ws = ws_path(workspace)?;
            let before = store_size(&ws);
            let c = open_db(&ws)?;
            c.execute_batch("VACUUM; PRAGMA wal_checkpoint(TRUNCATE);")
                .map_err(|e| e.to_string())?;
            drop(c);
            let freed = before.saturating_sub(store_size(&ws));
            Ok(Applied {
                freed_bytes: freed,
                message: format!("VACUUM แล้ว · คืน {}", human(freed)),
            })
        }
        Action::PackCheckpoints { workspace } => {
            let ws = ws_path(workspace)?;
            let before = du1(&ws.join(".chorus/ckpt"));
            ckpt::git(&ws, &["repack", "-a", "-d", "-q"])?;
            ckpt::git(&ws, &["prune-packed", "-q"])?;
            let freed = before.saturating_sub(du1(&ws.join(".chorus/ckpt")));
            Ok(Applied {
                freed_bytes: freed,
                message: format!("pack แล้ว · คืน {}", human(freed)),
            })
        }
        Action::PruneCheckpoints {
            workspace,
            days,
            protect,
        } => {
            let ws = ws_path(workspace)?;
            let before = du1(&ws.join(".chorus/ckpt"));
            let plan = ckpt::plan(&ws, *days, protect)?;
            let dropped = ckpt::apply(&ws, &plan)?;
            let freed = before.saturating_sub(du1(&ws.join(".chorus/ckpt")));
            Ok(Applied {
                freed_bytes: freed,
                message: format!(
                    "ลบ checkpoint {} อัน ({dropped} แถว) · คืน {}",
                    plan.drop.len(),
                    human(freed)
                ),
            })
        }
        Action::PruneWorktrees { workspace, ids } => {
            let ws = ws_path(workspace)?;
            let items = worktree_items(&ws);
            let mut freed = 0;
            let mut removed = 0;
            for id in ids {
                // Re-check at apply time: the list may be minutes old.
                let Some(it) = items.iter().find(|i| &i.id == id && i.selectable) else {
                    continue;
                };
                let path = PathBuf::from(&it.id);
                let size = du1(&path);
                if it.kind == "orphaned" {
                    std::fs::remove_dir_all(&path)
                        .map_err(|e| format!("{}: {e}", path.display()))?;
                } else {
                    let branch = it
                        .label
                        .split(" · ")
                        .nth(1)
                        .filter(|b| b.starts_with("chorus/"));
                    chorus_core::worktrees::remove_worktree(&ws, &path, branch)
                        .map_err(|e| e.to_string())?;
                    if path.exists() {
                        continue;
                    }
                }
                freed += size;
                removed += 1;
            }
            let _ = Command::new("git")
                .args(["worktree", "prune"])
                .current_dir(&ws)
                .stdin(Stdio::null())
                .output();
            Ok(Applied {
                freed_bytes: freed,
                message: format!("ลบ worktree {removed} อัน · คืน {}", human(freed)),
            })
        }
        Action::UnloadModel => {
            let loaded = ollama::loaded()?;
            let bytes = loaded.iter().map(|(_, b)| *b).sum();
            for (name, _) in &loaded {
                ollama::unload(name)?;
            }
            Ok(Applied {
                freed_bytes: bytes,
                message: format!("เอาโมเดลออกจาก RAM {} ตัว", loaded.len()),
            })
        }
        Action::ClearCaches => {
            let mut freed = 0;
            for d in disk::cache_dirs() {
                let Ok(rd) = std::fs::read_dir(&d) else {
                    continue;
                };
                for e in rd.flatten() {
                    let p = e.path();
                    let n = du1(&p);
                    let ok = if p.is_dir() && !p.is_symlink() {
                        std::fs::remove_dir_all(&p)
                    } else {
                        std::fs::remove_file(&p)
                    };
                    if ok.is_ok() {
                        freed += n;
                    }
                }
            }
            Ok(Applied {
                freed_bytes: freed,
                message: format!("ล้าง cache แล้ว · คืน {}", human(freed)),
            })
        }
        Action::DeleteModels { ids } => {
            let items = model_items();
            let dir = disk::models_dir()
                .canonicalize()
                .map_err(|e| e.to_string())?;
            let mut freed = 0;
            for id in ids {
                let Some(it) = items.iter().find(|i| &i.id == id && i.selectable) else {
                    continue;
                };
                // File names only, resolved inside the models dir.
                if it.id.contains('/') || it.id.starts_with('.') {
                    continue;
                }
                let p = dir.join(&it.id);
                let md = std::fs::symlink_metadata(&p).map_err(|e| e.to_string())?;
                if !md.is_file() {
                    continue;
                }
                let n = du1(&p);
                std::fs::remove_file(&p).map_err(|e| e.to_string())?;
                freed += n;
            }
            Ok(Applied {
                freed_bytes: freed,
                message: format!("ลบโมเดลแล้ว · คืน {}", human(freed)),
            })
        }
    }
}

pub fn human(b: u64) -> String {
    const U: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i + 1 < U.len() {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{b} B")
    } else {
        format!("{v:.1} {}", U[i])
    }
}

fn loose_bytes(ws: &Path) -> R<u64> {
    let out = ckpt::git(ws, &["count-objects", "-v"])?;
    Ok(out
        .lines()
        .find_map(|l| l.strip_prefix("size: "))
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(0)
        * 1024)
}

// ---- worktrees ------------------------------------------------------------

fn worktree_items(ws: &Path) -> Vec<Item> {
    let root = ws.join(".chorus/worktrees");
    let Ok(rd) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    // path → branch (or "" when detached / branch deleted).
    let mut registered: Vec<(PathBuf, String)> = Vec::new();
    if let Ok(o) = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(ws)
        .stdin(Stdio::null())
        .output()
    {
        let text = String::from_utf8_lossy(&o.stdout);
        let mut path: Option<PathBuf> = None;
        let mut branch = String::new();
        for line in text.lines().chain(std::iter::once("")) {
            if let Some(p) = line.strip_prefix("worktree ") {
                path = Some(PathBuf::from(p.trim()));
            } else if let Some(b) = line.strip_prefix("branch refs/heads/") {
                branch = b.trim().to_string();
            } else if line.is_empty() {
                if let Some(p) = path.take() {
                    registered.push((p.canonicalize().unwrap_or(p), std::mem::take(&mut branch)));
                }
                branch.clear();
            }
        }
    }
    let merged: Vec<String> = Command::new("git")
        .args(["branch", "--merged", "HEAD", "--format=%(refname:short)"])
        .current_dir(ws)
        .stdin(Stdio::null())
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|l| l.trim().to_string())
                .collect()
        })
        .unwrap_or_default();

    let mut items = Vec::new();
    for e in rd.flatten() {
        let p = e.path();
        if !p.is_dir() || p.is_symlink() {
            continue;
        }
        let canon = p.canonicalize().unwrap_or(p.clone());
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let bytes = du1(&canon);
        let (kind, branch) = match registered.iter().find(|(rp, _)| rp == &canon) {
            None => ("orphaned", String::new()),
            Some((_, b)) => {
                let dirty = Command::new("git")
                    .args(["status", "--porcelain"])
                    .current_dir(&canon)
                    .stdin(Stdio::null())
                    .output()
                    .map(|o| !o.status.success() || !o.stdout.is_empty())
                    .unwrap_or(true);
                if dirty {
                    ("dirty", b.clone())
                } else if b.is_empty() {
                    ("discarded", String::new())
                } else if merged.contains(b) {
                    ("merged", b.clone())
                } else {
                    ("active", b.clone())
                }
            }
        };
        let label = if branch.is_empty() {
            name
        } else {
            format!("{name} · {branch}")
        };
        items.push(Item {
            id: canon.display().to_string(),
            label,
            bytes,
            kind: kind.into(),
            selectable: matches!(kind, "orphaned" | "merged" | "discarded"),
        });
    }
    items.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    items
}

// ---- models ---------------------------------------------------------------

fn model_items() -> Vec<Item> {
    use chorus_core::model_manager::ModelManager;
    let dir = disk::models_dir();
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let curated = ModelManager::curated_models();
    let mut items = Vec::new();
    for e in rd.flatten() {
        let p = e.path();
        let Ok(md) = std::fs::symlink_metadata(&p) else {
            continue;
        };
        if !md.is_file() {
            continue;
        }
        let file = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if file.starts_with('.') {
            continue;
        }
        let spec = curated.iter().find(|s| s.filename == file);
        items.push(Item {
            id: file.clone(),
            label: spec.map(|s| format!("{} ({file})", s.id)).unwrap_or(file),
            bytes: du1(&p),
            kind: if spec.is_some() {
                "ready".into()
            } else {
                "stray".into()
            },
            selectable: true,
        });
    }
    items.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    items
}

// ---- Ollama -----------------------------------------------------------------

mod ollama {
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpStream};
    use std::time::Duration;

    const ADDR: &str = "127.0.0.1:11434";

    fn http(method: &str, path: &str, body: &str) -> Result<String, String> {
        let addr: SocketAddr = ADDR.parse().map_err(|e| format!("{e}"))?;
        let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(800))
            .map_err(|_| "Ollama ไม่ได้รันอยู่".to_string())?;
        s.set_read_timeout(Some(Duration::from_secs(10))).ok();
        let req = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
        let mut buf = Vec::new();
        s.take(1 << 20)
            .read_to_end(&mut buf)
            .map_err(|e| e.to_string())?;
        let text = String::from_utf8_lossy(&buf).into_owned();
        let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
        if !head.starts_with("HTTP/1.1 200") && !head.starts_with("HTTP/1.0 200") {
            return Err(head.lines().next().unwrap_or("bad response").to_string());
        }
        // Small JSON bodies; chunked encoding wraps them in hex length lines.
        let body = if head
            .to_ascii_lowercase()
            .contains("transfer-encoding: chunked")
        {
            body.lines()
                .filter(|l| !l.chars().all(|c| c.is_ascii_hexdigit()))
                .collect::<Vec<_>>()
                .join("")
        } else {
            body.to_string()
        };
        Ok(body)
    }

    /// `(model, bytes in memory)` from `/api/ps`.
    pub fn loaded() -> Result<Vec<(String, u64)>, String> {
        let body = http("GET", "/api/ps", "")?;
        let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
        Ok(v["models"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|m| {
                        Some((
                            m["name"].as_str()?.to_string(),
                            m["size"].as_u64().unwrap_or(0),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    pub fn unload(model: &str) -> Result<(), String> {
        let body = serde_json::json!({ "model": model, "keep_alive": 0 }).to_string();
        http("POST", "/api/generate", &body).map(|_| ())
    }
}

// ---- checkpoint pruning ---------------------------------------------------

pub(crate) mod ckpt {
    //! Prune old checkpoints without rewriting a single sha.
    //!
    //! The shadow repo is a chain of single-parent commits (plus side chains
    //! left by `restore`, kept alive by the reflog). Rewriting history would
    //! change every sha that `checkpoint_turn`, `review_base` and an open
    //! session's base point at. Instead we mark the oldest commit we keep on
    //! each chain as *shallow* (`<git-dir>/shallow`, git's own mechanism for
    //! `--depth` clones) and pin every kept tip with a ref, then `gc`: what is
    //! older than a shallow boundary is no longer reachable and gets pruned.

    use super::R;
    use std::collections::{HashMap, HashSet};
    use std::path::Path;
    use std::process::{Command, Stdio};

    pub fn git(ws: &Path, args: &[&str]) -> R<String> {
        git_in(ws, args, None)
    }

    fn git_in(ws: &Path, args: &[&str], stdin: Option<&str>) -> R<String> {
        let git_dir = ws.join(".chorus/ckpt");
        if !git_dir.join("HEAD").exists() {
            return Err("ยังไม่มี checkpoint ใน workspace นี้".into());
        }
        let mut cmd = Command::new("git");
        cmd.arg("--git-dir")
            .arg(&git_dir)
            .arg("--work-tree")
            .arg(ws)
            .args(args)
            .current_dir(ws);
        cmd.stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| format!("git: {e}"))?;
        if let Some(input) = stdin {
            use std::io::Write;
            child
                .stdin
                .take()
                .map(|mut s| s.write_all(input.as_bytes()))
                .transpose()
                .map_err(|e| e.to_string())?;
        }
        let out = child.wait_with_output().map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(format!(
                "git {}: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    #[derive(Debug, Default)]
    pub struct Plan {
        /// Kept commits (tips and everything between them and their boundary).
        pub keep: HashSet<String>,
        pub drop: HashSet<String>,
        /// Tips that need a ref so `gc` keeps them (side chains, old protects).
        pub tips: Vec<String>,
        /// Kept commits whose parent is dropped: they go in the shallow file.
        pub boundaries: Vec<String>,
        pub estimated_bytes: u64,
    }

    /// Shas `review_base` points at, when the column exists (schema v7).
    fn review_bases(ws: &Path) -> Vec<String> {
        let Ok(c) = rusqlite::Connection::open_with_flags(
            ws.join(".chorus/store.db"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ) else {
            return Vec::new();
        };
        let has = c
            .prepare("SELECT 1 FROM pragma_table_info('session') WHERE name = 'review_base'")
            .and_then(|mut s| s.exists([]))
            .unwrap_or(false);
        if !has {
            return Vec::new();
        }
        c.prepare("SELECT DISTINCT review_base FROM session WHERE review_base IS NOT NULL AND review_base != ''")
            .and_then(|mut s| s.query_map([], |r| r.get::<_, String>(0)).map(|r| r.flatten().collect()))
            .unwrap_or_default()
    }

    /// `checkpoint_turn` rows `(sha, created_ms)`, when the table exists.
    fn turn_rows(ws: &Path) -> Vec<(String, i64)> {
        let Ok(c) = rusqlite::Connection::open_with_flags(
            ws.join(".chorus/store.db"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ) else {
            return Vec::new();
        };
        c.prepare("SELECT commit_sha, created_ms FROM checkpoint_turn")
            .and_then(|mut s| {
                s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                    .map(|r| r.flatten().collect())
            })
            .unwrap_or_default()
    }

    /// Every commit git can still reach (refs + reflogs): sha → (parent, time).
    fn graph(ws: &Path) -> R<HashMap<String, (Option<String>, i64)>> {
        let text = git(ws, &["log", "--all", "--reflog", "--format=%H %ct %P"])?;
        let mut g = HashMap::new();
        for line in text.lines() {
            let mut t = line.split_whitespace();
            let (Some(sha), Some(ct)) = (t.next(), t.next()) else {
                continue;
            };
            let ct = ct.parse::<i64>().unwrap_or(0);
            g.insert(sha.to_string(), (t.next().map(|p| p.to_string()), ct));
        }
        Ok(g)
    }

    pub fn plan(ws: &Path, days: u32, protect: &[String]) -> R<Plan> {
        let g = graph(ws)?;
        let cutoff_s = super::now_ms() / 1000 - days as i64 * 86_400;
        let head = git(ws, &["rev-parse", "HEAD"])
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        let resolve = |s: &str| -> Option<String> {
            let s = s.trim();
            if s.is_empty() {
                return None;
            }
            if g.contains_key(s) {
                return Some(s.to_string());
            }
            g.keys().find(|k| k.starts_with(s) && s.len() >= 7).cloned()
        };
        let mut tips: Vec<String> = Vec::new();
        let mut add = |s: &str| {
            if let Some(sha) = resolve(s) {
                if !tips.contains(&sha) {
                    tips.push(sha);
                }
            }
        };
        add(&head);
        for p in protect {
            add(p);
        }
        for r in review_bases(ws) {
            add(&r);
        }
        for (sha, ms) in turn_rows(ws) {
            if ms / 1000 >= cutoff_s {
                add(&sha);
            }
        }
        // Commits newer than the cutoff on any reachable chain stay, so
        // `restore` side chains and turn guards inside the window survive.
        for (sha, (_, ct)) in &g {
            if *ct >= cutoff_s {
                add(sha);
            }
        }

        let mut keep: HashSet<String> = HashSet::new();
        for t in &tips {
            let mut cur = t.clone();
            loop {
                if !keep.insert(cur.clone()) {
                    break;
                }
                match g.get(&cur).and_then(|(p, _)| p.clone()) {
                    Some(parent) if g.get(&parent).is_some_and(|(_, ct)| *ct >= cutoff_s) => {
                        cur = parent
                    }
                    _ => break,
                }
            }
        }
        let drop: HashSet<String> = g.keys().filter(|k| !keep.contains(*k)).cloned().collect();
        let boundaries: Vec<String> = keep
            .iter()
            .filter(|k| {
                g.get(*k)
                    .and_then(|(p, _)| p.as_ref())
                    .is_some_and(|p| drop.contains(p))
            })
            .cloned()
            .collect();

        // Only chain ends need a ref; everything behind them is reachable.
        let parents: HashSet<&String> = keep
            .iter()
            .filter_map(|k| g.get(k).and_then(|(p, _)| p.as_ref()))
            .collect();
        let tips: Vec<String> = tips.into_iter().filter(|t| !parents.contains(t)).collect();
        let mut plan = Plan {
            keep,
            drop,
            tips,
            boundaries,
            estimated_bytes: 0,
        };
        if !plan.drop.is_empty() {
            plan.estimated_bytes = estimate(ws, &plan).unwrap_or(0);
        }
        Ok(plan)
    }

    /// Total reachable bytes minus what the kept commits still need.
    fn estimate(ws: &Path, plan: &Plan) -> R<u64> {
        let parse = |s: String| s.trim().parse::<u64>().unwrap_or(0);
        let total = parse(git(
            ws,
            &["rev-list", "--objects", "--disk-usage", "--all", "--reflog"],
        )?);
        // Objects new in the kept range …
        let mut input: Vec<String> = plan.tips.clone();
        for b in &plan.boundaries {
            input.push(format!("^{b}^"));
        }
        let kept_range = parse(git_in(
            ws,
            &["rev-list", "--objects", "--disk-usage", "--stdin"],
            Some(&(input.join("\n") + "\n")),
        )?);
        // … plus everything the boundary trees still use.
        let kept_roots = if plan.boundaries.is_empty() {
            0
        } else {
            parse(git_in(
                ws,
                &[
                    "rev-list",
                    "--objects",
                    "--disk-usage",
                    "--no-walk",
                    "--stdin",
                ],
                Some(&(plan.boundaries.join("\n") + "\n")),
            )?)
        };
        Ok(total.saturating_sub(kept_range + kept_roots))
    }

    /// Returns how many `checkpoint_turn` rows were deleted.
    pub fn apply(ws: &Path, plan: &Plan) -> R<usize> {
        if plan.drop.is_empty() {
            return Ok(0);
        }
        let git_dir = ws.join(".chorus/ckpt");
        // 1. Pin every tip. Stale pins from an earlier prune are replaced.
        let old = git(
            ws,
            &["for-each-ref", "--format=%(refname)", "refs/chorus-keep/"],
        )?;
        let mut cmds = String::new();
        for r in old.lines().filter(|l| !l.is_empty()) {
            cmds.push_str(&format!("delete {r}\n"));
        }
        for t in &plan.tips {
            cmds.push_str(&format!("update refs/chorus-keep/{t} {t}\n"));
        }
        git_in(ws, &["update-ref", "--stdin"], Some(&cmds))?;
        // 2. Cut each chain at its boundary.
        let shallow = git_dir.join("shallow");
        let mut lines: Vec<String> = std::fs::read_to_string(&shallow)
            .unwrap_or_default()
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty() && plan.keep.contains(l))
            .collect();
        for b in &plan.boundaries {
            if !lines.contains(b) {
                lines.push(b.clone());
            }
        }
        std::fs::write(&shallow, lines.join("\n") + "\n").map_err(|e| e.to_string())?;
        // 3. Forget the reflog (it is what keeps side chains alive), then gc.
        git(ws, &["reflog", "expire", "--expire=now", "--all"])?;
        git(ws, &["gc", "--prune=now", "--quiet"])?;
        // 4. Rows pointing at commits that no longer exist.
        let mut n = 0;
        if let Ok(c) = rusqlite::Connection::open(ws.join(".chorus/store.db")) {
            let _ = c.busy_timeout(std::time::Duration::from_secs(3));
            let has = c
                .prepare(
                    "SELECT 1 FROM sqlite_master WHERE type='table' AND name='checkpoint_turn'",
                )
                .and_then(|mut s| s.exists([]))
                .unwrap_or(false);
            if has {
                for sha in &plan.drop {
                    n += c
                        .execute("DELETE FROM checkpoint_turn WHERE commit_sha = ?1", [sha])
                        .unwrap_or(0);
                }
            }
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(dir: &Path, args: &[&str], date: Option<&str>) -> String {
        let mut c = Command::new("git");
        c.arg("--git-dir")
            .arg(dir.join(".chorus/ckpt"))
            .arg("--work-tree")
            .arg(dir)
            .args(args)
            .current_dir(dir);
        if let Some(d) = date {
            c.env("GIT_COMMITTER_DATE", d).env("GIT_AUTHOR_DATE", d);
        }
        let o = c.output().unwrap();
        assert!(
            o.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    }

    fn commit(dir: &Path, content: &str, date: &str) -> String {
        std::fs::write(dir.join("f.txt"), content).unwrap();
        sh(dir, &["add", "-A"], None);
        sh(dir, &["commit", "-q", "-m", "x"], Some(date));
        sh(dir, &["rev-parse", "HEAD"], None)
    }

    #[test]
    fn prune_keeps_protected_and_recent_shas_unchanged() {
        let d = tempfile::tempdir().unwrap();
        let ws = d.path().canonicalize().unwrap();
        chorus_core::checkpoint::Checkpoint::open(&ws).unwrap();
        let day = 86_400;
        let now = now_ms() / 1000;
        let at = |days_ago: i64| format!("@{} +0000", now - days_ago * day);
        // Unique, incompressible-ish content per commit so dropping is visible.
        let big = |i: u32| {
            (0..20_000u32)
                .map(|k| (k.wrapping_mul(2_654_435_761).wrapping_add(i * 97) % 251) as u8 as char)
                .collect::<String>()
        };
        let c_old1 = commit(&ws, &big(1), &at(40));
        let c_review = commit(&ws, &big(2), &at(35));
        let c_old3 = commit(&ws, &big(3), &at(31));
        let c_new1 = commit(&ws, &big(4), &at(2));
        let c_new2 = commit(&ws, &big(5), &at(1));

        let plan = ckpt::plan(&ws, 30, std::slice::from_ref(&c_review)).unwrap();
        assert!(
            plan.keep.contains(&c_review)
                && plan.keep.contains(&c_new1)
                && plan.keep.contains(&c_new2)
        );
        assert!(
            plan.drop.contains(&c_old1) && plan.drop.contains(&c_old3),
            "{plan:?}"
        );
        assert!(plan.estimated_bytes > 0);

        ckpt::apply(&ws, &plan).unwrap();
        let exists = |sha: &str| {
            Command::new("git")
                .arg("--git-dir")
                .arg(ws.join(".chorus/ckpt"))
                .args(["cat-file", "-e", &format!("{sha}^{{commit}}")])
                .status()
                .unwrap()
                .success()
        };
        assert!(
            exists(&c_review) && exists(&c_new1) && exists(&c_new2),
            "kept shas must survive unchanged"
        );
        assert!(
            !exists(&c_old1) && !exists(&c_old3),
            "old checkpoints must be gone"
        );
        // The kept commits still diff and restore.
        assert_eq!(sh(&ws, &["rev-parse", "HEAD"], None), c_new2);
        sh(&ws, &["diff", "--stat", &c_new1, &c_new2], None);
        sh(&ws, &["diff", "--stat", &c_review], None);
        let ck = chorus_core::checkpoint::Checkpoint::open(&ws).unwrap();
        ck.restore(&c_new1).unwrap();
        assert_eq!(std::fs::read_to_string(ws.join("f.txt")).unwrap(), big(4));
        // New snapshots still work on the pruned repo.
        std::fs::write(ws.join("g.txt"), "after").unwrap();
        assert!(!ck.snapshot("after prune").unwrap().is_empty());
        // Second prune with nothing old left is a no-op.
        let again = ckpt::plan(&ws, 30, std::slice::from_ref(&c_review)).unwrap();
        assert!(again.drop.is_empty(), "{again:?}");
    }

    #[test]
    fn worktree_list_marks_orphans_and_spares_active_ones() {
        let d = tempfile::tempdir().unwrap();
        let ws = d.path().canonicalize().unwrap();
        let g = |args: &[&str]| {
            assert!(
                Command::new("git")
                    .args(args)
                    .current_dir(&ws)
                    .output()
                    .unwrap()
                    .status
                    .success(),
                "{args:?}"
            )
        };
        g(&["init", "-q"]);
        g(&[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "root",
        ]);
        std::fs::create_dir_all(ws.join(".chorus/worktrees/orphan-1")).unwrap();
        std::fs::write(ws.join(".chorus/worktrees/orphan-1/x"), "x").unwrap();
        let wt = ws.join(".chorus/worktrees/coder-abc");
        g(&[
            "worktree",
            "add",
            "-q",
            "-b",
            "chorus/coder-abc",
            wt.to_str().unwrap(),
            "HEAD",
        ]);
        g(&[
            "-C",
            wt.to_str().unwrap(),
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "work",
        ]);

        let items = worktree_items(&ws);
        let orphan = items.iter().find(|i| i.label == "orphan-1").unwrap();
        assert_eq!(
            (orphan.kind.as_str(), orphan.selectable),
            ("orphaned", true)
        );
        let active = items
            .iter()
            .find(|i| i.label.starts_with("coder-abc"))
            .unwrap();
        assert_eq!(
            (active.kind.as_str(), active.selectable),
            ("active", false),
            "unmerged work is never offered"
        );

        let ws_s = ws.display().to_string();
        let applied = apply(&Action::PruneWorktrees {
            workspace: ws_s.clone(),
            ids: vec![orphan.id.clone(), active.id.clone()],
        })
        .unwrap();
        assert!(!ws.join(".chorus/worktrees/orphan-1").exists());
        assert!(
            wt.exists(),
            "active worktree must survive even when its id is sent"
        );
        assert!(applied.freed_bytes > 0);
    }

    #[test]
    fn blob_gc_preview_matches_gc() {
        let d = tempfile::tempdir().unwrap();
        let ws = d.path();
        std::fs::create_dir_all(ws.join(".chorus")).unwrap();
        let store = chorus_core::store::Store::open(&ws.join(".chorus/store.db")).unwrap();
        let id = chorus_core::blob::put(store.conn(), b"orphan blob bytes").unwrap();
        store
            .conn()
            .execute(
                "UPDATE blob SET created_ms = 0 WHERE id = ?1",
                [id.to_string()],
            )
            .unwrap();
        drop(store);
        let w = ws.display().to_string();
        let p = preview(&Action::BlobGc {
            workspace: w.clone(),
        })
        .unwrap();
        assert!(p.bytes > 0, "{p:?}");
        let a = apply(&Action::BlobGc {
            workspace: w.clone(),
        })
        .unwrap();
        assert_eq!(a.freed_bytes, p.bytes);
        assert_eq!(preview(&Action::BlobGc { workspace: w }).unwrap().bytes, 0);
    }

    #[test]
    fn human_sizes() {
        assert_eq!(human(512), "512 B");
        assert_eq!(human(1536), "1.5 KB");
        assert_eq!(human(3 * 1024 * 1024 * 1024), "3.0 GB");
    }
}
