#!/usr/bin/env python3
"""สรุปผล eval ของ handoff — รัน: python3 score.py"""
import json, pathlib, sys

KEYS = ["understood_goal", "no_repeat_work", "respected_constraints",
        "proceeded_unaided", "output_usable"]
PASS_AT = 8

cases_dir = pathlib.Path(__file__).parent / "cases"
cases = sorted(p for p in cases_dir.glob("*.json")
               if not p.name.startswith("_"))
if not cases:
    sys.exit("ยังไม่มีเคสใน cases/ — ก๊อป _template.json แล้วเริ่มกรอกได้เลย")

rows, passed, per_key = [], 0, {k: 0 for k in KEYS}
for p in cases:
    c = json.loads(p.read_text(encoding="utf-8"))
    r = c.get("result", {})
    total = sum(int(r.get(k, 0)) for k in KEYS)
    for k in KEYS:
        per_key[k] += int(r.get(k, 0))
    ok = total >= PASS_AT
    passed += ok
    rows.append((c.get("id", p.stem), total, ok, c.get("failure_note", "")))

print(f"{'case':<28} {'คะแนน':>6}  ผล")
for cid, total, ok, note in rows:
    print(f"{cid:<28} {total:>4}/10  {'ผ่าน' if ok else 'ตก'}")
    if note:
        print(f"{'':<28}        ↳ {note}")

n = len(rows)
print(f"\nผ่าน {passed}/{n}  =  {passed / n * 100:.0f}%   (เป้า ≥ 70%)")
print("\nจุดอ่อนรายข้อ (เต็ม {}):".format(n * 2))
for k, v in sorted(per_key.items(), key=lambda kv: kv[1]):
    print(f"  {k:<24} {v:>3}/{n * 2}")
