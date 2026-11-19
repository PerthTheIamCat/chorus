import AppKit
import SwiftUI
import UniformTypeIdentifiers
import UserNotifications

// ContentView, split out of ContentView.swift: inspector panes (changes, memory, context).
// Moved verbatim; only `private` was dropped so the pieces can see each other.

extension ContentView {
    var changesPane: some View {
        let hunks = engine.diffHunks
        let files = Dictionary(grouping: hunks, by: \.file)
        let order = engine.diffFiles
        return VStack(spacing: 0) {
            PaneHeader(title: "ที่แก้ไปใน session นี้",
                       subtitle: hunks.isEmpty ? nil
                           : "\(order.count) ไฟล์ · +\(hunks.map(\.addCount).reduce(0, +)) −\(hunks.map(\.delCount).reduce(0, +))") {
                Button { engine.snapshot() } label: { Image(systemName: "camera") }
                    .buttonStyle(InteractiveStyle(cornerRadius: 6, padding: .init(top: 4, leading: 6, bottom: 4, trailing: 6)))
                    .disabled(engine.sessionID < 0)
                    .help("บันทึกจุดย้อนกลับตอนนี้")
            }
            if hunks.isEmpty {
                EmptyPane(icon: "checkmark.seal", title: "ยังไม่มีไฟล์ถูกแก้",
                          hint: "เมื่อ agent แก้ไฟล์ จะเห็นทีละก้อนที่นี่ เก็บหรือตีกลับได้ทีละก้อน")
            } else {
                ScrollView {
                    // Lazy and flat: one row per file header and per hunk,
                    // so only what is on screen is built.
                    LazyVStack(alignment: .leading, spacing: 6) {
                        ForEach(order, id: \.self) { f in
                            let fileHunks = files[f] ?? []
                            FileChangesHeader(file: f, hunks: fileHunks, open: !collapsedFiles.contains(f),
                                              isTest: engine.testFiles.contains(f)) {
                                if collapsedFiles.contains(f) { collapsedFiles.remove(f) } else { collapsedFiles.insert(f) }
                            }
                            if !collapsedFiles.contains(f) {
                                ForEach(fileHunks) { h in
                                    HunkView(hunk: h, rejected: rejected.contains(h.id)) {
                                        if rejected.contains(h.id) { rejected.remove(h.id) } else { rejected.insert(h.id) }
                                    }
                                }
                            }
                        }
                    }
                    .padding(10)
                }
                .chorusScrollIndicators()
                VStack(spacing: 6) {
                    if !rejected.isEmpty {
                        Button {
                            let patch = hunks.filter { rejected.contains($0.id) }.map(\.patch).joined()
                            engine.rejectHunks(patch)
                            rejected.removeAll()
                        } label: {
                            Label("ตีกลับ \(rejected.count) ก้อนที่เลือก", systemImage: "arrow.uturn.backward")
                                .frame(maxWidth: .infinity)
                                .foregroundStyle(.white)
                        }
                        .buttonStyle(InteractiveStyle(cornerRadius: 8))
                        .background(Color.red.opacity(0.85), in: RoundedRectangle(cornerRadius: 8))
                    }
                    Button { confirmRevert = true } label: {
                        Label("ย้อนทุกอย่างกลับไปตอนเริ่ม session", systemImage: "clock.arrow.circlepath")
                            .frame(maxWidth: .infinity)
                            .foregroundStyle(.red)
                    }
                    .buttonStyle(InteractiveStyle(cornerRadius: 8))
                    .confirmationDialog(
                        "ทิ้งทุกอย่างที่ agent แก้ใน session นี้?",
                        isPresented: $confirmRevert, titleVisibility: .visible
                    ) {
                        Button("ย้อนทั้งหมด", role: .destructive) { engine.revertToStart() }
                        Button("ยกเลิก", role: .cancel) {}
                    }
                }
                .padding(10)
            }
        }
        .frame(maxHeight: .infinity, alignment: .top)
    }

    /// The small label above a queued entry. What the entry *is* was decided in
    /// Rust (`kind`, `replaces`, `suspicious`); this only words it.
    static func proposalTag(_ p: Engine.Proposal) -> String? {
        var parts: [String] = []
        if p.suspicious == true { parts.append("⚠ อาจถูกวางยา — มาจากเว็บหรือ session ที่นำเข้า") }
        switch p.kind {
        case "lesson": parts.append("บทเรียนจากที่ผ่านมา")
        case "retire": parts.append("เสนอให้เลิกจำข้อนี้")
        default: if p.replaces != nil { parts.append("แทนที่ข้อที่จำไว้เดิม") }
        }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }

    var memoryPane: some View {
        VStack(spacing: 0) {
            PaneHeader(title: engine.memoryScope.map { "ความจำร่วม · \($0)" } ?? "ความจำของโฟลเดอร์นี้",
                       subtitle: "ทุก agent ใน session นี้อ่านชุดเดียวกัน") { EmptyView() }
            if engine.facts.isEmpty && engine.pending.isEmpty {
                EmptyPane(icon: "brain", title: "ยังไม่มีความจำ",
                          hint: "agent เสนอเรื่องที่ควรจำผ่าน memory_propose แล้วจะมารอให้คุณอนุมัติที่นี่")
            } else {
                ScrollView {
                    VStack(alignment: .leading, spacing: 8) {
                        if !engine.pending.isEmpty {
                            SectionLabel("รอคุณดู \(engine.pending.count) ข้อ", count: nil, tint: .orange)
                            ForEach(engine.pending) { p in
                                HStack(alignment: .top, spacing: 8) {
                                    VStack(alignment: .leading, spacing: 3) {
                                        if let tag = Self.proposalTag(p) {
                                            Text(tag).font(.caption2.weight(.semibold))
                                                .foregroundStyle(p.suspicious == true ? Color.red : Color.orange)
                                        }
                                        Text(p.text).font(.callout)
                                        if let why = p.evidence {
                                            Text(why).font(.caption2).foregroundStyle(.secondary)
                                        }
                                    }
                                    .frame(maxWidth: .infinity, alignment: .leading)
                                    Button { engine.review(p.id, approve: false) } label: { Image(systemName: "xmark") }
                                        .buttonStyle(InteractiveStyle(cornerRadius: 6, padding: .init(top: 4, leading: 6, bottom: 4, trailing: 6)))
                                        .foregroundStyle(.red).help("ปฏิเสธ")
                                    Button { engine.review(p.id, approve: true) } label: { Image(systemName: "checkmark") }
                                        .buttonStyle(InteractiveStyle(cornerRadius: 6, padding: .init(top: 4, leading: 6, bottom: 4, trailing: 6)))
                                        .foregroundStyle(.green).help("อนุมัติ")
                                }
                                .padding(10)
                                .background(Color.orange.opacity(0.10), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                            }
                        }
                        SectionLabel("จำไว้แล้ว", count: engine.facts.count, tint: .secondary)
                        ForEach(Array(engine.facts.enumerated()), id: \.offset) { _, f in
                            HStack(alignment: .top, spacing: 8) {
                                Circle().fill(Color.accentColor.opacity(0.6)).frame(width: 5, height: 5).padding(.top, 6)
                                Text(f).font(.callout).textSelection(.enabled)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                            }
                            .padding(.horizontal, 8).padding(.vertical, 6)
                            .hoverHighlight(cornerRadius: 8)
                        }
                    }
                    .padding(10)
                }
                .chorusScrollIndicators()
            }
        }
        .frame(maxHeight: .infinity, alignment: .top)
    }

    /// The app's honesty surface. Everything else in this window *describes*
    /// what gets carried between agents; this shows the bytes.
    var contextInspector: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                VStack(alignment: .leading, spacing: 2) {
                    Text("สิ่งที่ agent ตัวถัดไปจะได้เห็น").font(.headline)
                    Text("ประกอบสดจาก ledger ไม่ใช่คำอธิบาย — นี่คือข้อความจริงที่จะถูกส่ง")
                        .font(.caption).foregroundStyle(.secondary)
                }
                Spacer()
                if let n = inspected?.tokens, n > 0 {
                    Text("\(n) token").font(.caption.monospaced()).foregroundStyle(.secondary)
                }
                Button("Done") { inspected = nil }.keyboardShortcut(.defaultAction)
            }
            ScrollView {
                Text(inspected?.text ?? "")
                    .font(.caption.monospaced())
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .padding(16)
        .frame(minWidth: 640, minHeight: 460)
    }

    // MARK: projects — several folders at once, each with its own sessions

    var contextPane: some View {
        VStack(spacing: 0) {
            PaneHeader(title: "คอนเท็กซ์", subtitle: "สิ่งที่ agent ถัดไปจะได้รับ") { EmptyView() }
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    let frac: Double? = {
                        guard let u = engine.contextUsed, let w = engine.contextWindow, w > 0 else { return nil }
                        return min(max(Double(u) / Double(w), 0.0), 1.0)
                    }()
                    // No reading is "ไม่ทราบ", never 0%: a session that has
                    // talked has used context even if the agent never said how much.
                    let hasTurns = engine.events.contains { $0.actor != "user" }
                    let percentText = frac.map { "\(Int($0 * 100))%" } ?? (hasTurns ? "ไม่ทราบ" : "0%")
                    let barColor: Color = {
                        guard let f = frac else { return .accentColor }
                        if f > 0.9 { return .red }
                        if f > 0.75 { return .orange }
                        return .accentColor
                    }()

                    VStack(alignment: .leading, spacing: 8) {
                        HStack {
                            Label("ขนาดหน้าต่างบริบท", systemImage: "cpu")
                                .font(.subheadline.weight(.medium))
                            Spacer()
                            Text(percentText)
                                .font(.subheadline.weight(.semibold).monospacedDigit())
                                .foregroundStyle(barColor)
                        }

                        // Horizontal progress bar (Codex / Claude style)
                        GeometryReader { geo in
                            ZStack(alignment: .leading) {
                                Capsule()
                                    .fill(Color.primary.opacity(0.08))
                                    .frame(height: 7)
                                Capsule()
                                    .fill(barColor)
                                    .frame(width: max(geo.size.width * CGFloat(frac ?? 0), (frac != nil && frac! > 0) ? 6 : 0), height: 7)
                                    .animation(.easeOut(duration: 0.25), value: frac)
                            }
                        }
                        .frame(height: 7)

                        HStack {
                            if let u = engine.contextUsed, let w = engine.contextWindow {
                                Text("\(u.formatted()) / \(w.formatted()) tokens")
                                    .font(.caption.monospacedDigit())
                                    .foregroundStyle(.secondary)
                            } else if let u = engine.contextUsed {
                                Text("\(u.formatted()) tokens · ไม่ทราบขนาดหน้าต่าง")
                                    .font(.caption.monospacedDigit())
                                    .foregroundStyle(.secondary)
                            } else if hasTurns {
                                Text("agent นี้ไม่ได้รายงานจำนวน token")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            } else {
                                Text("ยังไม่มีเทิร์น (ขึ้นหลัง agent ตอบ)")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                            Spacer()
                            if let f = frac, f > 0.75 {
                                Label("ใกล้เต็ม — ส่งต่องานได้", systemImage: "exclamationmark.triangle.fill")
                                    .font(.caption2.weight(.medium))
                                    .foregroundStyle(.orange)
                            }
                        }
                    }
                    .padding(12)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 10, style: .continuous))

                    // 2. 5-hour and Weekly Quota Bars (from limits.rs)
                    if let accountWithLimits = engine.accounts.first(where: { $0.rate_limits?.primary != nil || $0.rate_limits?.secondary != nil }),
                       let rl = accountWithLimits.rate_limits {
                        VStack(alignment: .leading, spacing: 8) {
                            HStack {
                                Label("โควตาการใช้งาน (\(accountWithLimits.agent.isEmpty ? accountWithLimits.name : accountWithLimits.agent))", systemImage: "chart.bar.xaxis")
                                    .font(.caption.weight(.semibold))
                                    .foregroundStyle(.secondary)
                                Spacer()
                            }

                            if let p = rl.primary {
                                let pct = min(max(p.used_percent / 100.0, 0.0), 1.0)
                                VStack(alignment: .leading, spacing: 3) {
                                    HStack {
                                        Text("5 ชั่วโมง")
                                            .font(.caption2)
                                            .foregroundStyle(.secondary)
                                        Spacer()
                                        Text("\(Int(p.used_percent))%")
                                            .font(.caption2.weight(.semibold).monospacedDigit())
                                            .foregroundStyle(p.used_percent >= 90 ? Color.red : (p.used_percent >= 75 ? Color.orange : Color.primary))
                                    }
                                    GeometryReader { g in
                                        ZStack(alignment: .leading) {
                                            Capsule().fill(Color.primary.opacity(0.08)).frame(height: 5)
                                            Capsule().fill(p.used_percent >= 90 ? Color.red : (p.used_percent >= 75 ? Color.orange : Color.accentColor))
                                                .frame(width: max(g.size.width * CGFloat(pct), pct > 0 ? 5 : 0), height: 5)
                                        }
                                    }
                                    .frame(height: 5)
                                    if let reset = p.resets_label {
                                        Text("รีเซ็ตใน \(reset)")
                                            .font(.system(size: 9.5))
                                            .foregroundStyle(.tertiary)
                                    }
                                }
                            }

                            if let s = rl.secondary {
                                let pct = min(max(s.used_percent / 100.0, 0.0), 1.0)
                                VStack(alignment: .leading, spacing: 3) {
                                    HStack {
                                        Text("รายสัปดาห์")
                                            .font(.caption2)
                                            .foregroundStyle(.secondary)
                                        Spacer()
                                        Text("\(Int(s.used_percent))%")
                                            .font(.caption2.weight(.semibold).monospacedDigit())
                                            .foregroundStyle(s.used_percent >= 90 ? Color.red : (s.used_percent >= 75 ? Color.orange : Color.primary))
                                    }
                                    GeometryReader { g in
                                        ZStack(alignment: .leading) {
                                            Capsule().fill(Color.primary.opacity(0.08)).frame(height: 5)
                                            Capsule().fill(s.used_percent >= 90 ? Color.red : (s.used_percent >= 75 ? Color.orange : Color.accentColor))
                                                .frame(width: max(g.size.width * CGFloat(pct), pct > 0 ? 5 : 0), height: 5)
                                        }
                                    }
                                    .frame(height: 5)
                                    if let reset = s.resets_label {
                                        Text("รีเซ็ตใน \(reset)")
                                            .font(.system(size: 9.5))
                                            .foregroundStyle(.tertiary)
                                    }
                                }
                            }
                        }
                        .padding(10)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(Color.primary.opacity(0.03), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                    }

                    // 3. Session Tokens In / Out
                    VStack(alignment: .leading, spacing: 6) {
                        HStack {
                            Label("Token ของ session", systemImage: "arrow.triangle.swap")
                                .font(.caption.weight(.semibold))
                                .foregroundStyle(.secondary)
                            Spacer()
                        }
                        HStack(spacing: 14) {
                            HStack(spacing: 4) {
                                Image(systemName: "arrow.down.right")
                                    .font(.system(size: 10))
                                    .foregroundStyle(.blue)
                                Text("In: \(engine.sessionTokensIn.formatted())")
                                    .font(.caption.monospacedDigit())
                            }
                            HStack(spacing: 4) {
                                Image(systemName: "arrow.up.right")
                                    .font(.system(size: 10))
                                    .foregroundStyle(.purple)
                                Text("Out: \(engine.sessionTokensOut.formatted())")
                                    .font(.caption.monospacedDigit())
                            }
                            Spacer()
                        }
                    }
                    .padding(10)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Color.primary.opacity(0.03), in: RoundedRectangle(cornerRadius: 10, style: .continuous))

                    // 4. Monetary Cost Accounting Card (มูลค่าเทียบ API)
                    VStack(alignment: .leading, spacing: 6) {
                        HStack {
                            Label("มูลค่าเทียบ API", systemImage: "dollarsign.circle.fill")
                                .font(.caption.weight(.semibold))
                                .foregroundStyle(.green)
                            Spacer()
                            Button {
                                if engine.sessionID >= 0 {
                                    engine.updateSessionCost(db: dbPath, id: engine.sessionID)
                                }
                            } label: {
                                Image(systemName: "arrow.clockwise")
                                    .font(.system(size: 10))
                                    .foregroundStyle(.secondary)
                            }
                            .buttonStyle(.chorusPlain)
                            .help("รีเฟรชค่าใช้จ่าย")
                        }
                        if let cost = engine.sessionCostUSD {
                            HStack(alignment: .firstTextBaseline, spacing: 6) {
                                Text(String(format: "$%.4f USD", cost))
                                    .font(.system(.body, design: .monospaced).weight(.bold))
                                    .foregroundStyle(cost > 0 ? Color.primary : Color.secondary)

                                if let rate = Double(thbRate.trimmingCharacters(in: .whitespaces)), rate > 0 {
                                    Text(String(format: "(≈ ฿%.2f)", cost * rate))
                                        .font(.caption.monospacedDigit())
                                        .foregroundStyle(.secondary)
                                } else {
                                    Button("ตั้งอัตราบาท") {
                                        settingsSection = .cost
                                        showSettings = true
                                    }
                                    .font(.caption)
                                    .buttonStyle(.link)
                                }
                                Spacer()
                            }
                            Text("รายงานจาก usage_update.cost ของ bridge")
                                .font(.caption2)
                                .foregroundStyle(.tertiary)
                        } else {
                            HStack {
                                Text("ไม่มีข้อมูลราคา")
                                    .font(.system(.body).weight(.medium))
                                    .foregroundStyle(.secondary)
                                Spacer()
                            }
                            Text("โมเดล/bridge นี้ไม่รายงานราคา API (เช่น บัญชี subscription หรือ Codex)")
                                .font(.caption2)
                                .foregroundStyle(.tertiary)
                        }
                    }
                    .padding(10)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Color.green.opacity(0.06), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                    .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).stroke(Color.green.opacity(0.18), lineWidth: 1))

                    // Compiler & Linter Verification Health Card (Autonomous Self-Healing Loop)
                    VStack(alignment: .leading, spacing: 8) {
                        HStack {
                            if engine.isVerifying {
                                ProgressView()
                                    .controlSize(.mini)
                                Text("กำลังตรวจสอบโค้ด...")
                                    .font(.caption.weight(.semibold))
                                    .foregroundStyle(.secondary)
                            } else if let v = engine.lastVerification {
                                Image(systemName: v.passed ? "checkmark.seal.fill" : "exclamationmark.triangle.fill")
                                    .foregroundStyle(v.passed ? .green : .red)
                                    .font(.caption)
                                Text(v.passed ? "คอมไพล์ผ่าน" : "พบข้อผิดพลาด \(v.diagnostics.count) จุด")
                                    .font(.caption.weight(.semibold))
                                    .foregroundStyle(v.passed ? .green : .red)
                            } else {
                                Image(systemName: "wrench.and.screwdriver")
                                    .foregroundStyle(.secondary)
                                    .font(.caption)
                                Text("การตรวจสอบโค้ดอัตโนมัติ")
                                    .font(.caption.weight(.semibold))
                                    .foregroundStyle(.secondary)
                            }
                            Spacer()
                            Button {
                                engine.verifyWorkspace(ws: workspace)
                            } label: {
                                Image(systemName: "arrow.clockwise")
                                    .font(.system(size: 10))
                                    .foregroundStyle(.secondary)
                            }
                            .buttonStyle(.chorusPlain)
                            .disabled(engine.isVerifying || workspace.isEmpty || !engine.projectTrusted)
                            .help("รัน cargo check, biome, ruff หรือ swiftlint ในโปรเจกต์ทันที (ใน sandbox)")
                        }

                        Toggle(isOn: Binding(
                            get: { engine.projectTrusted },
                            set: { engine.setProjectTrusted(ws: workspace, $0) }
                        )) {
                            Text("เชื่อถือโปรเจกต์นี้ — รันตรวจโค้ด/hooks หลังทุกเทิร์น")
                                .font(.caption2)
                        }
                        .toggleStyle(.switch)
                        .controlSize(.mini)
                        .disabled(workspace.isEmpty)
                        .help("build.rs, hooks และ linter คือโค้ดของโปรเจกต์ เปิดเฉพาะโปรเจกต์ที่ไว้ใจ — ถึงเปิดก็รันใน sandbox เดียวกับเทิร์น")

                        if let v = engine.lastVerification, !v.passed {
                            VStack(alignment: .leading, spacing: 4) {
                                ForEach(v.diagnostics.prefix(3)) { d in
                                    HStack(alignment: .top, spacing: 4) {
                                        Text("[\(d.tool)]")
                                            .font(.caption2.monospaced())
                                            .foregroundStyle(.red)
                                        Text("\((d.file as NSString).lastPathComponent):\(d.line) \(d.message)")
                                            .font(.caption2)
                                            .lineLimit(1)
                                            .truncationMode(.tail)
                                    }
                                }
                                if v.diagnostics.count > 3 {
                                    let rem = v.diagnostics.count - 3
                                    Text("...และอีก \(rem) รายการ")
                                        .font(.caption2)
                                        .foregroundStyle(.secondary)
                                }
                            }
                            .padding(6)
                            .background(Color.red.opacity(0.08), in: RoundedRectangle(cornerRadius: 6))

                            // One-click Self-Heal button
                            Button {
                                var prompt = "กรุณาแก้ข้อผิดพลาดจากการคอมไพล์/ตรวจโค้ดต่อไปนี้ให้ผ่าน:\n\n"
                                for d in v.diagnostics {
                                    prompt += "- [\(d.tool)] \(d.file):\(d.line) - \(d.message)\n"
                                }
                                draftBox.text = prompt
                                submit()
                            } label: {
                                HStack(spacing: 4) {
                                    Image(systemName: "bolt.fill")
                                    Text("ให้ agent แก้เลย")
                                }
                                .font(.caption.weight(.semibold))
                                .frame(maxWidth: .infinity)
                            }
                            .buttonStyle(InteractiveStyle(cornerRadius: 6))
                            .tint(.red)
                            .disabled(engine.busy)
                        } else {
                            Text("รันคอมไพเลอร์อัตโนมัติหลัง agent ตอบ เพื่อป้องกันโค้ดพัง")
                                .font(.caption2)
                                .foregroundStyle(.tertiary)
                        }
                    }
                    .padding(10)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 10, style: .continuous))

                    ContextInspectorSection(engine: engine) { text, tokens in inspected = (text, tokens) }

                    Button {
                        engine.assembledContext { text, tokens in inspected = (text, tokens) }
                    } label: {
                        Label("ดูสิ่งที่ agent เห็นจริง ๆ", systemImage: "doc.text.magnifyingglass")
                            .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(InteractiveStyle(cornerRadius: 8))
                    .background(Color.primary.opacity(0.05), in: RoundedRectangle(cornerRadius: 8))
                    .disabled(engine.sessionID < 0)

                    // High-Density Repo Map (Personalized PageRank)
                    Button {
                        isGeneratingRepoMap = true
                        engine.generateRepoMap(workspace: workspace) { map, count, _ in
                            isGeneratingRepoMap = false
                            if let map = map {
                                inspectedRepoMap = (map, count ?? 0)
                                showingRepoMapSheet = true
                            }
                        }
                    } label: {
                        HStack {
                            Label("High-Density Repo Map", systemImage: "point.3.filled.connected.trianglepath.dotted")
                            Spacer()
                            if isGeneratingRepoMap {
                                ProgressView().controlSize(.mini)
                            }
                        }
                        .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(InteractiveStyle(cornerRadius: 8))
                    .background(Color.primary.opacity(0.05), in: RoundedRectangle(cornerRadius: 8))
                    .disabled(workspace.isEmpty || isGeneratingRepoMap)
                    .sheet(isPresented: $showingRepoMapSheet) {
                        if let rm = inspectedRepoMap {
                            VStack(alignment: .leading, spacing: 10) {
                                HStack {
                                    Label("Repo Map (\(rm.count) Symbols Ranked)", systemImage: "point.3.filled.connected.trianglepath.dotted")
                                        .font(.headline)
                                    Spacer()
                                    Button("ปิด") { showingRepoMapSheet = false }
                                        .buttonStyle(.borderedProminent)
                                        .controlSize(.small)
                                }
                                Divider()
                                ScrollView {
                                    Text(rm.text)
                                        .font(.caption.monospaced())
                                        .textSelection(.enabled)
                                        .frame(maxWidth: .infinity, alignment: .leading)
                                }
                            }
                            .padding(16)
                            .frame(minWidth: 540, minHeight: 400)
                        }
                    }

                    // The tier the design never trims. Showing it is also the
                    // cheapest way to notice if it ever goes empty again.
                    if !engine.taskState.isEmpty {
                        SectionLabel("สถานะงาน", count: nil, tint: .secondary)
                        Text(engine.taskState)
                            .font(.caption.monospaced())
                            .textSelection(.enabled)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(10)
                            .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                    }
                }
                .padding(10)
            }
            .chorusScrollIndicators()
            // The warm tier catches up after every turn; this keeps a backlog
            // visible rather than mysterious.
            HStack(spacing: 6) {
                Label("\(engine.events.count) events", systemImage: "list.bullet")
                if engine.uncompacted > 0 {
                    Text("·")
                    Text("รอสรุป \(engine.uncompacted)")
                    Spacer()
                    Button("สรุปตอนนี้") { engine.compactNow(limit: 500) }
                        .buttonStyle(InteractiveStyle(cornerRadius: 6, padding: .init(top: 2, leading: 6, bottom: 2, trailing: 6)))
                        .disabled(engine.busy)
                } else {
                    Spacer()
                }
            }
            .font(.caption)
            .foregroundStyle(.secondary)
            .padding(.horizontal, 12).padding(.vertical, 8)
        }
        .frame(maxHeight: .infinity, alignment: .top)
    }
}
