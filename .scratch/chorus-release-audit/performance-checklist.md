# Performance readiness — Chorus รุ่นแจกผู้ใช้ทั่วไป

วันที่ค้นคว้า: 2026-10-12 · สถานะ: **ยังไม่ได้วัดจริง** · ขอบเขต: Swift UI, Rust/FFI, storage/search, daemon/remote, subprocess และ local AI

เอกสารนี้เสนอวิธีวัดและงบประมาณเริ่มต้น ไม่ใช่ผล benchmark ไม่ใช่ข้อกำหนดที่ผู้ใช้ตกลงแล้ว และยังไม่รับรองเครื่องขั้นต่ำ รองรับทุกฟีเจอร์โดยแยกผลค่าเริ่มต้นกับเมื่อเปิดใช้งาน

## หลักฐานและขอบเขตข้อสรุป

- Apple แนะนำ main-thread work สำหรับ discrete interaction ต่ำกว่า 100 ms และงานเตรียม UI ต่อเฟรมประมาณต่ำกว่า 5 ms; เป็นแนวทาง responsiveness ไม่ใช่ SLA ของ Chorus หรือเวลา network/model ทั้งคำขอ ([Improving app responsiveness](https://developer.apple.com/documentation/xcode/improving-app-responsiveness)).
- วัด startup ถึงจุดที่ผู้ใช้ทำงานได้จริงด้วย เพราะการวาดเฟรมแรกอาจยังไม่พร้อมใช้งาน; App Launch ใน Instruments ช่วยแยกเวลาทำงานและเวลารอ thread ([Reducing launch time](https://developer.apple.com/documentation/xcode/reducing-your-app-s-launch-time)).
- ใช้ Allocations และ generation marks ตรวจการสะสมหลังทำงานซ้ำ; leak scan อย่างเดียวไม่พิสูจน์ว่า cache/ข้อมูลที่ยังมี reference มีขนาดเหมาะสม ([Gathering memory information](https://developer.apple.com/documentation/xcode/gathering-information-about-memory-use)).
- Energy Impact เป็นค่าความสิ้นเปลืองแบบสัมพัทธ์ ไม่ใช่ watts; เปรียบเทียบเครื่องเดียวและสถานะเดียว พร้อม App Nap/Preventing Sleep ([Activity Monitor energy](https://support.apple.com/guide/activity-monitor/view-energy-consumption-actmntr43697/mac)).
- Ollama มี duration หน่วย ns แยก load, prompt evaluation และ token generation; ใช้แยกต้นทุนโมเดลจากแอป และคำนวณ tokens/s = eval_count × 1e9 / eval_duration เมื่อ duration > 0 ([Generate API](https://docs.ollama.com/api/generate)).

## Protocol ที่เสนอให้ใช้ทุกชุด

- [ ] เครื่องทดลองเริ่มต้น: Apple Silicon 8 GB เป็นเครื่องสำรวจข้อจำกัด และ 16 GB เป็นเครื่องเปรียบเทียบ; รุ่น CPU/GPU และ macOS ต้องบันทึกจริงก่อนวัด หากจะรองรับ Intel ให้เพิ่มเครื่องจริง ไม่อนุมานผลจาก ARM
- [ ] บันทึก commit + สถานะ dirty, artifact hash, release/debug flags, Swift/Rust/Xcode/Ollama versions, OS build, RAM, SSD/free space, display refresh/scale, AC/battery, Low Power Mode, thermal state, background load และ feature flags
- [ ] ใช้ release artifact เดิมทุก run; แยก run ที่ไม่มี profiler สำหรับ timing กับ run Instruments สำหรับหาสาเหตุ ไม่รวมสองกลุ่มใน percentile เดียว
- [ ] fixture สังเคราะห์มี seed/hash/count/bytes และภาษาไทย+อังกฤษ; ห้ามนำแชตจริงหรือข้อมูลลับเข้า trace บันทึกเฉพาะ ID/ขนาด/เวลา
- [ ] แยก first-ever launch, process-cold launch (ปิดโปรเซสแล้วเปิด), post-reboot launch และ warm/repeated workload; ระบุ OS page cache และ model residency แยกกัน ห้ามเรียกทุกแบบว่า cold เหมือนกัน
- [ ] latency ปกติ: 3 warm-up ไม่นับผล + 30 measured runs ต่อ scenario; รายงาน n, p50, p95 nearest-rank, max, failures/timeouts และข้อมูลดิบ ไม่ซ่อน failed runs; p99 ใช้เมื่อมีอย่างน้อย 100 observations และระบุความไม่แน่นอน
- [ ] post-reboot อย่างน้อย 5 runs รายงานแต่ละค่า/range ไม่อ้าง p95 ที่มั่นคง; แยก reboot ครั้งแรกออกจากการเปิดซ้ำ ใช้ steady state และพักเครื่องก่อนเทียบ A/B
- [ ] CPU ให้ระบุหน่วย 100% = หนึ่ง logical core; memory แยก app/daemon/child/model, peak/settled footprint, compressed memory และ system memory pressure/swap ห้ามรวม shared mappings แล้วเรียกว่า RAM จริงโดยไม่ระบุวิธี

## Checklist และงบประมาณเริ่มต้นที่เสนอ

**ตัวเลขด้านล่างเป็นสมมติฐานเพื่อเริ่ม baseline ทั้งหมด ยังไม่มีผลผ่าน/ไม่ผ่าน** ยืนยันหรือปรับหลังการวัดชุดแรกบนเครื่องที่ตกลง ค่า latency เป็น p95 ยกเว้นระบุอื่น

| ตรวจ | Workload ที่ทำซ้ำได้ | วัด/เครื่องมือ | งบประมาณเสนอและการตัดสิน |
|---|---|---|---|
| [ ] เปิดแอป | profile ว่างและ profile 10,000 messages; เปิดถึง composer พิมพ์ได้และรายการพร้อม | Instruments App Launch; elapsed ถึง interactive | warm ≤ 1 s, process-cold ≤ 2 s; first-ever/post-reboot เก็บ baseline ก่อน |
| [ ] UI/streaming | 100, 1,000, 10,000 messages ขนาดเฉลี่ย 1 KiB; code blocks 20%; replay 50 และ 200 chunks/s ครั้งละ 20 UTF-8 bytes นาน 60 s; scroll/type ระหว่าง stream | Time Profiler, UI/hitch trace, event→paint, received→visible | feedback ≤ 100 ms; received→visible ≤ 100 ms; ใช้แนวทาง Apple ตรวจ main thread; ไม่มี freeze ≥ 250 ms ใน fixture |
| [ ] แชตยาวและ memory | เปิด/ปิดแชตเดิม 100 รอบ แล้วรอ 60 s; stream ต่อเนื่อง 30 min | Allocations generations, app+child footprints, memory slope | หลัง warm-up และ 100 รอบ settled growth ≤ max(10%, 50 MiB); ตรวจสาเหตุถ้าโตตามรอบ; เสนอ idle app+daemon ≤ 300 MiB โดยไม่รวม model |
| [ ] FFI | เรียก read-only operation จริงแบบเดิม 1,000 ครั้ง ขนาด payload 1 KiB/100 KiB/1 MiB; 1/4/8 concurrent callers | interval แยก Swift serialize → Rust/lock/I/O → decode → UI; Time Profiler | ยังไม่ตั้ง blanket FFI SLA; operation ที่ผูก UI ต้องผ่าน feedback budget; ขนาด payload เพิ่มแล้วตรวจ copying/lock contention |
| [ ] Ledger/search | fixture 1k/10k/100k events, ขนาดรวมและ index state คงที่; 30 คำค้น fixed ที่ครอบคลุม hit/miss/TH/EN | wall time, read/write bytes, locks; cold/warm แยก | warm search ≤ 200 ms ที่ 100k events; cold ≤ 1 s; ผลลัพธ์ถูกต้อง ไม่โหลดผลทั้งหมดเพื่อวาดหน้าแรก |
| [ ] Repository ใหญ่ | fixture 1k/10k/100k source files, 2 KiB/file; เพิ่ม ignored tree 100k files; symlink cycle และ binary แยก scenario | open/scan/search latency, disk I/O, cancellation, peak memory | แสดงสถานะเริ่มงาน ≤ 100 ms; cancel feedback ≤ 100 ms และงานหยุด ≤ 1 s; scan duration เก็บ baseline ก่อน |
| [ ] Daemon/remote | loopback ก่อน แล้ว LAN fixture; clients 1/4/16, request body คงที่ 1 KiB; mix health/read และ slow consumer 10 min | request p50/p95/max, queue depth, errors, FD/thread/process count, bytes | local health/read ≤ 100 ms ที่ 4 clients; 16 เป็น stress สำรวจ; bounded queue/timeout และกลับ idle หลังจบ |
| [ ] Subprocess/agents | fake local worker deterministic 1/4/8 ตัว ส่ง 10 KiB/s ต่อ worker 60 s; success/failure/hang/cancel | spawn-to-ready, IPC lag, UI responsiveness, orphan/FD counts | cancel งานที่แอปเป็นเจ้าของแล้วไม่มี orphan หลัง grace period ที่กำหนด; ยังไม่ตั้ง spawn SLA ก่อน baseline |
| [ ] Local AI | model digest/quantization/context/settings คงที่; TH/EN prompt 128/2k/8k tokens, output cap 128; unloaded vs resident; sequential ก่อน concurrent=2 | TTFT, total time, load/prompt/eval ns, token/s, model footprint, UI lag | router resident ≤ 1 s เป็นเป้าหมายทดลอง; generation throughput/TTFT กำหนดหลังวัดแต่ละ model+เครื่อง ห้ามเทียบคนละ prompt หรือ cache state |
| [ ] Download | disposable fixture server/ปลายทางทดสอบ; ขนาด 100 MiB/2 GiB, จำกัด 10/100 Mbps, interrupt/resume/cancel; ไม่ใช้ registry จริงเพื่อ baseline | throughput, disk writes, progress cadence, cancel latency, temp bytes | UI feedback ≤ 100 ms; ไม่มี full-file buffer ใน RAM; cleanup/resume ถูกต้องตาม policy; transfer time ไม่รวมเป็น inference time |
| [ ] Idle/energy | foreground/background/minimized อย่างละ 10 min หลัง settle 2 min; daemon off/on, model unloaded/resident แยก; ไม่มี task ค้าง | Activity Monitor CPU/Energy/App Nap/sleep, Time Profiler หา wakeups | app+daemon mean CPU ≤ 1% ของหนึ่ง core; ไม่ prevent sleep โดยไร้งาน; Energy Impact ใช้ paired comparison ไม่ตั้งเลขสากล |
| [ ] Soak/recovery | 2 h สลับ chat/search/cancel deterministic ทุก 30 s แล้ว idle 10 min; offline/reconnect ใน test fixture | slopes, queue/process/FD count, memory, disk growth, recovery time | ไม่โตไม่จำกัด; กลับ idle budget; repeated failure ไม่เกิด busy retry loop |

## ก่อนตัดสินพร้อมแจก

1. เลือกเครื่องจริงและ release artifact แล้วบันทึก baseline; ค่าเป้าหมายที่ยังไม่มีข้อมูลให้เป็น **proposed** ต่อไป
2. ใช้แอป fixture เดิมเทียบการแก้ไข: p95 หรือ peak เพิ่ม > 10% และเกินความแปรปรวนเดิมให้เปิด investigation ไม่ตัดสินด้วย run เดียว
3. เก็บตารางต่อ scenario: metadata, n, cold/warm, p50/p95/max, errors, peak/settled memory, trace path, verdict, owner; verdict มี measured pass/fail หรือ unmeasured เท่านั้น
4. เลือก minimum supported hardware หลังทดสอบ default กับ optional local AI แยกกัน และตกลงข้อจำกัด workload; ความเร็ว cloud provider ไม่ใช่ความเร็ว UI

## สิ่งที่มีใน repo และสิ่งที่ยังขาด

[Harness README](../../evals/harness/README.md) อธิบาย invariant/router quality gates และ [Router README](../../evals/router/README.md) อธิบาย warm-up, nearest-rank p95 และ TH/EN smoke set สิ่งเหล่านี้เป็นจุดต่อยอด ไม่ใช่หลักฐานว่า desktop ผ่าน checklist นี้ และไม่ได้รันในงานนี้

ยังขาด: fixture manifests, release benchmark protocol ที่รันจริง, UI/FFI timing spans ที่ยืนยันแล้ว, raw timing traces, hardware matrix และ budget ที่ทีมรับรอง เอกสารนี้ไม่ได้แก้ source, build, benchmark, ดาวน์โหลด model หรือแทนที่แอป
