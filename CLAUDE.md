# CLAUDE.md — บริบทโปรเจกต์ Chorus

ไฟล์นี้คือความจำของโปรเจกต์ อ่านให้จบก่อนเริ่มงานทุกครั้ง
เขียนไว้เพื่อไม่ต้องเล่าเรื่องเดิมซ้ำทุก session

---

## 1. โปรเจกต์นี้คืออะไร

**Chorus — *Many agents. One score.***

Desktop app (macOS ก่อน) ที่ให้ AI agent หลายเจ้า — Claude, Codex, Gemini, โมเดล local —
ทำงานอยู่ใน **session เดียวกัน** ใช้ความจำก้อนเดียวกัน เห็น skill ชุดเดียวกัน
และ **ส่งไม้ต่อกันได้กลางประโยค** เมื่อตัวใดตัวหนึ่งชนลิมิต

Pain point ที่แก้: ทุกวันนี้ dev ที่ใช้ AI หลายตัวไม่ได้เจ็บเพราะโมเดลไม่เก่ง
แต่เจ็บเพราะ **บริบทมันขาดทุกครั้งที่เปลี่ยนเครื่องมือ**

คุยกับเจ้าของโปรเจกต์ (Perth) **เป็นภาษาไทย** เรียกว่า "เพื่อน" โค้ดและคอมเมนต์เป็นอังกฤษ
แต่ข้อความที่ผู้ใช้เห็น (CLI output, README) เป็นไทยได้

---

## 2. กฎ 3 ข้อที่ห้ามแหก

### 2.1 storage ≠ context
ที่เก็บบนดิสก์ กับ สิ่งที่ส่งให้โมเดล เป็นคนละ format กัน **เสมอ**

- **Cold (ดิสก์)** ปรับให้ไบต์น้อย + query เร็ว ไม่ต้องให้ AI อ่านรู้เรื่อง
- **Hot (เข้าโมเดล)** ปรับให้ token น้อย ประกอบใหม่ทุกครั้ง **ไม่เคยเขียนกลับลงดิสก์**
  และไม่มี metadata ที่โมเดลไม่ได้ใช้ (id, timestamp, confidence)

ถ้าเผลอเอา format ฝั่งหนึ่งไปใช้อีกฝั่ง จะได้ทั้งดิสก์บวมและ token เปลืองพร้อมกัน

### 2.2 ledger เป็นแหล่งความจริงเดียว
Session เป็นของแอป **ไม่ใช่ของ provider** provider เป็นแค่ "ผู้ถูกเรียกมาตอบหนึ่งเทิร์น"
พังตัวไหนไป ledger ยังอยู่ครบ อย่าออกแบบอะไรที่ต้องพึ่ง session ฝั่ง agent

### 2.3 ห้ามมี logic ใน Swift
เราทำ macOS ก่อน แต่ทุก logic อยู่ใน Rust และคุยกับ UI ผ่าน event stream ทางเดียว
Swift ทำแค่วาดกับรับ input เพื่อให้วันที่เริ่มทำ WinUI 3 **ไม่ต้องแก้ core แม้แต่บรรทัดเดียว**

---

## 3. สถานะตอนนี้ (19 ต.ค. 2026 · ปล่อยแล้วถึง v0.1.4 — ดู `CHANGELOG.md`)

**Phase 0–2 ครบ + macOS shell ครบ + warm tier + UX v2 (workbench / orchestra / sub sessions / live updates) ครบ**
— บน Linux: เทสต์ผ่าน 382 · ล้ม 6 ตัวที่ต้องมี Seatbelt (ดู 7.22) · clippy `-D warnings` สะอาด · build 0 warning · bench invariant ครบ
— CI (`.github/workflows/ci.yml`, macOS) รัน test/clippy/bench/swift ทุก push+PR — **ยังไม่เคยเห็นมันรันจริง** ดูผลรอบแรกก่อนเชื่อ
(ตัวเลขนี้ verify บน container Linux 19 ต.ค. — ยังไม่ได้รันซ้ำบน macOS · `SCHEMA_VERSION` = 7)

| crate | ทำอะไร |
|---|---|
| `chorus-core` | ledger (+ fork-chain `history()`), blob store, memory + approve queue (v2), token counter, render, context assembler, handoff brief, projector, checkpoint (shadow git + `apply_reverse`), compaction (Extractive + **Ollama**) — **รังส์ L1/L2 เก็บลงตาราง `summary` (เพิ่มใน schema v3 · ตอนนี้ `SCHEMA_VERSION` = 7) ไม่ generate ตอน assemble**, **mcp (Streamable-HTTP: `memory_propose` — ทำงานกับ Codex)** |
| `chorus-acp` | ACP client + `acp-probe` + mock (`--fail`) + `CodexAgent` + `bridge_for` (codex/claude/antigravity) · strip `CLAUDECODE` |
| `chorus-cli` | lib+bin · `bench`/`run`(+`--chain`/`--read-only`)/`handoff`/`memory`/`compare` · `Session` engine |
| `chorus-ffi` | C ABI (`chorus.h`) — engine/run_turn(+interactive callback)/run_relay/compare/handoff/events_since/working_diff/snapshot/restore/reject_hunks/memory_state+review/**compact/uncompacted/task_state** · JSON ล้วน · panic-guarded |
| `chorus-daemon` | `chorusd` headless engine — Unix socket `~/.chorus/engine.sock` + SSE ให้ shell/CLI/มือถือ · System Monitor · pairing |
| `chorus-eval` | harness eval — Tier 0 invariants · Tier 1 router gate · metrics |
| `apps/macos` | SwiftUI shell — sidebar (single/relay/compare/**ask-each**) · thread + agent blame · Context/Changes(**per-hunk keep-reject**)/Memory panes · handoff + compare sheets · **interactive permission dialog** · **Task state pane + warm-tier backlog (`compactIfIdle` หลังทุกเทิร์น)** · `apps/macos/build.sh` |

**Flow ที่ทำงานจริงกับ Codex**: หลาย agent ใน session เดียว · ledger เป็น SoT · จำข้ามเทิร์นจาก ledger
(ไม่ใช่ provider session) · handoff กลางคัน + Verify echo-back · brief พก diff จาก checkpoint ·
relay auto-failover 429/quota · compare (fork ต่อ agent) · projector เขียน agent files · memory `memory_propose`→approve queue · per-hunk diff reject · interactive permission

**ยังต้องเพื่อน (แต่ไม่บล็อกอะไร)**:
- **Gemini CLI — เอาออกแล้ว (20 ต.ค. 2026)** ไม่เคยมีใน `list_agents` ไม่ได้ติดตั้งบนเครื่องเพื่อน และไม่เคยถูกทดสอบ ฝั่ง Google ใช้ **Antigravity (`agy`)** ผ่าน `antigravity-acp` · การนำเข้า transcript ของ Gemini CLI (7.23) ยังอยู่
- **ทดสอบกับ agent จริงทั้ง flow** — `scripts/smoke-real-agents.sh [claude codex antigravity]` บน Mac ที่ login แล้ว: เขียนไฟล์ได้ · agent ตัวที่สองรู้ว่าตัวแรกทำอะไร (จาก ledger) · relay ข้าม bridge ที่พัง — **ยังไม่เคยรันกับ Claude/Antigravity จริง**
- **Claude bridge** — package ถูก rename เป็น `@zed-industries/claude-agent-acp` (ตัวเก่า `claude-code-acp` deprecated) · `bridge_for` ชี้ตัวใหม่แล้ว ถ้าเครื่องยังมีแต่ตัวเก่าให้ตั้ง `CHORUS_CLAUDE_BRIDGE="npx -y @zed-industries/claude-code-acp"` · เครื่องนี้ `claude` OAuth หมดอายุ → login ใหม่
- **Ollama compaction** — ใช้ได้จริงแล้วผ่าน `chorus compact` (ไม่อยู่บน turn path อีกต่อไป) · `brew install ollama && ollama pull qwen2.5:3b` แล้วตั้ง `CHORUS_SUMMARY_MODEL`
- **embedding + sqlite-vec** — ต้องตัดสิน dep (MLX / `NLEmbedding` / sqlite-vec)
- **Seatbelt sandbox · notarize · .app bundle** — ต้อง Apple Developer account

### ตัวเลขจาก `chorus bench` (78 events) — baseline ที่ห้ามถอยหลัง

```
อ่านไฟล์เดิม 8 ครั้ง → blob rows = 1 (naive = 8)   ประหยัด 99.6%
ledger ดิบ 103,553 token
  @200k window → 16,226 token (15.7%)   demoted/dropped 0/0
  @6k  window →  2,490 token (2.4%)     demoted/dropped 40/11
store.db 172 KB / 78 events   (schema v7 · เดิม 100 KB ตอน v3 — ไม่ใช่ invariant แต่จับตาไว้)
```

**Invariant 3 ข้อ** — ถ้า bench แสดงผิดจากนี้แปลว่ามี regression:
1. `blob rows = 1` (dedup ทำงาน)
2. assembled เป็นเศษเสี้ยวของ raw
3. **demoted ต้องมาก่อน dropped เสมอ** — ย่อก่อนทิ้ง ไม่ใช่ตัดท้าย
4. **task state ต้องไม่ว่าง** บน session ที่พูดอะไรไปแล้ว (bench section `real path`)
5. **assemble_context ต้องไม่เรียก summariser** — bench ตั้ง `CHORUS_SUMMARY_MODEL` แล้ววัดเวลา ถ้าเกิน 2 วิ = regression

> bench เดิมป้อน `task_state` ให้ตัวเองแล้วรายงานว่าผ่าน ทั้งที่ของจริงส่ง `String::new()`
> มาหลายเดือน — harness ที่ป้อน input ให้ตัวเองวัดได้แค่ตัวเอง ข้อ 4–5 เลยรันผ่าน
> `Session::assemble_context` ตัวจริง ไม่ใช่ `AssembleInput` ที่ประกอบมือ

---

## 4. คำสั่ง

### ติดตั้งเป็นคำสั่ง `chorus` (ไม่บังคับ)

```bash
cargo install --path crates/chorus-cli    # ได้คำสั่ง chorus
cargo install --path crates/chorus-acp    # ได้ acp-probe ซึ่ง `chorus demo` ต้องใช้
```

**ต้องลงทั้งสองอัน** — `chorus` กับ `acp-probe` อยู่คนละ crate และ `chorus demo` spawn
`acp-probe --serve-mock` เป็น agent ปลอม ถ้าลงแค่อันแรก demo จะขึ้น error ที่บอกวิธีแก้ให้เอง
ไม่ลงก็ได้ ใช้ `cargo run -p chorus-cli -- <คำสั่ง>` ในโฟลเดอร์โปรเจกต์แทน

```bash
cargo test                              # ~377 tests (6 ตัวผ่านเฉพาะ macOS — ดู 7.22)
cargo run -p chorus-cli --bin chorus -- bench   # วัด invariant (ต้องใส่ --bin chorus เพราะ crate มีหลาย binary)
cargo run --bin acp-probe -- --cwd /tmp/acp-scratch --mock         # ลอง ACP กับ agent ปลอม
cargo run -p chorus-acp --example codex_smoke -- /tmp/codex-smoke   # CodexAgent กับ bridge จริง (ต้อง codex login)

# session engine จริง (ต้อง codex login + npx)
cargo run -p chorus-cli -- run <cwd> "<prompt>" --db /tmp/s.db [--yes]
cargo run -p chorus-cli -- run "<prompt>" --session 1 --db /tmp/s.db          # เทิร์นต่อ
cargo run -p chorus-cli -- run <cwd> "<prompt>" --chain codex,claude-opus --db /tmp/s.db   # relay
cargo run -p chorus-cli -- handoff --db /tmp/s.db --session 1 --to codex --reason rate_limit --goal "..."
cargo run -p chorus-cli -- compare "<prompt>" --db /tmp/s.db --session 1 --agents codex,claude-opus
cargo run -p chorus-cli -- memory pending --db /tmp/s.db          # approve queue
cargo run -p chorus-cli -- compact --db /tmp/s.db --session 1 [--limit N]   # เติม warm tier (L1/L2)
cargo run -p chorus-cli -- demo          # เล่าทั้งเรื่องด้วย agent ปลอม ไม่ต้องมี API key
scripts/smoke-real-agents.sh            # e2e กับ agent จริงที่ login ไว้ (ส่ง request จริง ไม่กี่ครั้ง)
cargo run -p chorus-cli -- secrets [list|forget NAME|scan FILE]   # ชื่อ secret ที่จำไว้ (ไม่มีค่า) · scan บอกแค่บรรทัด+ชนิด
cargo run -p chorus-cli -- doctor        # ทำไมเทิร์นถึงรัน/ไม่รัน — agent, login, bridge, env, error ที่ failover ไม่รู้จัก
cargo run -p chorus-cli -- sessions list                     # session ของ Claude Code / Codex / Gemini CLI ในเครื่อง
cargo run -p chorus-cli -- sessions search "<คำ>"            # ค้นชื่อ/โฟลเดอร์/เนื้อหา
cargo run -p chorus-cli -- sessions import codex <ID> --db /tmp/s.db   # นำเข้าเป็น session แล้ว `run --session N` คุยต่อ

# macOS shell
apps/macos/build.sh                     # cargo build -p chorus-ffi + swift build + bundle + open
```

**รัน `cargo test` และ `bench` ทุกครั้งที่แก้ core** — bench คือตัวที่จับ regression ที่ test จับไม่ได้
ไม่มี rustfmt.toml — โค้ดสไตล์กว้างกว่า default **ห้าม `cargo fmt` ทั้ง crate** (จะรื้อไฟล์อื่น)
format เฉพาะไฟล์ที่แก้: `rustfmt --edition 2021 <file>`  ·  clippy ตามปกติ (ตอนนี้ clean)

---

## 5. แผนที่ repo

```
crates/chorus-core/src/
  types.rs      Event/EventKind/BlobId/Rung — สัญญากลาง อย่าแก้เล่น
  schema.rs     migration + FTS5 (มี fallback ถ้า fts5 ไม่มี)
  store.rs      เปิด db, ตั้ง WAL/pragma
  actor.rs      intern ชื่อ agent เป็น integer
  blob.rs       content-addressed: BLAKE3 + zstd + dedup + GC
  ledger.rs     append/get/tail/fork/search + history() (fork-chain walk) — DAG, fork ไม่ copy อะไรเลย
  memory.rs     fact store (≤140 ตัวอักษร) + render ไม่มี metadata + propose/pending/approve/reject
  tokens.rs     ตัวประมาณ token ที่รู้จักภาษาไทย + cache
  render.rs     table/manifest/truncate/dedup_window — รูปแบบที่ส่งเข้าโมเดล
  context.rs    budget allocator + rung ladder L0→L1→L2
  handoff.rs    Brief::build + render_markdown + to_json + HandoffReason::from_failure (relay classifier) — pure
  projector.rs  render memory → CLAUDE.md/AGENTS.md/GEMINI.md + drift detect (marker + hash)
  checkpoint.rs shadow git repo <cwd>/.chorus/ckpt — snapshot/diff_since/restore (shell ออก git)
  compaction.rs Summarizer trait · Extractive (model-free) + Ollama · best() เลือกให้
                + warm tier: save_rungs/load_session_rungs/uncompacted — L1/L2 เก็บในตาราง `summary`
  external_sessions.rs  หา + นำเข้า session ของ Claude Code/Codex/Gemini CLI จากดิสก์ (ดู 7.23)
  mcp.rs        Streamable-HTTP MCP server (std::net, ไม่มี dep) — tool memory_propose · ทำงานกับ Codex แล้ว (persistent conn + held-open SSE)
crates/chorus-cli/src/main.rs           bench + run + handoff + memory + compare subcommands
crates/chorus-cli/src/session.rs        Session engine — run_turn/run_relay/handoff/compare/assemble_context/task_state/compact/events_since
crates/chorus-acp/src/lib.rs            ACP client (Agent) + mock agent module (--fail สำหรับเทสต์ relay)
crates/chorus-acp/src/codex.rs         CodexAgent adapter + bridge_for(name) — start/load/prompt → TurnOutcome
crates/chorus-acp/src/bin/acp_probe.rs  สไปก์ที่ 1 (probe)
crates/chorus-ffi/src/lib.rs           C ABI: engine_open/reopen/run_turn/events_since/handoff (+ include/chorus.h)
apps/macos/Sources/Chorus/             SwiftUI shell — ChorusApp / ContentView / Engine (glue เท่านั้น)
crates/chorus-acp/examples/codex_smoke.rs  ยิง CodexAgent กับ bridge จริง (ต้อง codex login)
evals/handoff/                          สไปก์ที่ 2 (README + เกณฑ์ + score.py + cases)
```

---

## 6. ข้อตัดสินใจที่ทำไปแล้ว — อย่ารื้อโดยไม่ถามก่อน

| เรื่อง | ตัดสินใจว่า | เพราะ |
|---|---|---|
| ที่เก็บ ledger | **SQLite ไม่ใช่ JSONL** | JSONL ไม่มี index, parse ทั้งไฟล์, key ซ้ำทุกบรรทัด |
| `seq` | monotonic integer ไม่ใช่ uuid | 8 ไบต์ vs 36 |
| `t` | ms offset จาก `session.started_ms` | ไม่ใช่ ISO string |
| `kind` / `actor` | integer enum + ตาราง lookup | ไม่เก็บ `"tool_result"` ซ้ำ 5,000 ครั้ง |
| `parent` | pointer เดียวขึ้นข้างบน (DAG) | fork = 1 แถว ไม่ copy history |
| hash | BLAKE3 | SIMD บน ARM เร็วกว่า SHA-256 |
| `blob.zstd` | คอลัมน์บอกว่าเก็บบีบหรือดิบ | บางทีบีบแล้วใหญ่ขึ้น |
| FTS5 | สร้างใน block ที่ไม่ fatal + fallback เป็น LIKE | portability |
| token ไทย | **1 token/ตัวอักษร** | ดูข้อ 7.1 |
| macOS shell | SwiftUI ตัวจริงเลย ไม่แวะ Tauri | โฟกัส macOS อย่างเดียวแล้ว |
| mock agent | อยู่ใน binary เดียวกับ probe (`--mock`) | กัน "ไม่ได้ build / path ผิด" |
| permission ใน probe | **ปฏิเสธเป็นดีฟอลต์** และไม่เขียนไฟล์เลยแม้ใส่ `--allow` | อยากเห็นรูปร่างคำขอ ไม่ได้อยากปล่อย agent เข้าโปรเจกต์จริง |
| L1/L2 | เก็บใน **ตาราง `summary` แยก** ไม่ใช่ `EventKind::Summary` event | summary เป็น metadata ของเทิร์น ไม่ใช่สิ่งที่ใครพูด — ใส่ใน ledger จะทำให้ `history()` กับตัวเลข dedup เพี้ยน |
| ใครสร้าง L1/L2 | `chorus compact` เท่านั้น **ห้าม** ทำใน `assemble_context` |
| compact หนึ่งรอบ | หยุดที่ `limit` **หรือ** เพดานเวลา 3 วิ (`CHORUS_COMPACT_BUDGET_MS`) แล้วแต่อันไหนถึงก่อน | มันใช้ serial queue ร่วมกับเทิร์น เพดานเวลาจึงเป็นเพดานที่ user ต้องรอด้วย · เช็ค*หลัง*เขียนเสมอ เพื่อให้ทุกรอบคืบหน้าอย่างน้อย 1 event |
| โหมดส่งงาน | ไม่มีให้เลือก — ทุกเทิร์นวิ่งผ่าน relay เสมอ (chain ยาว 1 = single) | failover ติดมา default โดย user ไม่ต้องรู้จักคำว่า relay |
| handoff | ปุ่มเดียว ไม่มีฟอร์ม (`handoff_auto` + `task_state`) | ให้คนกรอกสิ่งที่ ledger รู้อยู่แล้วคือการให้พิมพ์ซ้ำ |
| รายการโมเดล | curated ใน `agents.rs` + override ด้วย env | **ACP ไม่มีเมธอดถามรายการโมเดล** อย่าเข้าใจผิดว่าเป็นข้อมูลจาก provider |
| auth detection | `which <cli>` + เช็คว่ามีไฟล์ credential อยู่ไหม (`.exists()` เท่านั้น) | **ห้ามอ่าน/พิมพ์ไฟล์ credential เด็ดขาด** | read path ที่เรียก summariser = 1 model call ต่อ event ต่อเทิร์น (400 events → 800 HTTP) |
| fallback ตอนยังไม่ compact | `Extractive` เสมอ ไม่ใช่ `best()` | `best()` อาจไปเรียก Ollama ซึ่งพา network เข้ามาบน critical path |
| task state | derive จาก ledger (goal = ข้อความแรกของ user, pending/constraints = handoff brief ล่าสุด, done = Decision + L2 ที่เก็บไว้) | ไม่ต้องเพิ่ม state ใหม่ และไม่มีวันว่างบน session ที่พูดอะไรไปแล้ว |
| MCP body | เพดาน 64 KB → 413 · `Origin` ใด ๆ → 403 · read timeout 30 นาที | `vec![0u8; content_length]` ตาม header = OOM จาก header บรรทัดเดียว · Origin guard คือกัน DNS-rebinding ตามที่ MCP spec แนะนำ |
| plan mode | เทิร์นปกติ + `Policy::ReadOnly` + `PLAN_MODE_GUIDANCE` ต่อท้าย prompt (ไม่ลง ledger) · อนุมัติ = ส่ง `PLAN_APPROVED_PROMPT` ด้วยสิทธิ์ปกติของ user | ใช้ sandbox ที่มีอยู่บังคับ ไม่ใช่คำขอร้อง · ข้อความอนุมัติอยู่ใน ledger เพื่อให้ agent ถัดไปรู้ว่าแผนผ่านแล้ว |
| rewind | ย้อนแชท = `ledger::fork` ก่อนข้อความนั้น (ไม่ลบ) · ย้อนไฟล์ = sha ของ turn guard ที่ bind กับ seq ของข้อความ user (`checkpoint_turn`) + snapshot ก่อนย้อนเป็น backup | ledger เป็น SoT (2.2) — `CheckpointDag::rewind_to_seq` เดิมใช้ `DELETE FROM event` ตอนนี้แอปไม่เรียกแล้ว **อย่ากลับไปใช้** |
| browser | **extension ของแต่ละเจ้า ไม่ใช่ browser ของเรา** (เพื่อนตัดสิน 20 ต.ค. 2026 — เอา Playwright ออกแล้ว) · Claude = `CLAUDE_CODE_ENABLE_CFC=1` บน bridge (`provider_browser_env`) · Codex: extension มีเฉพาะในแอป Codex · Google ในแอปคือ **Antigravity (`agy`)** ไม่ใช่ Gemini CLI — มี browser subagent ในตัว ไม่ต้องเปิดอะไรจากเรา แต่ยังไม่ได้ทดสอบว่ารันได้ใต้ `--input-format stream-json` + Seatbelt · Chrome DevTools ยังอยู่ใน catalog (`--isolated --no-usage-statistics --no-performance-crux`) | **ห้ามเปิดให้ Seatbelt เขียน `NativeMessagingHosts/`** — Chrome รัน binary ที่ manifest ชี้นอก sandbox = ทางหนี sandbox ให้ user รัน `claude --chrome` ใน Terminal เองครั้งแรก |
| computer use | MCP tool `computer` ของ Chorus เอง (`chorus-core/src/computer.rs`) ไม่ใช้ community server · CoreGraphics + `screencapture`/`sips` ไม่มี dep ใหม่ | สิทธิ์ TCC อยู่กับ Chorus.app เท่านั้น · ทุก action ผ่าน gate: ปิดเป็นดีฟอลต์ · read-only/plan/orchestrator = ดูได้อย่างเดียว · ถามก่อน = ถามทุก action ผ่าน callback เดียวกับ permission · cap 300/เทิร์น · `TurnGuard` ปิด gate เมื่อเทิร์นจบ |
| secrets | 3 ชั้นใน `chorus-core/src/secrets.rs`: **mask** ทุกข้อความที่ผ่าน Chorus (user/agent/แนบไฟล์/memory/import/ชื่อ session) เป็น `$CHORUS_SECRET_<KIND>_<N>` · **inject** ค่าจริงเป็น env ของ bridge เฉพาะชื่อที่ prompt อ้าง · **block** Seatbelt `deny file-read-data` บน `.env*`/`*.pem|key|p12|pfx|jks|keystore` ใน workspace แล้ว Chorus อ่าน `.env` เองส่งเป็น env | ค่าอยู่ใน login Keychain (`security -i` ทาง stdin, hex) · ดิสก์มีแค่ชื่อ+BLAKE3 (`~/.chorus/secrets.json`) · ไม่มี regex dep — scanner เขียนมือ |
| bridge ของแต่ละเจ้า | override ได้ด้วย env (`CHORUS_CLAUDE_BRIDGE` ฯลฯ) | package ถูก rename มาแล้วครั้งหนึ่ง ไม่ควรต้อง rebuild เพื่อตามชื่อ |

---

## 7. กับดักที่เคยเหยียบมาแล้ว

### 7.1 อย่านับ token ด้วย `len/4`
ภาษาไทยเป็น 3 ไบต์/ตัวอักษรใน UTF-8 และไม่มีช่องว่างระหว่างคำ
`len/4` ทำให้ไทยดู "ถูกกว่า" อังกฤษต่อไบต์ ซึ่งกลับด้านกับความจริง
ค่าที่ใช้อยู่: **ไทย/CJK 1 token/ตัวอักษร, ละติน 4 ตัวอักษร/token**
มีเทสต์ `thai_estimated_denser_than_english_same_byte_length` กันการ regress ไว้แล้ว

`HeuristicCounter` เป็นแค่ตัวประมาณสำหรับจัดงบ — **ก่อนยิงจริงต้องใช้ tokenizer จริงของแต่ละเจ้า**
(tiktoken / count-tokens endpoint) แล้วแคชผลไว้กับ hash ของเนื้อหา

### 7.2 อย่าให้ agent เขียน memory ตรง ๆ
ผ่าน MCP tool `memory_propose` → approve queue (`chorus memory` / Memory pane) — **ทำแล้ว**
ปล่อยอิสระเมื่อไหร่ ความจำจะเต็มไปด้วยข้อสรุปผิดภายในไม่กี่วัน

### 7.2b assemble_context ห้ามสร้างอะไรทั้งนั้น
มันรันทุกเทิร์น ก่อน agent จะเห็น prompt ด้วยซ้ำ ถ้ามันเรียก summariser ขึ้นมาเมื่อไหร่
session ที่มี 400 event จะกลายเป็น 800 model call ต่อหนึ่งเทิร์น — เคยเป็นแบบนั้นมาแล้ว
และไม่มีใครเจอเพราะ default เป็น `Extractive` ที่เร็วจนมองไม่เห็น
**อ่านจากตาราง `summary` เท่านั้น** งานสร้างอยู่ที่ `chorus compact`

### 7.6b failover classifier ต้องตรงกับข้อความจริง
`HandoffReason::from_failure` เคยหาแค่ `rate limit` / `429` / `quota` แต่ข้อความจริงคือ
`usageLimitExceeded` (Codex) กับ `out of extra usage` (claude-agent-acp) ซึ่งไม่ตรงสักคำ →
**ฟีเจอร์พาดหัวของแอปไม่ทำงานเลย** โดยที่เทสต์ 69 ตัวผ่านหมด เพราะ mock ถูกสั่งให้พูดคำที่ classifier รู้จัก
เจอข้อความลิมิตแบบใหม่เมื่อไหร่ **เพิ่มทั้งใน `from_failure` และใน `tests/handoff.rs`**

### 7.7 compaction ใช้ queue เดียวกับเทิร์นโดยตั้งใจ
`Engine.compactIfIdle` วิ่งบน `queue` ตัวเดียวกับ `send` เพราะ engine handle ฝั่ง Rust
ถือ SQLite connection ที่แตะจากสองเธรดพร้อมกันไม่ได้ — ความ serial ของ `queue` คือสิ่งที่กัน
data race อยู่ตอนนี้ **อย่าย้ายไป queue ใหม่เพื่อให้ "ไม่บล็อก"** ถ้าจะขนานจริงต้องเปิด
engine handle คนละตัว แต่ละพาสเลยตั้ง limit ไว้แค่ 40 event — session ยาว ๆ ไล่ตามทีละนิด
ข้ามหลายเทิร์น แทนที่จะให้เทิร์นถัดไปรอพาสใหญ่พาสเดียว

### 7.8 `cargo run -p chorus-cli` ไม่ได้ build binary ของ crate อื่น

`acp-probe` อยู่ใน `chorus-acp` — `cargo run -p chorus-cli -- demo` build แค่ *library*
ของ crate นั้น ไม่ได้ build binary ให้ ผลคือ demo ไปเรียก `target/debug/acp-probe`
ตัวเก่าเงียบ ๆ แล้วแสดงพฤติกรรมของเมื่อวาน (เคยเกิดจริง: mock ที่แก้ path แล้ว
ยังรายงาน `/tmp/mock/README.md`). `mock_bridge()` จึงเตือนก่อนรัน

เทียบกับ **โค้ดของ chorus-acp** ไม่ใช่กับตัว binary เอง: `cargo build --workspace`
ครั้งเดียว compile chorus-acp ก่อน chorus-cli อยู่แล้ว ถ้าเทียบ binary-กับ-binary
มันจะเตือนทุกครั้งเพราะแก่กว่ากันเป็นมิลลิวินาที และจะ *ไม่* เตือนในเคสที่ควรเตือน
(แก้ mock แล้ว cargo ไม่ rebuild อะไรเลย) — path ของ source ฝังตอน build ถ้าเป็น
binary ที่ install แล้วก็ข้ามการเช็คไปเลย

### 7.9 บน macOS Claude ไม่มีไฟล์ credential ให้เช็ค

`claude` บน macOS ย้าย token เข้า login Keychain (service `Claude Code-credentials`)
แล้ว **ลบ** `~/.claude/.credentials.json` ทิ้ง — เช็คแค่ไฟล์เลยขึ้น `needs_login`
ตลอดกาลบนเครื่องที่เพิ่ง login สำเร็จ (เจอจริง 12 ก.ย. 2026) `claude_logged_in()`
จึงเช็คไฟล์ก่อน (Linux / เครื่องที่วาง token เอง) แล้ว fallback ไป Keychain

เช็ค Keychain ด้วย `security find-generic-password -s ...` **ห้ามใส่ `-w`** —
`-w` พิมพ์ secret ออกมาและเด้ง prompt ขออนุญาต; ไม่ใส่ก็แค่ lookup แล้วเราอ่าน
เฉพาะ exit status (44 = ไม่เจอ) โดย null ทั้ง stdout/stderr ไม่ให้ไบต์ของ
credential เข้ามาใน process เลย — Codex ยังใช้ `~/.codex/auth.json` ตามเดิม

### 7.10 ACP mode คุม Codex ไม่ได้ — `--read-only` ต้องบังคับเอง (วัดจริง 12 ก.ย. 2026)

ทดลองกับ `@agentclientprotocol/codex-acp` ตัวจริง:

- `session/set_mode` → ตอบ `{}` แต่โหมดไม่เปลี่ยน (ยังเป็น `agent`)
- `session/set_config_option {configId:"mode", value:"read-only"}` → ตอบกลับ
  `currentValue: "read-only"` **จริง** คือรับรู้และรายงานถูก
- แต่ codex ยัง **เขียนไฟล์ได้** ทั้งใน cwd และ **นอก cwd** (`/tmp/outside9/O.txt`)
  โดยไม่ส่ง `session/request_permission` แม้แต่ครั้งเดียว

สรุป: `Policy::decide` เป็นแค่ "คำตอบ" ไม่ใช่การบังคับ — agent ที่ไม่ถาม ไม่มีอะไรหยุด
`claude-agent-acp` หนักกว่า: ไม่ตอบทั้ง `set_config_option` และ `set_mode` เลย (`via: none`)

เคยเขียนให้ **ไม่รันเทิร์น** ถ้า agent ไม่ยืนยันโหมด ปรากฏว่าผิดเลเวอร์ — มันบล็อก
ทุกเทิร์นของ Claude ในแอป ขณะที่ Codex ซึ่ง *ยืนยัน* กลับเขียนไฟล์ได้อยู่ดี
การยืนยันโหมดจึงทำนายอะไรไม่ได้เลย ตอนนี้เหลือแค่ `note_mode()` เตือนเฉย ๆ

ของจริงที่รักษาสัญญาคือ snapshot shadow repo ก่อนเทิร์น แล้วเช็คหลังจบ ถ้ามีไฟล์เปลี่ยน
→ `restore()` + `bail!` (`revert_if_touched`) ซึ่งไม่ต้องพึ่งความร่วมมือจาก agent เลย
เงื่อนไขที่ทริกเกอร์: policy ที่จำกัด (CLI) หรือ — ในแอป — user กด Deny **หรือ agent
ไม่ถามอะไรเลยสักครั้งแล้วเขียนไฟล์** (เคสของ Codex: คนนั่งอยู่หน้าโหมด "ถามก่อน"
แต่ไม่เคยถูกถาม)

**ตอนนี้มี Seatbelt แล้ว** (ดู 7.20) การย้อนไฟล์เหลือเป็นชั้นที่สอง ไม่ใช่ชั้นเดียว

ห้ามโฆษณา `--read-only` ว่าเป็น sandbox จนกว่าจะมี Seatbelt

### 7.11 done/pending ว่างเปล่าใน session จริง — mock หลอกเราเอง

`task_state().done` เคยอ่านจาก Diff event ซึ่งเกิดเฉพาะตอน **client** เป็นคนเขียนไฟล์
Codex shell ออกไปเขียนเอง → ไม่มี Diff event → brief บอก agent ถัดไปว่า "ไม่มีอะไรถูกแตะ"
ทั้งที่เพิ่งสร้างไฟล์ไป เดโมดูแน่นเพราะ mock ยิง Decision + Diff ให้เอง

แก้: อ่านจาก shadow repo (`Checkpoint::files_since`) ซึ่งเห็นทุกกรณี ใช้
`git diff --name-only <base>` + `git ls-files --others` แทน `diff_since` เพราะอันนั้น
`git add -A` ก่อน — mutate index บน read path ที่รันทุกเทิร์น และช้าใน repo ใหญ่
กรอง `CLAUDE.md`/`AGENTS.md`/`GEMINI.md` ออก เพราะเป็นไฟล์ที่ **เราเขียนเอง**

### 7.12 tool result ที่ไม่มีเนื้อ กิน context ฟรี

ทุก tool call กิน 2 บรรทัดใน conversation tier และบรรทัดผลลัพธ์เป็น `X → completed`
ที่ไม่เพิ่มข้อมูลอะไรจาก tool call ข้างบนเลย — วัดจาก session จริงคือ ~30% ของชั้นนั้น
`is_bare_success()` ตัดเฉพาะ `completed|success|ok` เท่านั้น ส่วน `failed`/สถานะแปลก ๆ
ปล่อยผ่านหมด เพราะค่าเสียหายของการทิ้งบรรทัดที่สำคัญสูงกว่า token ที่ประหยัดได้มาก

### 7.13 เทสต์ที่ผูกกับความยาว path จะพังเฉพาะ macOS

`assemble_context_reads_the_stored_rung...` ใช้ budget คงที่ 700 token และ fixture
ที่ทุก event ซ้ำ absolute path — tempdir ของ macOS
(`/private/var/folders/xx/<28 ตัว>/T/...`) ยาวกว่า `/tmp` ของ Linux ~70 ตัวอักษร
เทิร์นเก่าสุดเลยหลุด budget ก่อนจะถูก demote ลง rung → ผ่านบน Linux พังบน macOS
โดยไม่เกี่ยวกับ read path เลย แก้ด้วย `ShortDir` (path สั้นคงที่) ไม่ใช่ขยาย budget

### 7.14 Picker ของ SwiftUI ที่ selection ไม่ตรง tag จะว่างเปล่าเงียบ ๆ

ช่อง Agent ในแอปว่างมาตลอดเพราะ seed ด้วย `model = "codex"` (agent id) แต่ tag ของ
Picker เป็น **model id** (`gpt-5.1-codex`) SwiftUI ไม่ error ไม่ fallback — มันเรนเดอร์ว่าง
ทำให้แยกไม่ออกว่า detection พัง หรือแค่ยังไม่วาด แก้ด้วย `syncModelSelection()`
ที่ seed จาก agent ที่ `ready` จริงหลัง `loadAgents()` กลับมา

### 7.15 สิทธิ์เหลือสองปุ่ม

เดิมมีสี่: ask / deny / read-only / allow — หลังจากรู้ว่า agent ไม่ถามอนุญาตเสมอไป
deny กับ read-only ให้ผลเหมือนกันเป๊ะ (โหมดจำกัด + ย้อนไฟล์) ส่วน deny-ทุกอย่าง
ใช้ไม่ได้จริงอยู่แล้ว (agent ที่อ่านไม่ได้ก็ตอบไม่ได้) เหลือ **ถามก่อน** กับ **ให้ทำเลย**
ตัวเลือกที่ต่างกันแค่ในทฤษฎีคือตัวเลือกที่ทำให้คนต้องเดา

### 7.16 Skill registry — เขียนที่เดียว ฉายตามที่แต่ละเจ้าอ่านได้

ต้นฉบับอยู่ที่ `.chorus/skills/<name>/SKILL.md` แล้วฉายออกสองแบบ:

- **Claude** ได้ skill จริง เขียนลง `.claude/skills/<name>/SKILL.md` ตาม path ที่ loader
  มันหา (ยืนยันจาก code.claude.com/docs/en/skills 12 ก.ย. 2026)
- **Codex / Gemini** ไม่มีระบบ skill เลย → ได้ index บรรทัดละ skill ใน AGENTS.md
  (ชื่อ | ใช้เมื่อไหร่ | path) พอให้มันเปิดไฟล์อ่านเองได้ โดยไม่ต้องยัดทั้งเอกสารเข้า prompt

**ห้ามใส่ index ให้ Claude ด้วย** — มันอ่านไฟล์จริงอยู่แล้ว ใส่ซ้ำ = จ่าย token ซ้ำ

marker ของ skill ต้องอยู่ **ท้ายไฟล์** เพราะ frontmatter ต้องเป็นบรรทัดแรกไม่งั้น
loader ของ Claude parse ไม่ออก → ต้องใช้ `drift_raw()` แยกจาก `drift()` ถ้าใช้ตัวเดิม
ทุก skill จะถูกมองเป็น `Foreign` แล้ว**เลิกอัปเดตอย่างเงียบ ๆ**

### 7.17 ข้อความ limit จริงตัวแรกที่เคยเห็น (12 ก.ย. 2026)

```
Internal error: You've hit your limit · resets 9:50pm (Asia/Bangkok)
```

จาก `claude-agent-acp` ระหว่างเทสต์ skill — **ไม่ตรงกับ key ไหนเลย** ที่ classifier มีอยู่
ตอนนั้น (ไม่ใช่ "usage limit", ไม่ใช่ "rate limit", ไม่มี 429) เพราะ key ทุกตัวมาจาก
eval log ซึ่งรู้แค่สำนวนที่บังเอิญอยู่ในนั้น เพิ่ม `"hit your limit"` แล้ว + เก็บข้อความ
เต็มไว้ใน `tests/handoff.rs` **แบบคำต่อคำ** (ถ้า paraphrase เทสต์จะผ่านทั้งที่ของจริงยังพัง)

หลังแก้ รัน `--chain claude,codex` ตอน Claude ติดลิมิตจริง → `relay: claude → codex
(quota exhausted)` แล้ว codex ทำต่อได้ **ครั้งแรกที่ failover ทำงานกับลิมิตของจริง
ไม่ใช่ของจำลอง**

### 7.18 MCP config ก้อนเดียว — ACP ให้ client เป็นคนตัดสิน

`session/new` รับ `mcpServers` และ **สิ่งที่อยู่ในนั้นคือทั้งหมดที่ agent เอื้อมถึงได้**
ไม่เกี่ยวกับ config ของ CLI ตัวนั้นเลย → เก็บที่ `.chorus/mcp.json` ที่เดียว
แล้วส่งชุดเดียวกันให้ทุกเจ้า (`chorus mcp import` ดูด `.mcp.json` เดิมมาให้)

รูปแบบต่างกันตรงที่คนมักพลาด: `env` กับ `headers` ของ ACP เป็น
**array ของ {name, value}** ไม่ใช่ object แบบที่ไฟล์ config ทุกตัวบนดิสก์เขียนกัน
→ `mcp_config::pairs()` แปลงให้

**ห้าม print ค่า env/headers** ทุกกรณี — `redact()` คืนแค่ *ชื่อ* ตัวแปร
(`GITHUB_TOKEN` พอให้ debug ได้ ส่วนค่ามันพอให้คนอื่นเอาไปใช้) มีเทสต์กันไว้ว่า
ค่าจริงต้องไม่โผล่ใน `{:?}` · `CHORUS_ACP_LOG` ที่ dump ทุกเฟรม **มีค่าจริง**
จึงตั้งเป็น env var ที่ต้องพิมพ์เอง ไม่ใช่ flag

### 7.19 รายชื่อโมเดลต้องถาม ไม่ใช่เดา

เคยเขียนใน `agents.rs` ว่า "ACP ไม่มีเมธอดสำหรับ list model" — จริงเรื่องเมธอด
แต่ผิดเรื่องข้อสรุป: **รายการอยู่ใน reply ของ `session/new`** (`models.availableModels`
+ `currentModelId`) ของเดาที่ใส่ไว้ผิดทั้งสองเจ้า

| | ที่เดาไว้ | ของจริง (วัด 12 ก.ย. 2026) |
|---|---|---|
| Codex | `gpt-5.1-codex`, `-mini` | `gpt-6-astra` / `gpt-5.6-sol` / `terra` / `luna` / `gpt-5.5` × 6 ระดับ reasoning (27 ตัว) ดีฟอลต์ `gpt-5.6-luna[high]` |
| Claude | `claude-opus-4-6`, `claude-sonnet-4-6` | `default` · `opus` · `haiku` |

การถามต้องเปิด session จริง (spawn npx) → ช้าเกินกว่าจะทำตอนวาด picker
`chorus models` จึงถามแล้ว cache ลง `~/.cache/chorus/models-<agent>.json`
ส่วน `list_agents` อ่าน cache · ค่าเดาเหลือไว้เป็น fallback และ **บอกตัวเองว่าเป็นค่าเดา**
ผ่าน `ModelSource::Fallback`

**เลือกโมเดลมีผลจริงแล้ว** ผ่าน `Session::set_model()` → `CodexConfig.model` →
`session/set_model` ตอนเปิด session (CLI: `--model ID` · FFI: `chorus_set_model`)

⚠️ **id มีสอง namespace ห้ามสลับกัน** — พลาดแล้วได้ `-32602 Invalid params`:

| ทาง | id ที่รับ | ตัวอย่าง |
|---|---|---|
| `session/set_model` | จาก `models.availableModels[].modelId` | `gpt-5.5[low]` |
| `set_config_option {configId:"model"}` | id เปล่า ๆ + `reasoning_effort` แยก | `gpt-5.5` |

เราโชว์ id แบบแรกให้ user เพราะมันบอกว่าจะรันอะไรจริง ๆ `pin_model()` เลยยิง
`session/set_model` ก่อน แล้ว fallback เป็น config option โดยตัด `[...]` ทิ้ง
(`session/set_model` สเปกยังระบุว่า unstable — fallback จึงไม่ใช่ของเผื่อ)

ถ้าขอโมเดลแล้ว agent ตอบกลับเป็นคนละตัว **ต้องเตือน** ไม่ใช่เงียบ —
คนที่คิดว่าสลับโมเดลแล้วกำลังอ่านผลจากโมเดลเดิมคือความผิดพลาดที่มองไม่เห็น

### 7.20 Seatbelt — กันตอนเขียน ไม่ใช่ย้อนทีหลัง (วัดจริง 13 ก.ย. 2026)

`sandbox::wrap()` ครอบคำสั่ง bridge ด้วย `sandbox-exec -p` ก่อน spawn
โปรไฟล์คือ `(allow default)` + `(deny file-write*)` แล้วอนุญาตเขียนกลับเฉพาะที่จำเป็น

**จำกัดแค่ `file-write*` เท่านั้น** — อ่าน/เน็ต/spawn ปล่อยหมด agent ที่อ่านเครื่อง
ไม่ได้หรือต่อ API ตัวเองไม่ได้ไม่ใช่ agent และรูที่วัดเจอจริงคือการเขียน

ที่ต้องอนุญาตกลับ ไม่งั้น agent พังแบบดูเหมือนบั๊กของเรา:
`~/.codex` `~/.claude` (auth + session state) · `~/.npm` `~/.cache` `~/.config`
`Library/Caches` · temp ทั้งหมด · `/dev`

⚠️ **path ต้อง canonicalize** — macOS ให้ `/tmp` กับ `/var` เป็น symlink ไป `/private`
และ Seatbelt แมตช์ path ที่ resolve แล้ว เขียนโปรไฟล์ด้วย `/tmp` = ไม่อนุญาตอะไรเลยเงียบ ๆ

⚠️ **อย่าเทสต์นอก workspace ด้วยโฟลเดอร์ใน `/tmp`** — `/private/tmp` อยู่ใน allow list
อยู่แล้ว (node ต้องใช้) เทสต์แรกผมวางไว้ที่ `/tmp/outside-sbx` เลยผ่านฉลุยและดูเหมือน
sandbox ไม่ทำงาน ต้องใช้ `~/` ถึงจะวัดของจริง

ผลจริง (Profile::Workspace, `--yes`): แก้ `repo.ts` ใน workspace **สำเร็จ** ·
เขียน `~/chorus-sbx-out/O.txt` **ถูกปฏิเสธ** และ codex รายงานเองว่า
"ระบบปฏิเสธสิทธิ์ (operation not permitted)" · `--read-only` codex ขออนุญาตก่อน
แล้วโดน DENY → `stop: cancelled` ไม่มีไฟล์ถูกสร้าง

**carve-out ในตัว workspace** (Workspace profile): `.chorus/` (shadow repo ที่ `revert_if_touched` เชื่อ — agent แก้ได้ = ซ่อนร่องรอยตัวเองได้) · `.git/hooks` · `.git/config` (ไฟล์ที่เขียนตรงนี้ `git` ของ user รันทีหลังนอก sandbox) — Seatbelt เลือกกฎ*ตัวสุดท้าย* จึงต้อง `deny` ต่อท้าย `allow` · ปิดได้ด้วย `CHORUS_SANDBOX=off` (ยังพิมพ์เตือน) · เทสต์ `enforced::` รัน `sandbox-exec` จริง ไม่ใช่ string match · รัน codex จริง 5 ต.ค. 2026: เขียนใน workspace ได้ · `.git/hooks/pre-commit` กับ `~/…` โดน "operation not permitted"

ถ้าไม่มี `sandbox-exec` (ไม่ใช่ macOS) → `wrap` คืนคำสั่งเดิมพร้อมเหตุผล **และต้องพิมพ์บอก**
sandbox ที่ทำให้คนทำงานไม่ได้คือ sandbox ที่โดนปิด — แต่ sandbox ที่ปิดอยู่เงียบ ๆ แย่กว่า

### 7.21 agent ใช้ git/GitHub ของ user ไม่ได้ (วัดจริง 16 ต.ค. 2026)

สองสาเหตุแยกกัน ทั้งคู่เงียบ:

- **แอปเปิดจาก Finder/Dock → launchd ให้ `PATH=/usr/bin:/bin:/usr/sbin:/sbin`** agent หา `gh`/`node`/`npx`
  ไม่เจอ (Apple git เจอ เลยดูเหมือน "git ใช้ได้แต่ account ไม่ขึ้น") แก้ที่ `chorus-acp/src/env.rs`:
  ถาม login shell (`$SHELL -ilc`, cache, timeout 6 วิ) แล้ว **ต่อท้าย** PATH เดิม ไม่ import env อื่น (กัน token หลุด)
- **Seatbelt ห้ามเขียน `~/.ssh`** → `ssh` จด known_hosts ไม่ได้ push ผ่าน SSH ล้ม
  อนุญาตเฉพาะ `known_hosts*` กับ `control/` — `config` ต้องอ่านอย่างเดียว (`ProxyCommand` รันนอก sandbox ทีหลัง)

ยังเป็นข้อจำกัดโดยตั้งใจ: `git push -u` / `git branch --set-upstream-to` เขียน `.git/config` ไม่ได้ (carve-out ใน 7.20)

### 7.22 เทสต์ 6 ตัวผ่านได้เฉพาะบน macOS (วัด 19 ต.ค. 2026)

turn ที่ policy เป็น read-only (`DenyAll`, `ReadOnly`, orchestrator, reader child ที่ไม่มี worktree)
**ต้องปฏิเสธ**ถ้าครอบด้วย Seatbelt ไม่ได้ (`Cannot enforce read-only sandbox`) — ตั้งใจไว้ ห้ามแก้ให้รันต่อ
บน Linux จึงล้ม: `orchestra_core` ×4, `orchestra_failover::lead_at_its_limit…`, `session::the_ledger_survives…`
ไม่ใช่ regression · ฝั่ง Linux มี `restrictive_policies_fail_closed_without_seatbelt` ยืนยันว่าปฏิเสธจริง
จะ gate ด้วย `cfg_attr(..., ignore)` หรือไม่ ต้องให้เจ้าของโปรเจกต์ตัดสิน · CI ตอนนี้มีแค่ release บน `macos-14`

### 7.23 นำเข้า session จากแอปอื่น — ไฟล์ของเขา format ของเขา (19 ต.ค. 2026)

`chorus sessions` / แท็บ "จากแอปอื่น" ใน ⇧⌘F อ่าน transcript ที่ CLI แต่ละเจ้าเขียนไว้แล้ว
**copy เข้า ledger** — หลังจากนั้น ledger เป็นเจ้าของ (2.2) ไม่ resume ฝั่ง provider และไม่เขียนกลับ

| เจ้า | ที่อยู่ | รูปแบบ (ยืนยันจาก) |
|---|---|---|
| Claude Code | `<CLAUDE_CONFIG_DIR or ~/.claude>/projects/<cwd แปลง>/<id>.jsonl` | `type` user/assistant + `message.content` (docs บอกเองว่า **format ภายใน เปลี่ยนได้ทุกเวอร์ชัน** → ทุกฟิลด์ optional) |
| Codex | `<CODEX_HOME>/sessions/YYYY/MM/DD/rollout-*.jsonl` **หรือ `.jsonl.zst`** | `{timestamp,type,payload}` บรรทัดแรก `session_meta` (source codex-rs/rollout) |
| Gemini CLI | `~/.gemini/tmp/<project_hash>/chats/*.jsonl` (+ `.json` เก่า) | metadata แล้ว message `{id,type,content,toolCalls}` + `$rewindTo`/`$set`/`$patch` ที่แก้ของเก่า (source chatRecordingService.ts) |

กับดักที่เจอแล้ว:
- **นับเทิร์นจากของดิบ ไม่ใช่ที่รวมแล้ว** — รวมข้อความติดกันฝั่งเดียวกันทำให้จำนวนไม่นิ่ง (user ส่งต่ออีกบรรทัด = รวมเข้าอันเดิม = "ไม่มีอะไรใหม่")
- Codex เขียนทุกข้อความซ้ำใน `event_msg` — อ่านแค่ `response_item` ไม่งั้นได้ทุกอย่างสองรอบ
- "user" ที่ CLI ยัดเอง (`<environment_context>`, AGENTS.md, `<command-name>`) ไม่ใช่สิ่งที่คนพิมพ์ → `is_wrapper_noise`
- Gemini ที่ rewind ไปแล้ว ถ้าไม่ replay `$rewindTo` จะได้คำตอบที่ user ทิ้งไปแล้วกลับมา

การนำเข้าซ้ำไม่สร้าง session ใหม่: marker = Handoff event (`source`, `turns`) ท้าย import — **ไม่เพิ่มตาราง**
ถ้าต้นทางมีข้อความใหม่และฝั่ง Chorus ยังไม่ได้คุยต่อ → ต่อท้าย · ถ้าคุยต่อไปแล้ว → แค่รายงาน (`not_appended`) ไม่สอดแทรก
marker ต้องอยู่ inline (< 2 KB) เพราะหาด้วย `LIKE` · ใน context เหลือบรรทัดเดียว `(conversation above imported from Codex)` (ดู 7.25)

### 7.24 harness ที่ป้อนผลให้ตัวเอง — เจอซ้ำใน `chorus-eval` (19 ต.ค. 2026)

Tier 1 ส่วน metrics ตั้ง `passed: true` ทุกงาน · token = raw × 0.45 · duration 12 ms แล้วพิมพ์เป็น
"Pass@1 100%" — ไม่มี agent รันเลย ส่วนที่เป็น gate จริง (classification/verification/assembly) วัดของจริง
ตอนนี้ report มี `metrics_simulated: true` และพิมพ์เตือนก่อนตัวเลข · `mlx_runtime` ก็เหมือนกัน
(`generate_tokens` คืนข้อความตายตัว → `MlxInferenceStats::simulated`) **ห้ามอ้างตัวเลขพวกนี้เป็นผลวัด**

### 7.25 handoff เก็บเป็น brief เต็ม แต่ส่งเข้าโมเดลบรรทัดเดียว (วัด 19 ต.ค. 2026)

`assemble_context` เคยส่ง JSON ของ brief (รวม `diff`) เข้า conversation tier ทุกเทิร์นหลัง handoff —
**8,250 token ต่อเทิร์น** สำหรับ diff 300 บรรทัด ทั้งที่ทุกอย่างในนั้นถึงโมเดลทางอื่นอยู่แล้ว
(goal/pending/constraints → task state · ไฟล์ → shadow repo) และ diff เป็นภาพเก่าตั้งแต่ edit ถัดไป
ตอนนี้ `handoff::context_line()` ให้ `handoff: claude → codex (quota exhausted)` ทุก rung = **10 token**
JSON บนดิสก์ไม่แตะ (`task_state` ต้องอ่าน) · brief เต็มยังส่งครั้งเดียวตอน Verify (`render_markdown`)
bench `handoff in later turns' context` กันไว้: เพดาน 30 token + ห้ามมี `` ```diff `` / `"constraints"` ใน context
+ pending/constraints ต้องยังอยู่ (นับแยกเป็นของ task state)

### 7.26 repo map — จัดอันดับไฟล์ ไม่ใช่ symbol (วัด 19 ต.ค. 2026)

`Session::assemble_context` ใส่ `repo-map` เป็น item แรกของ code tier (≤ 1/3 ของชั้น, เพดาน 1,024 token)
cache ใน `RepoMapCache` (หน่วยความจำ ไม่ลงดิสก์ — 2.1) สร้างใหม่เฉพาะเมื่อ size/mtime ของไฟล์ต้นฉบับเปลี่ยน
รายชื่อไฟล์มาจาก shadow repo (`Checkpoint::files`, อ่านอย่างเดียว ยึด .gitignore) · seed = `files_since(base)`

กับดักที่เจอระหว่างทำ (วัดบนสำเนา repo นี้ 176 ไฟล์):
- graph ระดับ symbol เดิม (`build_graph`) ผูกทุก symbol ในไฟล์กับทุกชื่อที่ไฟล์นั้นพูดถึง → ContentView.swift ไฟล์เดียวเป็นล้าน edge
  **~340 ms ต่อเทิร์น** แม้ cache แล้ว → เปลี่ยนเป็น PageRank ระดับไฟล์ (แบบ Aider) แล้วแจกคะแนนให้ symbol ที่ถูกอ้าง: ~20–30 ms
- สแกน identifier แยกไม่ออกว่าเป็นการเรียกจริงหรือคำในคอมเมนต์ → map เต็มไปด้วย `new`/`default`/`match`
  แก้ด้วย `name_weight`: ชื่อที่นิยาม > 3 ที่ = 0 · snake/camel ≥ 8 ตัว ×10 · คำล้วนตัวเล็ก ×0.1 · คูณ IDF
- render นับด้วย `TokenCounter` ตัวเดียวกับชั้นอื่น ไม่ใช่ `len/4` (7.1)

### 7.27 computer use — สิ่งที่ต้องรู้ก่อนแตะ (20 ต.ค. 2026 · ยังไม่เคยรันบน Mac จริง)

- action ตามคำศัพท์ของ computer-use tool ของ Anthropic (`left_click`, `key` แบบ `cmd+s`, `coordinate` เป็น pixel ของ screenshot)
  Claude เทรนกับชุดนี้ · screenshot ย่อให้ด้านยาว ≤ 1280 px แล้ว map กลับเป็น point ในโค้ด — Retina 2× ไม่ถึงโมเดล
- TCC: Screen Recording (ภาพ) + Accessibility (เมาส์/คีย์บอร์ด) **ผูกกับ code signature** — build ad-hoc ใหม่แต่ละครั้ง
  macOS อาจถือว่าเป็นแอปใหม่แล้วขอสิทธิ์ซ้ำ (แก้จริงด้วย notarize/Developer ID) · CLI ใช้สิทธิ์ของ Terminal ที่รัน
- `Package.swift` ต้อง link `CoreGraphics` + `ApplicationServices` (staticlib ไม่พา `#[link]` ไปให้ Swift linker)
- ส่วน `mac` ใน computer.rs คอมไพล์เฉพาะ macOS — clippy/test ของมันอยู่ที่ CI · ตรรกะ (gate, parse, scaling) เทสต์บน Linux ด้วย `Desktop` ปลอม
- โหมดถามก่อน: FFI ลงทะเบียน `computer_asker` ก่อน `run_turn_decide*` แล้วถอดหลังจบ — **เทิร์นที่ไม่มี asker จะปฏิเสธ action ที่เปลี่ยนสถานะ** ไม่ใช่ปล่อยผ่าน

### 7.28 secrets — กับดักที่ต้องรู้ (20 ต.ค. 2026 · Keychain/Seatbelt ยังไม่เคยรันบน Mac จริง)

- **`assemble_context` ไม่ต้อง mask** เพราะทุกทางที่ข้อความเข้า ledger mask ไปแล้ว (`mask_secrets` ใน session.rs, `secrets::user_vault().mask` ใน mcp.rs/external_sessions.rs) — ทางเข้าใหม่ต้องเพิ่มเอง
- ค่าที่ไม่มี pattern (URL ที่มีรหัสผ่านใน `.env`) จับได้เพราะ vault จำค่าที่เคยเห็นในรอบนี้ (`known`) แล้วแทนแบบตรงตัว — agent `echo $X` แล้วพูดค่ากลับมา ledger ยังได้ชื่อ แต่ **provider ของ agent ตัวนั้นเห็นค่าไปแล้ว** ป้องกันไม่ได้ ได้แค่ไม่ให้ไปต่อ
- `.env` ที่ฉีดเป็น env ห้ามทับตัวแปรที่ bridge ใช้เอง (`ANTHROPIC_*` `OPENAI_*` `CLAUDE*` `CODEX_*` `GEMINI_*` `CHORUS_*` `PATH` `DYLD_*` …) — `ANTHROPIC_API_KEY` ของโปรเจกต์จะเปลี่ยน Claude จาก login เป็นจ่ายตาม API เงียบ ๆ ตัวพวกนี้ยังได้ทางชื่อ `$CHORUS_SECRET_…`
- deny แค่ `file-read-data` ไม่ใช่ `file-read*` — ไฟล์ยังโผล่ใน `ls`/`git status` และเขียนได้ · template (`.env.example` ฯลฯ) allow กลับหลัง deny (last-match-wins) · **ไม่**ปิด `~/.netrc` `~/.aws/credentials` `~/.ssh` — git/aws/npm ของ agent ต้องใช้ (7.21)
- framework ที่อ่าน `.env` จากไฟล์เองโดยไม่ดู env (บางเวอร์ชันของ Next.js) จะเห็นไฟล์อ่านไม่ได้ → ปิดใน Settings › Secret
- เทสต์ตั้ง `CHORUS_SECRETS_BACKEND=memory` — `user_vault()` เช็คทุกครั้ง ไม่งั้น `cargo test` บน Mac จะเขียนค่าปลอมลง Keychain จริงของคนรัน · Keychain ล็อก (CI) → เก็บในหน่วยความจำรอบนี้แทน (`fallback`) ยัง mask และ inject ได้
- เทสต์สร้าง key ปลอมตอนรัน (`fake()`) ห้ามเขียนเป็น literal — GitHub push protection จะบล็อก push

### 7.3 repo เงาสำหรับ checkpoint
ต้องยึด `.gitignore` ของผู้ใช้ ไม่งั้น `.chorus/` จะโตกว่าโปรเจกต์จริง

### 7.4 ledger บวม
tool output ดิบทำให้ log โตเป็น GB ได้ (Codex เคยเจอ — issue #24948)
เก็บ output ใหญ่แยก blob แล้วอ้างด้วย hash ตั้งแต่วันแรก

### 7.5 ACP — ข้อเท็จจริงที่ยืนยันจากสเปกแล้ว
- transport = **newline-delimited JSON over stdio** ไม่ใช่ Content-Length header แบบ LSP
- `protocolVersion` เป็น **integer `1`** ไม่ใช่ string
- `session/new` ต้องมี `cwd` + `mcpServers`; `session/load` ต้องมี `sessionId` + `cwd` + `mcpServers`
- `session/update` เป็น tagged union ผ่านฟิลด์ `sessionUpdate`
- **client เป็นคนถือไฟล์** — agent ต้องขอ `fs/read_text_file` ผ่านเรา
  แปลว่า permission engine ของเราแทรกตรงนี้ได้ ไม่ต้อง sandbox ซ้อน sandbox

### 7.6 Codex ไม่มี ACP ในตัว — ต้องผ่าน bridge (ยืนยันจากเครื่องจริง 11 ก.ย. 2026)
`codex-cli 0.153.4` **ไม่มี** subcommand `acp` (`codex acp` = ตกลง TUI แล้วตายเพราะ `stdin is not a terminal`)
ที่มีคือ `codex mcp-server` (stdio) กับ `codex app-server` (experimental, JSON-RPC ของตัวเอง) — ทั้งคู่ไม่ใช่ ACP
**ทางที่ใช้**: bridge `@agentclientprotocol/codex-acp` (npm, v1.11.0, bin `codex-acp`)
เดิมชื่อ `@zed-industries/codex-acp` — deprecated แล้ว อย่าใช้ตัวเก่า
คำสั่ง probe จริง: `cargo run --bin acp-probe -- --cwd /tmp/acp-scratch -- npx -y @agentclientprotocol/codex-acp`

---

## 8. ถัดไป

### สไปก์ที่ 1 — ACP กับ Codex จริง ✅ (probe สำเร็จ 11 ก.ย. 2026 ผ่าน bridge)
รันผ่าน `@agentclientprotocol/codex-acp` v1.11.0 (ดู 7.6) auth = ChatGPT login
transcript อยู่ที่ราก repo (`acp-probe-*.jsonl`, gitignored) — 1789059819734 = probe เต็ม

**คำตอบสามข้อ:**
- **Q1 session/load** — รองรับ (`loadSession:true`) แต่ **replay ประวัติกลับมา ~12–14 เฟรม** ไม่ใช่ no-replay
  ไม่กระทบ "ledger = SoT" — เราทิ้ง replay ของ codex ทิ้ง ใช้ ledger เราต่อ แค่ต้องรู้ว่ามันจะส่งมา
- **Q2 permission** — ✅ **คุมได้ที่ ACP layer** ถ้า pin โหมดถูก (ครั้งแรกสรุปผิดว่า ❌ เพราะทดสอบผิด —
  ยิงแต่ write ที่ sandbox อนุญาตอยู่แล้ว)
  โหมดของ bridge (จาก `AgentMode` ใน bundle):
  | mode | approvalsReviewer | sandbox |
  |---|---|---|
  | `agent` (default) | `auto_review` — โมเดล codex อนุมัติเอง เราไม่เห็น request | workspace-write |
  | `read-only` | **`user` — request วิ่งมาที่ `session/request_permission` ของเรา** | workspace-write |
  | `agent-full-access` | — | danger-full-access |
  `session/set_mode "read-only"` ทำงานจริง (`--mode read-only` ใน probe)
  ใต้ sandbox `workspace-write`: write ใน workspace + `/tmp` ไม่ต้องขอ; **write นอกนั้น / network / คำสั่ง →
  `on-request` → `user` → probe เราได้ `session/request_permission`** (ทดสอบ: เขียน `~/acp-outside-test.txt`
  ในโหมด read-only → probe ปฏิเสธ → `stopReason: cancelled` ไฟล์ไม่ถูกสร้าง)
  bridge map ตัวเลือกของ codex (`accept_execpolicy_amendment` ฯลฯ) เป็น ACP `kind` (allow_once/allow_always/reject_once) ให้แล้ว — engine เราคีย์ที่ `kind` ได้
  ⚠️ **`approvalPolicy` ทั้งสองโหมดคือ `on-request` = โมเดล codex เลือกเองว่าจะถามเมื่อไหร่** ไม่ใช่ทุก action ที่ถูกบล็อกจะกลายเป็น prompt
  (เทสต์: `curl` ในโหมด read-only → codex ไม่ถาม รันใน sandbox ที่ `networkAccess:false` แล้ว report fail กลับโมเดลเอง)
  → **กำแพงจริงคือ sandbox** (`networkAccess:false`, write เฉพาะ workspace) `session/request_permission` เป็นแค่ทาง escalation ตอน codex เลือกถาม
  **adapter Phase 2**: `session/new` แล้ว `session/set_mode read-only` ทันที; ถ้าอยากให้ทุก write ใน workspace ผ่าน engine ด้วย
  ต้อง `sandboxMode: read-only` (ไม่ใช่ workspace-write) — bridge ไม่มี preset ต้องลองผ่าน `CODEX_CONFIG`
- **Q3 โครงสร้าง** — ✅ ดีกว่าที่กลัว ได้ `tool_call` / `tool_call_update` แยกชัด + `agent_thought_chunk`
  แยกจาก `agent_message_chunk` + `usage_update` (token count มาด้วย) → **ไม่ต้องเขียน text parser เปราะ ๆ**

**สรุปสไปก์ 1**: Q1 ✅ Q2 ✅ Q3 ✅ — ผ่านครบ · probe ได้ flag `--mode` (ส่ง `session/set_mode` หลัง `session/new`) ทำงานจริง

### สไปก์ที่ 2 — handoff eval (ทำไปแล้ว 7 เคส · ผ่าน 7/7)
`python3 evals/handoff/score.py` → 7/7 (100%) เป้าคือ ≥70%

**แต่ยังอย่าเพิ่งเชื่อตัวเลขนี้เต็มร้อย** ทั้ง 7 เคสเป็นงานที่เลือกมาจาก repo ตัวเอง
(in-domain ทั้งหมด) และคนเขียน brief คือคนให้คะแนนเอง สเปกเดิมบอกให้ใช้ *งานจริงของ Perth*
ซึ่งยังไม่มีสักเคส — เคสที่ 8 เป็นต้นไปควรเป็นงานนอก repo นี้ และถ้าเป็นไปได้ให้คนอื่นให้คะแนน
failure note ของแต่ละเคสมีค่ากว่าตัวเลขรวม (เคส 06 จับได้เองว่า agent พลาด `refresh()`
เพราะชื่อฟังก์ชันไม่มีคำว่า checkpoint ให้ grep เจอ)

### Phase 1 (หลังสองสไปก์)
- SwiftUI shell ตัวจริง — แชท, diff review, context inspector
- compaction ladder L1/L2 ด้วย Ollama ตอนเครื่อง idle (ผูกกับ thermal state + แบตเตอรี่)
- embedding ด้วย MLX บน Apple Silicon (หรือ `NLEmbedding` ถ้าเอาเบา)
- memory store + projector ไป `CLAUDE.md` / `AGENTS.md` / `GEMINI.md`

---

## 9. Handoff brief format (ใช้ทั้งในแอปและในสไปก์ที่ 2)

```json
{
  "goal": "แก้ N+1 query ในหน้า order list ให้เหลือ < 200ms",
  "done": ["พบว่า OrderRepo.findAll โหลด relation ทีละแถว", "เพิ่ม index แล้ว (migration 0042)"],
  "pending": ["เขียน eager-load ใน findAll", "รัน test ชุด order"],
  "constraints": ["ห้ามแตะ schema เพิ่ม", "ต้องผ่าน pnpm test:order"],
  "files": ["src/order/repo.ts:88", "migrations/0042_*.sql"],
  "diff": "<unified diff ตั้งแต่ checkpoint>",
  "handoff": { "from": "claude-opus", "to": "codex", "reason": "rate_limit", "at": "..." }
}
```

Handoff pipeline 5 ขั้น: **Freeze → Repack → Brief → Verify → Resume**
(Verify = ให้ agent ใหม่ echo-back ความเข้าใจก่อนลงมือ ผู้ใช้แก้ brief ได้ก่อนปล่อย)

---

## 10. Context budget (หน้าต่าง 200k เป็นตัวอย่าง)

| ชั้น | เพดาน | เต็มแล้วทำยังไง |
|---|---|---|
| System + skill | คงที่ | ปิด skill ที่ไม่เกี่ยว |
| Task state | 5% | **ไม่มีวันตัด** |
| Memory | 9% | ลด top-k |
| Conversation | 38% | ไล่ลง L1 → L2 (ย่อก่อนทิ้ง) |
| Code/diff | 26% | เปลี่ยนเป็น manifest ให้ agent เรียกอ่านเอง |
| Reserve | 16% | กันไว้ให้โมเดลตอบ |

Rung ladder: 10 เทิร์นล่าสุด = L0 · เทิร์น 11–40 = L1 · เก่ากว่านั้น = L2 · ข้าม session = L3

---

## 11. วิธีทำงานที่เจ้าของโปรเจกต์อยากได้

- **อธิบายเหตุผลก่อนเปลี่ยนอะไรที่ไม่ trivial** — architecture, dependency, schema, refactor
- **ถามก่อนทำ high-impact**: เปลี่ยน schema/migration, เพิ่ม-ถอด dependency, แก้ contract ใน `types.rs`,
  ลบไฟล์หรือ rename เป็นวงกว้าง, เปลี่ยน CI/deployment
- ทำ **เล็ก ตรงจุด typed** ดีกว่ารื้อใหญ่ ตามแบบแผนที่มีอยู่แล้วก่อนสร้างแบบแผนใหม่
- **รัน validation จริงก่อนบอกว่าเสร็จ** แล้วรายงานว่ารันอะไร ผ่านไหม อะไรที่ยังไม่ได้ตรวจ
- ห้าม commit/push/สร้าง PR เว้นแต่ถูกสั่ง
- ห้ามอ่านหรือพิมพ์ `.env` / credential / token
- ถ้ามี Serena MCP ให้ใช้สำรวจ symbol ก่อนแก้โค้ดหลายไฟล์

---

## 12. เอกสารฉบับเต็ม

อยู่ในรีโปที่ `docs/ref/` (มิเรอร์จาก Claude project **"AI Agent All In One"**):

- `docs/ref/Chorus_design.md` — blueprint: pain point, หลักการ, สถาปัตยกรรม, adapter, UI/UX, roadmap
- `docs/ref/Chorus_storage_spec.md` — สเปกการเก็บข้อมูลฉบับเต็ม
- `docs/ref/Chorus_phase0_status.md` — สถานะล่าสุด
- (มี `.html` + `_files/` ที่เซฟหน้า artifact มาด้วย — อ่าน `.md` พอ ตัว `_files/` ควร gitignore)

Artifact (มีไดอะแกรมกับ wireframe):
- Design blueprint — https://claude.ai/code/artifact/46e28c3e-8410-4446-ae5d-b7c0b865cc98
- Storage spec — https://claude.ai/code/artifact/d9b8ceb2-e6a9-4f63-a929-608f4c66725f

อ้างอิงภายนอกที่ใช้ตัดสินใจ:
- Claude Agent SDK sessions — https://code.claude.com/docs/en/agent-sdk/sessions
- Agent Client Protocol — https://agentclientprotocol.com/protocol/overview
- TOON benchmarks (ข้อมูลรูปตารางประหยัด token 30–60%) — https://toonformat.dev/guide/benchmarks
- TOON vs JSON, ข้อควรระวังเรื่อง "ค่าสอนไวยากรณ์" — https://arxiv.org/abs/2603.03306

---

## Agent skills

### Issue tracker

Issues/specs live as local markdown under `.scratch/<feature>/`. See `docs/agents/issue-tracker.md`.

### Triage labels

Default canonical vocabulary (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `CONTEXT.md` + `docs/adr/` at repo root. See `docs/agents/domain.md`.
