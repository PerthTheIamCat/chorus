# Which workloads and measurements should define Chorus performance readiness?

Parent: ../map.md
Labels: wayfinder:research
Type: research
Status: resolved
Assignee: performance_criteria
Blocked by: none

## Question

Using current primary sources, propose reproducible macOS performance measurements for startup, long chat rendering/streaming, UI responsiveness, Rust/Swift FFI, ledger/search, daemon/subprocess concurrency, local model inference/downloads, idle CPU/memory and energy. Specify hardware/build/workload metadata, cold/warm runs, percentiles and tools. Separate official responsiveness guidance from provisional product budgets; never invent observed measurements or claim a minimum supported machine is agreed.

## Answer

2026-10-12 — [Performance readiness checklist](../performance-checklist.md) เสนอ reproducible fixtures, metadata, cold/warm protocol, percentiles และรายการวัดครบ UI/FFI/storage/daemon/subprocess/local AI/idle/energy พร้อมแหล่งหลัก Apple และ Ollama จำนวน 5 แหล่ง งบประมาณเป็นข้อเสนอทั้งหมด; ยังไม่ได้วัดหรือรับรองเครื่องขั้นต่ำ ไม่ได้ build/run/download หรือแก้ source
