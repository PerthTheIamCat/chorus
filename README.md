# Chorus

> **Many agents. One score.**

Desktop app (macOS ก่อน) ที่ให้ AI agent หลายเจ้า — Claude, Codex, Antigravity (Gemini) และโมเดล local —
ทำงานใน **session เดียวกัน** ใช้ความจำก้อนเดียวกัน เห็น skill ชุดเดียวกัน และ
**ส่งไม้ต่อกันได้กลางประโยค** เมื่อตัวใดตัวหนึ่งชนลิมิต

ปัญหาที่แก้: dev ที่ใช้ AI หลายตัวไม่ได้เจ็บเพราะโมเดลไม่เก่ง แต่เจ็บเพราะ **บริบทขาดทุกครั้งที่เปลี่ยนเครื่องมือ**

สถานะ: **v0.2.x** — Rust core + SwiftUI shell บน macOS ใช้งานได้ ดูรายละเอียดรุ่นใน [CHANGELOG.md](CHANGELOG.md)
(Windows/WinUI 3 ยังไม่เริ่ม — core ไม่มี logic ใน Swift เพื่อให้พอร์ตได้โดยไม่แก้ core)

---

## ทำอะไรได้ตอนนี้

- หลาย agent ใน session เดียว — **ledger** (SQLite) เป็นแหล่งความจริง ไม่ใช่ session ของ provider
- ส่งไม้ต่อ (handoff) พร้อม brief + diff จาก checkpoint และ failover อัตโนมัติเมื่อชน 429 / quota
- compare (fork ต่อ agent), plan mode, rewind ทั้งแชทและไฟล์
- ความจำของโปรเจกต์ผ่าน approve queue · skill ชุดเดียวฉายให้ทุกเจ้า · MCP config ก้อนเดียว
- review diff ทีละ hunk, verify loop, repo map ใน context
- sandbox (Seatbelt) กัน agent เขียนนอก workspace · กัน secret หลุด 3 ชั้น
- นำเข้า session จาก Claude Code / Codex / Gemini CLI

**ยังไม่มี:** semantic retrieval ของความจำ (ตอนนี้เรียงตาม pinned/hits/ความใหม่) · notarize/.app ที่แจกจ่ายได้ · Windows

## เริ่มยังไง

ต้องมี Rust (1.85+) และสำหรับแอป macOS: Xcode toolchain (Swift 5.9+)

```bash
cargo test --workspace
cargo run -p chorus-cli --bin chorus -- bench    # วัด invariant
cargo run -p chorus-cli --bin chorus -- doctor   # agent ไหนพร้อมใช้
apps/macos/build.sh                              # build + เปิดแอป
```

ลอง agent จริง (ต้อง login claude/codex ไว้แล้ว):

```bash
scripts/smoke-real-agents.sh claude codex        # เขียนไฟล์ · จำข้าม agent · failover
```

`bench` คือตัวจับ regression ที่เทสต์จับไม่ได้ ตัวเลขที่ห้ามถอย: **blob rows = 1** (dedup),
**assembled เป็นเศษเสี้ยวของ raw**, **demoted มาก่อน dropped เสมอ**, task state ไม่ว่าง,
และ `assemble_context` ต้องไม่เรียก summariser

## กฎที่ทุกอย่างตามมา

1. **storage ≠ context** — ดิสก์ปรับให้ไบต์น้อย/query เร็ว; ที่ส่งเข้าโมเดลปรับให้ token น้อย ประกอบใหม่ทุกครั้ง ไม่เขียนกลับดิสก์
2. **ledger เป็นแหล่งความจริงเดียว** — provider เป็นแค่ผู้ถูกเรียกมาตอบหนึ่งเทิร์น
3. **ห้ามมี logic ใน Swift** — ทุกอย่างอยู่ใน Rust, UI คุยผ่าน FFI/event stream

## โครงสร้าง

```
crates/chorus-core/    ledger, blob, memory, context assembler, handoff, checkpoint, compaction, …
crates/chorus-acp/     ACP client, bridge ของแต่ละเจ้า, sandbox
crates/chorus-cli/     คำสั่ง `chorus` + Session engine
crates/chorus-ffi/     C ABI (chorus.h) ที่ SwiftUI shell ลิงก์
crates/chorus-daemon/  chorusd headless engine
crates/chorus-eval/    harness eval
apps/macos/            SwiftUI shell
evals/handoff/         eval คุณภาพการส่งไม้ต่อ
docs/                  ref/ (design, storage spec), adr/, agents/
```

บริบทเต็มสำหรับคนและ agent ที่จะแก้โค้ด: [CLAUDE.md](CLAUDE.md) (ข้อตัดสินใจ กับดักที่เคยเหยียบ) และ [CONTEXT.md](CONTEXT.md)
