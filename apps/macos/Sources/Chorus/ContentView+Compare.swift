import AppKit
import SwiftUI
import UniformTypeIdentifiers
import UserNotifications

// ContentView, split out of ContentView.swift: compare flow and command palette.
// Moved verbatim; only `private` was dropped so the pieces can see each other.

extension ContentView {
    /// A session exists, or the first message would start one (a fresh draft).
    var canCompare: Bool { engine.sessionID >= 0 || engine.draft != nil }

    /// What a compare would ask: the text typed in the composer, otherwise
    /// the last prompt.
    var comparePrompt: (text: String, fromComposer: Bool) {
        let typed = draftBox.text.trimmingCharacters(in: .whitespacesAndNewlines)
        if !typed.isEmpty { return (typed, true) }
        return (lastPrompt.isEmpty ? "ทบทวนงานที่ค้างอยู่แล้วเสนอขั้นต่อไป" : lastPrompt, false)
    }

    /// Entry point for the button, ⇧⌘K and /compare: it fans one prompt out to
    /// every ready agent (each in its own fork, costing tokens), so ask first.
    func runCompare() {
        guard engine.agents.filter(\.ready).count > 1, canCompare, !comparing else { return }
        confirmCompare = true
    }

    var compareConfirmSheet: some View {
        let agents = engine.agents.filter(\.ready)
        let source = comparePrompt
        return VStack(alignment: .leading, spacing: 14) {
            Label("เทียบ agent", systemImage: "square.split.2x1").font(.title3.bold())
            Text("จะส่งคำถามเดียวกันให้ทุก agent ที่พร้อม แยกกันทำใน fork ของตัวเอง แล้วเทียบคำตอบ ไฟล์ที่แก้ เวลา และค่าใช้จ่าย")
                .font(.callout).foregroundStyle(.secondary)
            VStack(alignment: .leading, spacing: 4) {
                Text(source.fromComposer ? "คำถาม (จากช่องพิมพ์ — จะถูกล้างหลังส่ง)" : "คำถาม (ข้อความล่าสุดที่ส่ง)")
                    .font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                Text(source.text).lineLimit(5).font(.callout)
                    .padding(8).frame(maxWidth: .infinity, alignment: .leading)
                    .background(Color.primary.opacity(0.06), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            }
            VStack(alignment: .leading, spacing: 4) {
                Text("agent ที่จะถูกเรียก (\(agents.count))").font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                Text(agents.map(\.label).joined(separator: " · ")).font(.callout)
            }
            Text("แต่ละตัวใช้โควตา/โทเคนของตัวเองหนึ่งเทิร์น").font(.caption).foregroundStyle(.secondary)
            Toggle("ตัดสินด้วยหลักฐาน — รันเช็ค+เทสต์ในแต่ละสำเนา แล้วให้อีกเจ้าจัดอันดับ", isOn: $compareJudge)
                .help("ใช้เวลารันเทสต์และโควตาของผู้ตัดสินอีกหนึ่งเทิร์น · เปิดได้เมื่อโปรเจกต์ trust แล้ว · คุณเลือกผู้ชนะเองเสมอ")
            Toggle("เทียบ agent เดี่ยว กับ orchestra ของ agent เดียวกัน", isOn: $compareOrchestra)
            HStack {
                Button("ยกเลิก") { confirmCompare = false }.keyboardShortcut(.cancelAction)
                Spacer()
                Button("เทียบเลย") { confirmCompare = false; performCompare() }
                    .buttonStyle(.borderedProminent).keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 420)
    }

    /// Compares what is typed in the composer (clearing it, like send); with
    /// nothing typed, the last prompt. In a fresh session the draft becomes a
    /// real session first, the same path the first message takes.
    func performCompare() {
        let others = engine.agents.filter(\.ready).map(\.id)
        guard others.count > 1, canCompare, !comparing else { return }
        let source = comparePrompt
        let prompt = source.text
        if source.fromComposer {
            lastPrompt = prompt
            draftBox.text = ""
            composerFocus &+= 1
        }
        let level = max(permission.rawValue, 0)
        var lanes = others
        if compareOrchestra, let first = others.first { lanes.append(first + "+orchestra") }
        let agents = lanes.joined(separator: ",")
        let options: String? = compareJudge ? "{\"judge\":true}" : nil
        let start = {
            comparing = true
            engine.compare(prompt, agents: agents, level: level, options: options) { comparing = false; compareOutput = $0 }
        }
        if engine.sessionID < 0 && engine.draft != nil {
            engine.commitDraft { applyModel(model); start() }
        } else {
            start()
        }
    }

    // MARK: command palette (⌘K)

    struct PaletteItem: Identifiable {
        let id = UUID()
        let title: String
        let icon: String
        let run: () -> Void
    }

    var paletteItems: [PaletteItem] {
        var items: [PaletteItem] = []
        for a in engine.agents where a.ready {
            items.append(.init(title: "สลับไปใช้ \(a.label)", icon: "person.crop.circle") { model = a.defaultModel })
            if engine.sessionID >= 0 && a.id != currentAgentID {
                items.append(.init(title: "ส่งต่องานให้ \(a.label)", icon: "arrowshape.turn.up.right.fill") {
                    engine.handoffAuto(to: a.id, level: max(permission.rawValue, 0))
                })
            }
        }
        items.append(.init(title: "เลือกโฟลเดอร์โปรเจกต์…", icon: "folder") { pickFolder() })
        if engine.sessionID >= 0 {
            items.append(.init(title: "Snapshot ตอนนี้", icon: "camera") { engine.snapshot() })
        }
        items.append(.init(title: "ตั้งค่า", icon: "gearshape") { showSettings = true })
        return items
    }

    var palette: some View {
        PaletteSheet(items: paletteItems.map { ($0.title, $0.icon, $0.run) }) { showPalette = false }
    }

    // MARK: context pane — budget + last agent
}
