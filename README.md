# Chorus

> **Many agents. One score.**

Desktop app ที่ให้ AI agent หลายเจ้า (Claude, Codex, Gemini, local) ทำงานใน **session เดียวกัน**
ใช้ความจำก้อนเดียวกัน เห็น skill ชุดเดียวกัน และ **ส่งไม้ต่อกันได้กลางประโยค** เมื่อตัวใดตัวหนึ่งชนลิมิต

สถานะ: **Phase 0** — Rust core + measurement harness. macOS first.

---

## กฎที่ทุกอย่างในนี้ตามมา

> ที่เก็บของ กับ สิ่งที่ส่งให้โมเดล เป็นคนละ format กัน

- **Cold (ดิสก์)** ปรับให้ไบต์น้อยและ query เร็ว ไม่ต้องให้ AI อ่านรู้เรื่อง → `schema.rs` `store.rs` `blob.rs` `ledger.rs` `memory.rs`
- **Hot (เข้าโมเดล)** ปรับให้ token น้อย ประกอบใหม่ทุกครั้ง ไม่เคยเขียนกลับลงดิสก์ → `tokens.rs` `render.rs` `context.rs`

ถ้าเผลอเอา format ฝั่งหนึ่งไปใช้อีกฝั่ง จะได้ทั้งดิสก์บวมและ token เปลืองพร้อมกัน

## เริ่มยังไง

```bash
cargo test          # 22 tests
cargo run -p chorus-cli -- bench
```

`bench` คือตัววัดว่าโค้ดยังทำตามสเปกอยู่จริง รันได้ทุกครั้งที่แก้ core:

```
── dedup ─────────────────────────────────────────────
  events                78
  reads of one file     8
  blob rows             1   (naive would be 8)
  stored                1.4 KB compressed / 49.0 KB raw
  naive would store     392.2 KB   → saved 99.6%

── context @ 6000 token window ─────────────────────
  ledger raw            103553 token
  assembled             2490 token   (2.4% ของดิบ)
  demoted / dropped     40 / 10
```

ตัวเลขสามอย่างที่ต้องไม่ถอยหลัง: **blob rows ต้องเป็น 1**, **assembled ต้องเป็นเศษเสี้ยวของ raw**,
และ **demoted ต้องมาก่อน dropped เสมอ** (ย่อก่อนทิ้ง)

## โครงสร้าง

```
crates/chorus-core/
  types.rs      Event/EventKind/BlobId — สัญญากลาง อย่าแก้เล่น
  schema.rs     migration + FTS5 (มี fallback ถ้าไม่มี fts5)
  store.rs      เปิด db, ตั้ง WAL/pragma
  actor.rs      intern ชื่อ agent เป็น integer
  blob.rs       content-addressed store: BLAKE3 + zstd + dedup + GC
  ledger.rs     append/get/tail/fork/search — ledger เป็น DAG, fork ไม่ copy อะไรเลย
  memory.rs     fact store (บังคับ ≤140 ตัวอักษร) + render แบบไม่มี metadata
  tokens.rs     ตัวประมาณ token ที่รู้จักภาษาไทย + cache
  render.rs     table/manifest/truncate/dedup_window — รูปแบบที่ส่งเข้าโมเดล
  context.rs    budget allocator + rung ladder L0→L1→L2
crates/chorus-cli/
  main.rs       bench harness
crates/chorus-acp/
  lib.rs        ACP client เปล่า ๆ (ndjson JSON-RPC over stdio)
  bin/acp_probe.rs        สไปก์ที่ 1 — ยิงจริงแล้วจดทุกเฟรม
  bin/acp_mock_agent.rs   agent ปลอม ไว้เทสต์ probe โดยไม่ต้องมี API key
evals/handoff/  spike ที่ 2 — วัดคุณภาพการส่งไม้ต่อ
```

## สิ่งที่ยังไม่มี (ตั้งใจ)

- ไม่มี adapter จริงสักตัว — `chorus-acp` เป็นสไปก์สำหรับ *วัด* ไม่ใช่ adapter
- ไม่มี sqlite-vec / embedding — Phase 1
- ไม่มี UI — Phase 1 (SwiftUI, ห้ามมี logic ใน Swift)
- `HeuristicCounter` เป็นแค่ตัวประมาณสำหรับจัดงบ **ก่อนยิงจริงต้องใช้ tokenizer จริงของแต่ละเจ้า**

## ข้อควรระวังที่เขียนไว้กันลืม

- อย่านับ token ด้วย `len/4` — ภาษาไทยกิน token ต่อไบต์สูงกว่าอังกฤษ มีเทสต์กันไว้แล้วใน `tests/context.rs`
- อย่าให้ agent เขียน memory ตรง ๆ ต้องผ่าน approve queue (ยังไม่ได้ทำ — Phase 1)
- repo เงาสำหรับ checkpoint ต้องยึด `.gitignore` ของผู้ใช้ ไม่งั้น `.chorus/` จะโตกว่าโปรเจกต์จริง

เอกสารออกแบบเต็มอยู่ใน Claude project: `Chorus Design Blueprint` และ `Chorus Storage Spec`
