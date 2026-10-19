# Changelog

การเปลี่ยนแปลงของ Chorus ในแต่ละเวอร์ชัน รูปแบบตาม [Keep a Changelog](https://keepachangelog.com/)
หัวข้อ `## [x.y.z]` ต้องตรงกับเลขเวอร์ชัน เพราะ `release.sh` ตัดส่วนนี้ไปเป็น release notes

## [Unreleased]

### Added

- **นำเข้า session จากแอปอื่น:** ค้นหาบทสนทนาของ Claude Code, Codex และ Gemini CLI ที่อยู่ในเครื่อง (⇧⌘F › จากแอปอื่น หรือ `chorus sessions`) แล้วนำเข้ามาคุยต่อใน Chorus กับ agent ตัวไหนก็ได้ นำเข้าซ้ำไม่สร้าง session ใหม่ ข้อความที่เพิ่มทีหลังจะต่อท้ายให้
- **`chorus doctor` แสดง error ที่ failover ไม่รู้จัก** เพื่อให้เห็นข้อความลิมิตรูปแบบใหม่ก่อนที่ failover จะเงียบหาย
- **CI บน macOS:** test, clippy, bench และ swift build/test ทุก push และ pull request

### Fixed

- **ค้นหาข้าม session ไม่เจออะไรเลย:** ช่องค้นหา (⇧⌘F) และ export Markdown อ่านคอลัมน์ที่ไม่มีอยู่จริง จึง error ทุกครั้ง และข้อความยาวที่เก็บแยกไว้ export ไม่ได้
- **bridge ของ Antigravity อาจค้าง** ถ้าอ่าน output ของ agent พังกลางทาง

## [0.1.4] - 2026-10-17

### Added

- **มีอะไรใหม่:** หน้า Settings › มีอะไรใหม่ แสดง changelog นี้ในแอป และหน้าต่างอัปเดตของ Sparkle แสดงรายการของเวอร์ชันใหม่ก่อนกดติดตั้ง
- **ตรวจหาอัปเดตจากหน้าเกี่ยวกับ:** ปุ่ม Check for Updates… อยู่ข้างเลขเวอร์ชัน พร้อมบอกว่าตรวจล่าสุดเมื่อไหร่

### Fixed

- **เลขเวอร์ชันค้างที่ 0.1.0:** build ในเครื่องเอาเลขจาก git tag ล่าสุดแทนค่าที่เขียนตายไว้ ซึ่งทำให้ทุก build รายงาน v0.1.0 แม้ปล่อย 0.1.3 ไปแล้ว

## [0.1.3] - 2026-10-17

0.1.2 มี tag แต่ไม่เคยถูกปล่อย (build บน CI ไม่ผ่าน) ทุกอย่างของ 0.1.2 รวมอยู่ในเวอร์ชันนี้

### Added

- **Workbench layout v2:** สี่คอลัมน์ — sessions │ file tree │ file viewer │ chat โดย viewer กับ chat แบ่งพื้นที่ราว 1.2 : 1 และจอแคบ (< 1200 px) ไฟล์ขึ้นแทน chat ชั่วคราว
- **ไอคอนไฟล์ตามชนิด** ใน file tree (Catppuccin)
- **Sub sessions** ซ้อนใต้ session แม่ใน sidebar พร้อมจุดสถานะ ชิปโมเดล และ breadcrumb
- **Orchestra mode:** โมเดลหลักวางแผนแล้วแจกงานให้ agent ย่อยใน worktree แยก มี checklist สด เพดานงาน และ role library
- **Route presets:** Economy / Balanced / Max สำหรับ Auto route
- **Composer v2:** วาง HTML จากเว็บแล้วแปลงเป็น Markdown และรูปแนบแสดง thumbnail จริง
- **Settings v2:** แถบหมวดด้านซ้าย และปรับถ้อยคำทั้งแอป
- **System Monitor** (⌥⌘M): port, process และพื้นที่ที่ Chorus ใช้
- **ค่าใช้จ่าย API:** แสดงเป็นบาท พร้อมแถบโควตา
- **Live updates:** แผงต่าง ๆ อัปเดตระหว่างเทิร์น ไม่ต้องรอจบ
- **Subagent ของ Claude และ Codex** แสดงในรูปเดียวกัน
- **Review baseline:** จำจุดที่ตรวจแล้ว diff ครั้งต่อไปเริ่มจากตรงนั้น

### Fixed

- **File viewer แคบเกินไป:** breadcrumb กับปุ่มขึ้นบรรทัดเดียวเสมอ (ย่อเป็นไอคอนเมื่อแคบ) และโค้ดบรรทัดยาวเลื่อนแนวนอนได้แทนถูกตัด
- **ปุ่มใน composer ขึ้นบรรทัดใหม่:** ป้ายอยู่บรรทัดเดียว ถ้าที่ไม่พอเหลือแค่ไอคอนพร้อม tooltip
- **System Monitor ซ้ำสองอันในเมนู Window**
- **Sandbox:** ปิดช่องที่ agent หลุดออกนอก sandbox และจุดอ่อนของ control surface
- **Context meter สด, chat ติดล่าง, diff ลื่นขึ้น** และงานขัดเกลาจาก UX triage

## [0.1.1] - 2026-10-16

### Fixed

- **เปิดจาก Finder/Dock แล้วหา agent ไม่เจอ:** แอปเติม PATH จาก login shell ให้ agent หา `gh` / `node` / `npx` เจอ และใช้ SSH key ของเครื่องได้
- **ไอคอนแอป** ขึ้นใน Dock และ Finder

## [0.1.0] - 2026-10-16

รุ่นแรกที่ปล่อยให้ติดตั้ง

### Added

- **หลาย agent ใน session เดียว:** Claude, Codex, Gemini ใช้ ledger และความจำก้อนเดียวกัน ส่งไม้ต่อกันกลางงานได้ และ failover อัตโนมัติเมื่อตัวใดตัวหนึ่งติดลิมิต
- **Auto route:** router เลือกโมเดลให้ทุกเทิร์น พร้อมสรุปว่าเลือกเพราะอะไร
- **Diff review ทีละ hunk**, checkpoint, ย้อนไฟล์ และขออนุญาตทีละคำขอ
- **Seatbelt sandbox:** agent เขียนได้เฉพาะในโฟลเดอร์งาน
- **Memory ที่ใช้ร่วมกันทั้งโปรเจกต์** ผ่านคิวอนุมัติ
- **Skills และ MCP config ชุดเดียว** ส่งให้ทุก agent
- **ติดตั้งเครื่องมือเสริม** จาก catalog และถอดออกได้หมดจด
- **หลายบัญชีต่อ agent** พร้อมแถบโควตา 5 ชั่วโมงและรายสัปดาห์
- **อัปเดตอัตโนมัติ** ผ่าน Sparkle
