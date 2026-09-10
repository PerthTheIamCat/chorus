# Chorus — Design Blueprint v0.2 (macOS first)

> **Many agents. One score.**

Desktop app ที่ให้ AI agent หลายเจ้าทำงานใน session เดียวกัน ใช้ memory ก้อนเดียวกัน
เห็น skill ชุดเดียวกัน และส่งงานต่อกันได้เมื่อชนลิมิต

_(เดิมชื่อ Baton — เปลี่ยนเป็น Chorus แล้วทั้งเอกสารและโค้ด)_

Artifact ฉบับเต็ม (มีไดอะแกรม/wireframe): https://claude.ai/code/artifact/46e28c3e-8410-4446-ae5d-b7c0b865cc98
สเปกการเก็บข้อมูล: `claude/chorus-storage-spec.md`
สถานะโค้ด: `claude/chorus-phase0-status.md`

---

## 1. Pain point ที่แก้

- ความจำแยกกัน: `~/.claude`, Codex rollout, Gemini — ย้าย agent ทีต้องเล่าใหม่
- ชนลิมิตแล้วงานหยุด ต้อง copy บริบทไปวางเอง
- CLAUDE.md / AGENTS.md / GEMINI.md / MCP config ซ้ำซ้อนหลายที่

## 2. หลักการออกแบบ

1. **Session เป็นของแอป ไม่ใช่ของ provider** — เก็บ conversation ledger กลางเอง
2. **ไฟล์บนดิสก์คือความจริง** — state ที่ส่งต่อคือ diff + ไฟล์ที่แตะ ไม่ใช่ transcript ทั้งก้อน
3. **Handoff เป็น first-class** มี pipeline ของตัวเอง ไม่ใช่ปุ่ม copy
4. **Local-first** — ledger/memory/index อยู่บนเครื่อง (SQLite), cloud sync เป็น optional
5. **ยืมมาตรฐานที่มีอยู่** — MCP (tool), ACP (คุยกับ agent CLI), SKILL.md (skill)

## 3. สถาปัตยกรรม (Rust core + native shell)

- **Shell (native)** — SwiftUI (macOS ก่อน) / WinUI 3 (ทีหลัง)
- **Orchestrator** — Session Engine + Handoff Engine (คิว, repack, budget, failover)
- **State** — Conversation Ledger บน SQLite, ทุก event มี `author`
- **Memory** — Memory Store 3 ชั้น + projector
- **Capability** — Skill Registry + MCP Host
- **Safety** — Tool Broker + Permission Engine + sandbox + audit log
- **Providers** — adapter ต่อเจ้า

### Adapter

| Provider | ช่องทาง                                                      | ข้อควรระวัง                                                          |
| -------- | ------------------------------------------------------------ | -------------------------------------------------------------------- |
| Claude   | Agent SDK (`resume`/`fork`/`listSessions`) หรือ Messages API | session file เป็น local ต่อเครื่อง ต้องทำ SessionStore เองถ้าจะ sync |
| Codex    | Codex CLI ผ่าน ACP                                           | rollout log บวมได้ระดับ GB ต้อง prune                                |
| Gemini   | Gemini CLI ผ่าน ACP                                          | tool call format ต้อง normalize                                      |
| Local    | Ollama / LM Studio (OpenAI-compatible)                       | tool-use อ่อน ใช้กับงานเบา (สรุป/ตั้งชื่อ/embed)                     |

ACP = JSON-RPC มาตรฐานเปิดของ Zed มี `session/new`, `session/load`, `session/prompt`,
`session/update`, `session/request_permission`, terminal, diff — ต่อครั้งเดียวรองรับ agent ได้หลายสิบตัว

## 4. Memory 3 ชั้น + Projector

| ชั้น      | เก็บอะไร                                         | ที่เก็บ              | อายุ         |
| --------- | ------------------------------------------------ | -------------------- | ------------ |
| Working   | เทิร์นล่าสุด, tool result, diff ที่ยังไม่ commit | ledger (SQLite)      | ตลอด session |
| Project   | สถาปัตยกรรม, convention, คำสั่ง build/test       | `.chorus/project.md` | ตลอดโปรเจกต์ |
| Long-term | fact เกี่ยวกับผู้ใช้, สรุป session เก่า          | SQLite + sqlite-vec  | ข้ามโปรเจกต์ |

**Projector**: ก่อนเรียก agent ใด render memory กลางออกเป็น `CLAUDE.md` / `AGENTS.md` /
`GEMINI.md` / system prompt แล้ว mount เข้า workdir ชั่วคราว — ไฟล์ generated มี header
`<!-- chorus:generated -->` และ detect drift ถ้าผู้ใช้แก้เอง

**กติกา**: agent เขียน memory ตรง ๆ ไม่ได้ ต้องผ่าน tool `memory.propose` → approve queue

## 5. Handoff pipeline (5 ขั้น)

1. **Freeze** — หยุด graceful, snapshot ไฟล์
2. **Repack** — บีบ ledger เป็น context ขนาดพอดีปลายทาง
3. **Brief** — สร้าง structured handoff brief
4. **Verify** — ให้ agent ใหม่ echo-back ความเข้าใจก่อนลงมือ
5. **Resume** — ทำต่อในเทรดเดิม มี divider บอกว่าเปลี่ยนมือตรงไหน/เพราะอะไร

Handoff brief: `goal`, `done[]`, `pending[]`, `constraints[]`, `files[]`, `diff`,
`handoff{from,to,reason,at}`

**Relay chain**: ตั้งลำดับล่วงหน้า เช่น Claude → Codex → Gemini → local; เจอ 429/quota
exhausted ระบบสลับเองแล้วเด้ง toast

## 6. Native — macOS ก่อน

แกน Rust ก้อนเดียว ผูกผ่าน UniFFI (Swift) และภายหลัง C ABI/csbindgen (C#)

| ส่วน        | macOS (ตอนนี้)                      | Windows (ทีหลัง)           |
| ----------- | ----------------------------------- | -------------------------- |
| UI          | SwiftUI + AppKit                    | WinUI 3 (Windows App SDK)  |
| Editor/diff | NSTextView หรือ Monaco ใน WKWebView | WebView2 + Monaco          |
| Terminal    | SwiftTerm                           | ConPTY                     |
| Sandbox     | Seatbelt + App Sandbox              | AppContainer + Job Object  |
| Key         | Keychain                            | Credential Manager / DPAPI |

**ตัดสินใจแล้ว**: เขียน SwiftUI เป็น shell ตัวจริงตั้งแต่แรก ไม่แวะ Tauri
แต่ **ห้ามมี logic ใน Swift** — ทุกอย่างอยู่ใน Rust คุยผ่าน event stream ทางเดียว
เพื่อให้วันที่ทำ WinUI 3 ไม่ต้องแก้ core

## 7. UI/UX

Layout: sessions ซ้าย · thread กลาง · context+budget ขวา

- Agent switcher: `@codex` ในช่องพิมพ์ (เทิร์นเดียว) หรือ ⌘K (ถาวรทั้งเทรด)
- Provenance ทุกบรรทัด (agent blame)
- Memory inspector แก้ได้ + preview ไฟล์ที่จะ generate
- Diff review เป็นศูนย์กลาง: accept/reject ราย hunk, checkpoint, "ให้ agent อื่นรีวิว"
- Permission 3 ระดับต่อ agent, คำสั่งอันตรายถามเสมอ
- Compare mode: ยิงโจทย์เดียวหลาย agent แล้วเทียบ diff

## 8. Roadmap (macOS first)

- **Phase 0 (2–3 สัปดาห์)** — Rust core: ledger + blob store + checkpoint; adapter Claude; context assembler; bench ตั้งแต่วันแรก ✅ _ทำแล้ว_
- **Phase 1 (~5 สัปดาห์)** — SwiftUI shell ตัวจริง; compaction L1/L2 ด้วยโมเดล local + MLX embedding; memory store + projector
- **Phase 2 (~6 สัปดาห์)** — Codex ผ่าน ACP + handoff ครบ 5 ขั้น + relay chain; MCP host + permission + Seatbelt; context inspector
- **Phase 3 (~6 สัปดาห์)** — Gemini + Ollama adapter, compare mode, provenance blame; Universal Binary + notarize
- **ทีหลัง** — WinUI 3 shell (core ไม่ต้องแก้), multi-agent พร้อมกัน, E2E sync, team memory

## 9. ความเสี่ยง

- **Context repack ห่วย = แอปไร้ค่า** ต้องมี eval วัดว่า handoff แล้วงานเดินต่อได้กี่ %
- **ToS ของแต่ละ provider** ต่างกัน ออกแบบให้ผู้ใช้ใส่ credential ตัวเองเสมอ
- **Ledger บวม** ต้อง compaction + เก็บ output ใหญ่แยกไฟล์ตั้งแต่วันแรก
- **Memory เน่า** ต้อง approve queue + TTL ของ fact

## อ้างอิง

- Claude Agent SDK — Work with sessions: https://code.claude.com/docs/en/agent-sdk/sessions
- Agent Client Protocol overview: https://agentclientprotocol.com/protocol/overview
- Zed ACP: https://zed.dev/acp
- Choose a Windows development path: https://learn.microsoft.com/en-us/windows/apps/get-started/
- openai/codex #24948 (session log บวม): https://github.com/openai/codex/issues/24948
