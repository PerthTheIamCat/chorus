# Chorus — Phase 0 status (10 ก.ย. 2026)

โค้ดอยู่ที่เครื่อง Perth: `~/private_project/chorus` (เดิมชื่อ Baton)

## ทำไปแล้ว — เทสต์ผ่าน 22/22

- `chorus-core` — ledger บน SQLite, blob store content-addressed (BLAKE3 + zstd + dedup + GC),
  actor interning, memory fact store (≤140 ตัวอักษร), token counter ที่รู้จักภาษาไทย,
  render (table/manifest/truncate/dedup_window), context budget allocator + rung ladder
- `chorus-cli` — `chorus bench` measurement harness
- `chorus-acp` — **สไปก์ที่ 1**: ACP client เปล่า ๆ (ndjson JSON-RPC over stdio) +
  `acp-probe` (ยิงจริง จดทุกเฟรมลง .jsonl) + `acp-mock-agent` (agent ปลอมไว้เทสต์)
- `evals/handoff/` — **สไปก์ที่ 2** scaffolding (README + เกณฑ์ + `score.py`)

รันบน MacBook Air แล้ว: `cargo test` เขียวหมด, build 11 วิ

## ตัวเลขจาก bench (78 events)

| วัด                  | ผล                                          |
| -------------------- | ------------------------------------------- |
| อ่านไฟล์เดิม 8 ครั้ง | blob rows = **1** → ประหยัด 99.6%           |
| ledger ดิบ           | 103,553 token                               |
| assembled @ 200k     | 16,226 token (15.7%)                        |
| assembled @ 6k       | 2,490 token (2.4%), demoted 40 / dropped 11 |
| store.db             | 84 KB / 78 events                           |

**Invariant ห้ามถอยหลัง**: blob rows = 1, assembled เป็นเศษเสี้ยวของ raw, demoted มาก่อน dropped

## ข้อเท็จจริงเรื่อง ACP ที่ยืนยันแล้ว

- Transport = **newline-delimited JSON over stdio** (ไม่ใช่ Content-Length header แบบ LSP)
- `protocolVersion` เป็น **integer `1`** ไม่ใช่ string
- `session/new` ต้องมี `cwd` + `mcpServers`; `session/load` ต้องมี `sessionId` + `cwd` + `mcpServers`
- `session/update` เป็น tagged union ผ่านฟิลด์ `sessionUpdate`
- client เป็นคนถือ `fs/read_text_file` / `fs/write_text_file` / terminal — agent ต้องขอผ่านเรา

## ข้อตัดสินใจระหว่างทาง

- คอลัมน์ `blob.zstd` — เก็บ raw เมื่อบีบแล้วไม่เล็กลง
- FTS5 create แบบไม่ fatal + `ledger::search` fallback เป็น LIKE
- **token ภาษาไทย = 1 token/ตัวอักษร** (ไม่ใช่ 1/2) มีเทสต์กัน regress
- probe **ปฏิเสธ permission เป็นดีฟอลต์** และไม่เขียนไฟล์เลยแม้ใส่ `--allow`

## ถัดไป

1. รัน `acp-probe` กับ Codex CLI จริงบนเครื่อง — ต้องเช็คก่อนว่า `codex acp` มีในเวอร์ชันที่ลงไว้ไหม
   ถ้าไม่มีให้ใช้ bridge แยก (`codex-acp`) เอา transcript มาอ่านด้วยกัน
2. สไปก์ handoff eval — 10 เคสจริง ทำมือ เป้า ≥70%
3. หลังได้ผลสองอันนี้ → เริ่ม Phase 1 (SwiftUI shell + compaction ด้วยโมเดล local)
