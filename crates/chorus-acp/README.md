# chorus-acp — สไปก์ที่ 1

ตอบสามคำถามที่ adapter layer ทั้งชั้นพิงอยู่ **ก่อน** ที่เราจะเขียน adapter จริง

1. `session/load` ใช้ได้จริงไหม และมันคืนอะไรกลับมา
2. permission flow คุมจากฝั่งเราได้แค่ไหน
3. diff / tool call มาเป็นของมีโครงสร้าง หรือมาเป็นข้อความให้เราแกะเอง

โค้ดในนี้ **ไม่ตีความอะไรเลย** — มันแค่คุย ACP หนึ่งรอบแล้วจดทุกเฟรมลงไฟล์
ผลสรุปที่พิมพ์ออกมาเป็นแค่การอ่าน transcript ให้ฟัง ของจริงคือไฟล์ `.jsonl`

## รัน

```bash
mkdir -p /tmp/acp-scratch && cd /tmp/acp-scratch && git init -q && echo '# scratch' > README.md

# ลองกับ agent ปลอมในตัวก่อน (ไม่ต้องมี API key ไม่ต้องต่อเน็ต)
cargo run --bin acp-probe -- --cwd /tmp/acp-scratch --mock

# ของจริง
cargo run --bin acp-probe -- --cwd /tmp/acp-scratch -- codex acp
cargo run --bin acp-probe -- --cwd /tmp/acp-scratch -- gemini --experimental-acp
```

ถ้า `codex acp` ขึ้นว่า `unrecognized subcommand` แปลว่าเวอร์ชันที่ลงไว้ยังไม่มีโหมด ACP ในตัว
probe จะบอกให้เองพร้อม stderr ของ codex — ลอง `codex --help` หาคำสั่งที่ใช่
หรือใช้ bridge แยกแล้วชี้ path ไปที่ binary นั้นตรง ๆ probe ไม่สนว่าใครอยู่ปลายท่อ

## ธง

| ธง | ทำอะไร |
|---|---|
| `--cwd DIR` | โฟลเดอร์ที่ให้ agent ทำงาน (ควรเป็นที่ทิ้งได้) |
| `--out FILE` | ไฟล์ transcript (ดีฟอลต์ `acp-probe-<ts>.jsonl`) |
| `--prompt TEXT` | เปลี่ยนโจทย์ ดีฟอลต์เป็นงานอ่านอย่างเดียว |
| `--allow` | ตอบ `allow_once` แทนที่จะปฏิเสธ **ใช้ในโฟลเดอร์ทิ้งเท่านั้น** |
| `--mock` | คุยกับ agent ปลอมที่ฝังอยู่ใน binary ตัวเดียวกัน ไม่ต้อง build อะไรเพิ่ม |

**ดีฟอลต์คือปฏิเสธทุก permission** ตั้งใจให้เป็นแบบนั้น — เราอยากเห็นรูปร่างของคำขอ
ไม่ได้อยากปล่อย agent เข้าไปในโปรเจกต์จริง ถ้าอยากเห็น diff ด้วยค่อยใส่ `--allow`
ในโฟลเดอร์ที่ลบทิ้งได้ (ถึงใส่ `--allow` probe ก็ยังไม่ยอมเขียนไฟล์ให้อยู่ดี)

## สิ่งที่ probe รับผิดชอบเอง

- `fs/read_text_file` — อ่านให้จริง แต่เฉพาะไฟล์ที่อยู่ใต้ `--cwd` เท่านั้น
- `fs/write_text_file` — ปฏิเสธเสมอ
- `session/request_permission` — เลือกตัวเลือกจาก `kind` ของมันเอง ไม่ใช่จากลำดับ

## เอาผลไปทำอะไรต่อ

- ถ้า **Q1 ตอบว่าไม่ replay** → ledger ของเราต้องเป็นแหล่งความจริงเดียว ไม่ต้องไปพึ่ง session ของ agent เลย ซึ่งตรงกับที่ออกแบบไว้
- ถ้า **Q2 ตอบว่าคุมได้** → permission engine ของเราแทรกตรงนี้ได้ ไม่ต้อง sandbox ซ้อน sandbox
- ถ้า **Q3 ตอบว่าได้แต่ข้อความ** → ต้องมี parser ต่อ agent ซึ่งเปราะมาก ให้คิดใหม่ว่าจะรองรับเจ้านั้นไหม
