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

## 3. สถานะตอนนี้ (11 ก.ย. 2026)

**Phase 0–2 ครบ + macOS shell ครบ design §7** — เทสต์ผ่าน 60/60 · clippy clean

| crate | ทำอะไร |
|---|---|
| `chorus-core` | ledger (+ fork-chain `history()`), blob store, memory + approve queue (v2), token counter, render, context assembler, handoff brief, projector, checkpoint (shadow git + `apply_reverse`), compaction (Extractive + **Ollama**), **mcp (Streamable-HTTP: `memory_propose` — ทำงานกับ Codex)** |
| `chorus-acp` | ACP client + `acp-probe` + mock (`--fail`) + `CodexAgent` + `bridge_for` (codex/claude/gemini) · strip `CLAUDECODE` |
| `chorus-cli` | lib+bin · `bench`/`run`(+`--chain`/`--read-only`)/`handoff`/`memory`/`compare` · `Session` engine |
| `chorus-ffi` | C ABI (`chorus.h`) — engine/run_turn(+interactive callback)/run_relay/compare/handoff/events_since/working_diff/snapshot/restore/reject_hunks/memory_state+review · JSON ล้วน · panic-guarded |
| `apps/macos` | SwiftUI shell — sidebar (single/relay/compare/**ask-each**) · thread + agent blame · Context/Changes(**per-hunk keep-reject**)/Memory panes · handoff + compare sheets · **interactive permission dialog** · `apps/macos/build.sh` |

**Flow ที่ทำงานจริงกับ Codex**: หลาย agent ใน session เดียว · ledger เป็น SoT · จำข้ามเทิร์นจาก ledger
(ไม่ใช่ provider session) · handoff กลางคัน + Verify echo-back · brief พก diff จาก checkpoint ·
relay auto-failover 429/quota · compare (fork ต่อ agent) · projector เขียน agent files · memory `memory_propose`→approve queue · per-hunk diff reject · interactive permission

**ยังต้องเพื่อน (แต่ไม่บล็อกอะไร)**:
- **Gemini adapter** — ไม่มี `gemini` CLI (`bridge_for` เตรียมไว้ · ลง CLI แล้วใช้ได้ทันที)
- **claude-code-acp** — โค้ดพร้อม · เครื่องนี้ `claude` OAuth หมดอายุ → login ใหม่
- **Ollama compaction** — โค้ดพร้อม · `brew install ollama && ollama pull qwen2.5:3b` แล้วตั้ง `CHORUS_SUMMARY_MODEL`
- **embedding + sqlite-vec** — ต้องตัดสิน dep (MLX / `NLEmbedding` / sqlite-vec)
- **Seatbelt sandbox · notarize · .app bundle** — ต้อง Apple Developer account
- **event push** — shell ยัง poll `events_since` (ทำงานได้ดี · push เป็น optimization)

### ตัวเลขจาก `chorus bench` (78 events) — baseline ที่ห้ามถอยหลัง

```
อ่านไฟล์เดิม 8 ครั้ง → blob rows = 1 (naive = 8)   ประหยัด 99.6%
ledger ดิบ 103,553 token
  @200k window → 16,226 token (15.7%)   demoted/dropped 0/0
  @6k  window →  2,490 token (2.4%)     demoted/dropped 40/11
store.db 92 KB / 78 events   (เดิม 84 KB — schema v2 เพิ่มตาราง fact_proposal + index ว่าง ~8 KB)
```

**Invariant 3 ข้อ** — ถ้า bench แสดงผิดจากนี้แปลว่ามี regression:
1. `blob rows = 1` (dedup ทำงาน)
2. assembled เป็นเศษเสี้ยวของ raw
3. **demoted ต้องมาก่อน dropped เสมอ** — ย่อก่อนทิ้ง ไม่ใช่ตัดท้าย

---

## 4. คำสั่ง

```bash
cargo test                              # 60 tests
cargo run -p chorus-cli -- bench        # วัด invariant ทั้ง 3
cargo run --bin acp-probe -- --cwd /tmp/acp-scratch --mock         # ลอง ACP กับ agent ปลอม
cargo run -p chorus-acp --example codex_smoke -- /tmp/codex-smoke   # CodexAgent กับ bridge จริง (ต้อง codex login)

# session engine จริง (ต้อง codex login + npx)
cargo run -p chorus-cli -- run <cwd> "<prompt>" --db /tmp/s.db [--yes]
cargo run -p chorus-cli -- run "<prompt>" --session 1 --db /tmp/s.db          # เทิร์นต่อ
cargo run -p chorus-cli -- run <cwd> "<prompt>" --chain codex,claude-opus --db /tmp/s.db   # relay
cargo run -p chorus-cli -- handoff --db /tmp/s.db --session 1 --to codex --reason rate_limit --goal "..."
cargo run -p chorus-cli -- compare "<prompt>" --db /tmp/s.db --session 1 --agents codex,claude-opus
cargo run -p chorus-cli -- memory pending --db /tmp/s.db          # approve queue

# macOS shell
apps/macos/build.sh                     # cargo build -p chorus-ffi + swift build + run
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
  compaction.rs Summarizer trait · Extractive (model-free) + Ollama (localhost:11434) · best() เลือกให้
  mcp.rs        Streamable-HTTP MCP server (std::net, ไม่มี dep) — tool memory_propose · ทำงานกับ Codex แล้ว (persistent conn + held-open SSE)
crates/chorus-cli/src/main.rs           bench + run + handoff + memory + compare subcommands
crates/chorus-cli/src/session.rs        Session engine — run_turn/run_relay/handoff/compare/assemble_context/events_since
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

### สไปก์ที่ 2 — handoff eval (ยังไม่เริ่ม)
**นี่คือคำถามที่ตัดสินว่าแอปนี้มีค่าหรือเปล่า** ทำมือได้เลย ไม่ต้องมีแอป
10 เคสจริง → เขียน brief → วางให้อีกโมเดล → ให้คะแนน → `python3 evals/handoff/score.py`
เป้า ≥70% ถ้าต่ำกว่านั้นปัญหาอยู่ที่ format ของ brief ไม่ใช่ที่ engineering
**และต้องแก้ตรงนั้นก่อนลงแรงเขียนแอปต่อ**

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
