# Spike 2 — วัดคุณภาพ handoff (ยังไม่ต้องมีแอป)

คำถามที่แพงที่สุดของโปรเจกต์นี้ไม่ใช่ "เขียน core ยังไง" แต่คือ
**ส่ง brief ไปแล้ว agent ตัวใหม่ทำงานต่อได้จริงกี่ %**
ถ้าตอบต่ำกว่า ~70% ปัญหาอยู่ที่ format ของ brief ไม่ใช่ที่ engineering
และควรแก้ตรงนั้นก่อนลงแรงเขียนแอปต่อ

## วิธีทำ (ทำมือ ใช้เวลา 2–3 วัน)

1. เลือกงานจริงที่เคยทำ 10 งาน ใส่ไว้ใน `cases/NN-ชื่องาน.json` ตาม `_template.json`
2. หยุดงานกลางคัน แล้วกรอก brief ตาม `brief-template.md` (goal / done / pending / constraints / files / diff)
3. เอา brief ไปวางให้ **อีกโมเดลหนึ่ง** ที่ไม่เคยเห็นบทสนทนาเดิม
4. ให้คะแนนตามเกณฑ์ข้างล่าง บันทึกลง `result` ในไฟล์เดียวกัน
5. `python3 score.py` เพื่อดูสรุป

### รันเคสสดกับ Codex (ใช้ ACP bridge ที่มีอยู่แล้ว)

ให้ผลเป็นกลางกว่าเอาไปวางในแชทเราเอง เพราะ Codex ไม่เห็น context อะไรเลยนอกจาก brief:

```bash
# 1. ก๊อป repo ไป throwaway (ไม่เอา target/.git) แล้ว commit baseline
rsync -a --exclude target --exclude .git --exclude '.chorus*' ./ /tmp/chorus-handoff-NN/
cd /tmp/chorus-handoff-NN && git init -q && git add -A && git commit -qm baseline

# 2. ยิง brief เป็น prompt เดียว ให้ Codex แก้ในโฟลเดอร์ทิ้ง
cargo run --manifest-path <repo>/Cargo.toml --bin acp-probe -- \
  --cwd /tmp/chorus-handoff-NN --allow --prompt "$(cat brief-NN.txt)" \
  -- npx -y @agentclientprotocol/codex-acp

# 3. ดูว่ามันทำอะไร แล้วให้คะแนน
cd /tmp/chorus-handoff-NN && git diff && cargo test
```

เก็บ transcript (`.chorus-probe/acp-probe-*.jsonl`) กับ `git diff` ไว้เป็นหลักฐานของเคสนั้น

## เกณฑ์ให้คะแนน (0–2 ต่อข้อ รวม 10)

| ข้อ | 0 | 1 | 2 |
|---|---|---|---|
| เข้าใจเป้าหมาย | หลงประเด็น | ถูกกว้าง ๆ | ตรงเป๊ะ |
| ไม่ทำซ้ำสิ่งที่ทำไปแล้ว | ทำซ้ำหมด | ทำซ้ำบางส่วน | ไม่ทำซ้ำเลย |
| เคารพ constraints | ละเมิด | ถามก่อน | ทำตามครบ |
| ไปต่อได้เองโดยไม่ต้องถาม | ถามทุกอย่าง | ถาม 1–2 ข้อ | ไปต่อได้เลย |
| ผลลัพธ์ใช้ได้จริง | ต้องรื้อ | แก้นิดหน่อย | ใช้ได้เลย |

ผ่าน = ได้ 8/10 ขึ้นไป

## สิ่งที่ต้องบันทึกด้วยเสมอ

เวลาเคสไหนตก ให้เขียน `failure_note` ว่า **ข้อมูลอะไรที่ขาดไปจาก brief** —
นั่นคือ output จริงของสไปก์นี้ ไม่ใช่ตัวเลข % เฉย ๆ
