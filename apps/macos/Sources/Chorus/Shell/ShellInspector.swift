import SwiftUI

// The inspector of the v0.4 window: a native `.inspector` column with three
// tabs. It is closed on launch; a dot on its toolbar button says something is
// waiting inside (changed files, memory proposals).

struct ShellInspector: View {
    @EnvironmentObject private var shell: ShellModel

    var body: some View {
        VStack(spacing: 0) {
            Picker("แผง", selection: $shell.inspectorTab) {
                ForEach(InspectorTab.allCases) { t in
                    Label(t.title, systemImage: t.symbol).tag(t)
                }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .padding(10)
            Divider()
            switch shell.inspectorTab {
            case .context: ContextTab()
            case .changes: ChangesTab()
            case .memory: MemoryTab()
            }
        }
    }
}

// MARK: changes

private struct ChangesTab: View {
    @EnvironmentObject private var engine: Engine
    @State private var confirmRevert = false

    var body: some View {
        if engine.diffHunks.isEmpty {
            ContentUnavailableView("ยังไม่มีไฟล์ที่แก้", systemImage: "doc.badge.ellipsis",
                                   description: Text("เมื่อ agent แก้ไฟล์ จะเห็นตรงนี้ทีละส่วน"))
        } else {
            VStack(spacing: 0) {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 12) {
                        ForEach(engine.diffFiles, id: \.self) { file in
                            FileDiff(file: file,
                                     hunks: engine.diffHunks.filter { $0.file == file },
                                     isTest: engine.testFiles.contains(file))
                        }
                    }
                    .padding(12)
                }
                Divider()
                HStack {
                    Text("\(engine.diffFiles.count) ไฟล์").font(.caption).foregroundStyle(.secondary)
                    Spacer()
                    Button("ย้อนทั้งหมด", role: .destructive) { confirmRevert = true }
                        .controlSize(.small)
                        .help("ย้อนโฟลเดอร์กลับไปตอนเริ่ม session")
                }
                .padding(10)
            }
            .confirmationDialog("ย้อนไฟล์ทั้งหมดกลับไปตอนเริ่ม session?", isPresented: $confirmRevert) {
                Button("ย้อนทั้งหมด", role: .destructive) { engine.revertToStart() }
                Button("ยกเลิก", role: .cancel) {}
            } message: {
                Text("ย้อนกลับไม่ได้ ระบบจะเก็บ snapshot ก่อนย้อนไว้ให้")
            }
        }
    }
}

private struct FileDiff: View {
    @EnvironmentObject private var engine: Engine
    let file: String
    let hunks: [DiffHunk]
    let isTest: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 6) {
                Text(file).font(.callout.monospaced()).lineLimit(1).truncationMode(.head)
                if isTest {
                    Text("แก้เทสต์").font(.caption2).foregroundStyle(.orange)
                        .padding(.horizontal, 5).padding(.vertical, 1)
                        .background(Color.orange.opacity(0.15), in: Capsule())
                }
                Spacer()
                Text("+\(hunks.reduce(0) { $0 + $1.addCount })").foregroundStyle(.green)
                Text("−\(hunks.reduce(0) { $0 + $1.delCount })").foregroundStyle(.red)
            }
            .font(.caption.monospacedDigit())
            ForEach(hunks) { h in
                VStack(alignment: .leading, spacing: 0) {
                    HStack {
                        Text(h.header).font(.caption2.monospaced()).foregroundStyle(.secondary).lineLimit(1)
                        Spacer()
                        Button("ย้อนส่วนนี้") { engine.rejectHunks(h.patch) }
                            .buttonStyle(.link).font(.caption)
                    }
                    .padding(.horizontal, 8).padding(.vertical, 4)
                    Divider()
                    VStack(alignment: .leading, spacing: 0) {
                        ForEach(Array(h.lines.enumerated()), id: \.offset) { _, l in
                            Text(l.isEmpty ? " " : l)
                                .font(.system(size: 11, design: .monospaced))
                                .padding(.horizontal, 8)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .background(tint(l))
                        }
                    }
                    .textSelection(.enabled)
                }
                .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
            }
        }
    }

    private func tint(_ line: String) -> Color {
        if line.hasPrefix("+") { return Color.green.opacity(0.14) }
        if line.hasPrefix("-") { return Color.red.opacity(0.14) }
        return Color.clear
    }
}

// MARK: memory

private struct MemoryTab: View {
    @EnvironmentObject private var engine: Engine

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                if let scope = engine.memoryScope {
                    Text("ความจำของโปรเจกต์ \(scope)").font(.caption).foregroundStyle(.secondary)
                }
                if !engine.pending.isEmpty {
                    Text("รออนุมัติ \(engine.pending.count)").font(.callout.weight(.semibold))
                    ForEach(engine.pending) { p in ProposalCard(p: p) }
                }
                Text("ที่จำไว้ \(engine.facts.count)").font(.callout.weight(.semibold))
                if engine.facts.isEmpty {
                    Text("ยังไม่มี agent จะเสนอสิ่งที่ควรจำเมื่อเจอ").font(.caption).foregroundStyle(.secondary)
                }
                ForEach(Array(engine.facts.enumerated()), id: \.offset) { _, f in
                    InspectorCard { Text(f).font(.callout).textSelection(.enabled) }
                }
            }
            .padding(12)
        }
        .onAppear { engine.refreshMemory() }
    }
}

private struct ProposalCard: View {
    @EnvironmentObject private var engine: Engine
    let p: Engine.Proposal

    var body: some View {
        InspectorCard {
            Text(p.text).font(.callout).textSelection(.enabled)
            if p.replaces != nil {
                Text("แทนข้อเดิม").font(.caption).foregroundStyle(.orange)
            }
            if p.suspicious == true {
                Label("มาจากของภายนอก ตรวจก่อนรับ", systemImage: "exclamationmark.shield")
                    .font(.caption).foregroundStyle(.orange)
            }
            if let e = p.evidence, !e.isEmpty {
                Text(e).font(.caption).foregroundStyle(.secondary).lineLimit(3)
            }
            HStack {
                Spacer()
                Button("ไม่") { engine.review(p.id, approve: false) }
                Button(p.kind == "retire" ? "เลิกจำ" : "จำไว้") { engine.review(p.id, approve: true) }
                    .buttonStyle(.borderedProminent)
            }
            .controlSize(.small)
        }
    }
}
