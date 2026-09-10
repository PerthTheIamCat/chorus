# Chorus — Storage Spec v0.2 (macOS first)

สเปกการเก็บข้อมูลที่ประหยัด token — คู่กับ `claude/chorus-design.md`
Artifact: https://claude.ai/code/artifact/d9b8ceb2-e6a9-4f63-a929-608f4c66725f

---

## 1. กฎข้อเดียว: storage ≠ context

- **Cold (ดิสก์)** — ปรับให้ไบต์น้อย + query เร็ว ไม่ต้องให้ AI อ่านรู้เรื่อง
- **Warm (ตัวย่อ)** — สรุปหลายชั้น generate ล่วงหน้าด้วยโมเดล local
- **Hot (เข้าโมเดล)** — ปรับให้ token น้อย ประกอบใหม่ทุกครั้ง **ไม่เคยเก็บลงดิสก์** และไม่มี metadata ที่โมเดลไม่ได้ใช้

ห้ามส่ง JSON ของ ledger เข้า prompt — key ซ้ำทุกแถวคือการจ่าย token ซ้ำ
ข้อมูลรูปตารางแบบมีหัวตารางครั้งเดียวประหยัด 30–60% โดยความแม่นยำไม่ลด
แต่อย่าคิด format ใหม่เอง เพราะค่าสอนไวยากรณ์จะกลืนกำไรเมื่อ context สั้น

## 2. Cold storage — SQLite ไฟล์เดียวต่อ workspace (เปลี่ยนจาก JSONL)

```sql
CREATE TABLE event (
  seq    INTEGER PRIMARY KEY,   -- monotonic ไม่ใช้ uuid (8 ไบต์ vs 36)
  t      INTEGER NOT NULL,      -- ms offset จาก session.started_ms ไม่ใช่ ISO string
  sess   INTEGER NOT NULL,
  parent INTEGER,               -- ledger เป็น DAG → fork = 1 แถว
  kind   INTEGER NOT NULL,      -- enum int: msg/tool_call/tool_result/diff/decision/handoff/summary
  actor  INTEGER NOT NULL,      -- FK → actor(id) ไม่เก็บชื่อโมเดลซ้ำทุกแถว
  body   TEXT,                  -- inline เฉพาะ < 2 KB
  blob   TEXT,                  -- ไม่งั้นเก็บ BlobId ชี้ไป blob store
  tok_in INTEGER, tok_out INTEGER, cost_usd REAL
);
CREATE INDEX ev_sess ON event(sess, seq);
CREATE VIRTUAL TABLE ev_fts USING fts5(body, content='event', content_rowid='seq');
```

WAL mode. `blob` มีคอลัมน์ `zstd` บอกว่าเก็บแบบบีบหรือดิบ (บีบแล้วไม่เล็กลงก็เก็บดิบ)
FTS5 สร้างใน block ที่ไม่ fatal + `ledger::search` fallback เป็น LIKE
sqlite-vec ยังไม่ทำ — Phase 1

## 3. Dedup — กำไรก้อนใหญ่ที่สุด

agent อ่านไฟล์เดิมซ้ำ 5–10 ครั้ง/session → content-addressed store

- hash = **BLAKE3** (SIMD บน ARM)
- เก็บครั้งแรกเต็ม ครั้งถัดไปเป็น reference
- ตอนประกอบ context: hash ซ้ำในหน้าต่างเดียวกัน → ส่งของจริงครั้งเดียว ที่เหลือแทนด้วย "(อ่านซ้ำ เนื้อหาเหมือนเดิม)"; ถ้าไฟล์เปลี่ยน → ส่งแค่ unified diff
- บีบด้วย **zstd + trained dictionary** ต่อ workspace
- **แคช token count คู่กับ hash** — นับครั้งเดียวตลอดไป
- GC blob ที่ไม่มีคนชี้และเก่ากว่า 30 วัน ตอนแอป idle

## 4. Checkpoint — ยืม git

libgit2 + repo เงาที่ `.chorus/ckpt` (ตั้ง GIT_DIR แยก ไม่ยุ่งกับ git ผู้ใช้)
ได้ dedup ระดับ object, compression, diff, rollback, branch (ใช้กับ compare mode) ฟรี
event ชนิด diff เก็บแค่ commit sha 20 ไบต์
⚠️ ยึด .gitignore ของผู้ใช้ + ignore list ของเรา ไม่งั้น `.chorus/` โตกว่าโปรเจกต์

## 5. บันไดการย่อ 4 ขั้น (ทำด้วยโมเดล local = ฟรี)

| ชั้น       | คืออะไร                                    | ขนาด          |
| ---------- | ------------------------------------------ | ------------- |
| L0 raw     | เหตุการณ์ดิบ อยู่บนดิสก์เสมอ               | ~12 KB/เทิร์น |
| L1 turn    | สรุป 2–3 บรรทัดต่อเทิร์น                   | ~60 token     |
| L2 episode | รวบเมื่อจบ milestone — ใช้ใน handoff brief | ~250 token    |
| L3 fact    | ข้อเท็จจริงถาวร → memory store             | ~15 token/ข้อ |

**กฎเลือกชั้น**: 10 เทิร์นล่าสุด = L0 · เทิร์น 11–40 = L1 · เก่ากว่านั้น = L2 · ข้าม session = L3
ยิ่งไกลยิ่งย่อ ไม่ตัดทิ้งแบบ sliding window

## 6. Memory — เก็บมี metadata ส่งไม่มี metadata

```sql
CREATE TABLE fact (
  id INTEGER PRIMARY KEY, scope INTEGER,  -- 0 global / 1 project / 2 session
  text TEXT NOT NULL,                     -- ≤ 140 ตัวอักษร บังคับ (reject ถ้าเกิน)
  src INTEGER, conf REAL, pinned INTEGER,
  hits INTEGER DEFAULT 0, last_used_ms INTEGER, expires_ms INTEGER, created_ms INTEGER
);
```

ส่งเข้าโมเดลเป็น bullet เปล่า ๆ ไม่มี id/conf/timestamp → ประหยัด ~70% ต่อ fact

- dedup ด้วย embedding (cosine > 0.92 → merge)
- TTL + usage decay: ไม่ถูกใช้ 90 วัน + ไม่ pinned → เสนอลบ
- ดึงด้วย vector search จากโจทย์ปัจจุบัน top-k ตามงบ ไม่เรียงตามเวลา

## 7. Context budget allocator (ตัวอย่างหน้าต่าง 200k)

| ลำดับ | ชั้น                           | เพดาน    | token  | เต็มแล้ว                                    |
| ----- | ------------------------------ | -------- | ------ | ------------------------------------------- |
| 1     | System + skill                 | คงที่    | 12,000 | ปิด skill ที่ไม่เกี่ยว                      |
| 2     | Task state (goal/done/pending) | 5%       | 10,000 | ไม่มีวันตัด                                 |
| 3     | Memory retrieved               | 9%       | 18,000 | ลด top-k                                    |
| 4     | บทสนทนา                        | 38%      | 76,000 | ไล่ลง L1 → L2 (ย่อก่อนทิ้งเสมอ)             |
| 5     | โค้ด/diff                      | 26%      | 52,000 | เปลี่ยนเป็น manifest ให้ agent เรียกอ่านเอง |
| 6     | Tool output                    | รวมข้อ 5 | —      | หัว 40 + ท้าย 40 บรรทัด                     |
|       | กันไว้ให้ตอบ                   | 16%      | 32,000 |                                             |

⚠️ ใช้ tokenizer จริง (tiktoken / count-tokens) แล้วแคชกับ hash
**`len/4` พังหนักกับภาษาไทย** — ค่าประมาณที่ใช้อยู่: ไทย/CJK 1 token/ตัวอักษร, ละติน 4 ตัวอักษร/token

## 8. ส่งอะไรใช้รูปแบบไหน

| ข้อมูล              | รูปแบบ                            |
| ------------------- | --------------------------------- |
| คำสั่ง/brief        | markdown สั้น หัวข้อระดับเดียว    |
| รายการโครงเหมือนกัน | หัวตารางครั้งเดียว + แถวคั่น `\|` |
| โค้ด                | เฉพาะ hunk + `path:บรรทัด`        |
| การเปลี่ยนแปลง      | unified diff                      |
| โครงสร้างโปรเจกต์   | tree ตัดตามความลึก + .gitignore   |
| tool output ยาว     | หัว 40 + ท้าย 40 + จำนวนที่ตัด    |

ตัวอย่าง: JSON 96 token → รูปแบบตาราง 38 token

```
ไฟล์ที่แก้ไป (path | สถานะ | +เพิ่ม/-ลบ):
src/order/repo.ts | modified | +38/-11
migrations/0042_idx.sql | added | +6/-0
```

## 9. ของ Apple ที่ใช้ได้ฟรี

- **Embedding บนเครื่อง**: MLX รันโมเดล embedding เล็กบน Apple Silicon (หรือ `NLEmbedding` สำหรับ dedup ระดับประโยค)
- **ย่อสรุป**: Ollama 3–8B ตอน idle ผูกกับ thermal state + สถานะแบตเตอรี่
- **ที่เก็บ**: store.db ที่ `~/Library/Application Support/Chorus/`, blob + repo เงาที่ `Caches/` ตั้ง `isExcludedFromBackup`
- **UI**: `LazyVStack` + keyset pagination ด้วย `seq` อย่าโหลดทั้ง session เข้า memory

## อ้างอิง

- TOON benchmarks: https://toonformat.dev/guide/benchmarks
- TOON vs JSON (arXiv 2603.03306): https://arxiv.org/abs/2603.03306
- openai/codex #24948 (log บวมระดับ GB): https://github.com/openai/codex/issues/24948
