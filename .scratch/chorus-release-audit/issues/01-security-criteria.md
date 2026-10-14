# Which security controls and checks apply to a public Chorus release?

Parent: ../map.md
Labels: wayfinder:research
Type: research
Status: resolved
Assignee: security_criteria
Blocked by: none

## Question

Using current primary sources, define an actionable checklist for public macOS distribution and this app's trust boundaries: agent execution, permission mediation, repository/MCP content, local daemon and remote pairing, Keychain/secrets, previews, model downloads, updates and dependency advisories. Distinguish mandatory platform requirements from recommended hardening and application-specific tests. Include verification methods and evidence needed, not claims that Chorus already passes or fails.

## Answer

Resolved 2026-10-12 by security_criteria. [Security checklist สำหรับ Chorus รุ่นแจกผู้ใช้ทั่วไป](../security-checklist.md) แยกข้อกำหนดการแจก macOS ตามช่องทางจาก hardening และ negative tests ครบ trust boundaries พร้อมหลักฐานขั้นต่ำและเกณฑ์จัดลำดับงาน อ้างอิง Apple, MCP, OWASP, Sparkle และ RustSec โดยไม่ตัดสินว่าโค้ดปัจจุบันผ่านหรือไม่ผ่าน

Local artifacts only; no branch or commit, per repository instructions. Validation: read back checklist and ticket; research only, no code tests/builds performed.
