import SwiftUI

/// Codex reports one model id per reasoning level (`gpt-5.6-luna[high]`), so a
/// flat list is 27 rows of the same five models. Split it back into the two
/// choices people actually make — which model, how hard it thinks — the way the
/// Claude and Codex apps put a model chip and an effort chip next to send.
struct ModelID: Hashable {
    let base: String
    let effort: String?

    init(_ id: String) {
        if let open = id.firstIndex(of: "["), id.hasSuffix("]") {
            base = String(id[..<open])
            effort = String(id[id.index(after: open)..<id.index(before: id.endIndex)])
        } else {
            base = id
            effort = nil
        }
    }

    static func compose(_ base: String, _ effort: String?) -> String {
        guard let effort, !effort.isEmpty else { return base }
        return "\(base)[\(effort)]"
    }
}

extension Engine.AgentInfo {
    /// Distinct base models, in the order the agent reported them.
    var baseModels: [(id: String, label: String)] {
        var seen = Set<String>()
        var out: [(String, String)] = []
        for m in models {
            let b = ModelID(m.id).base
            guard seen.insert(b).inserted else { continue }
            // "5.6 Luna (high)" → "5.6 Luna"
            let label = m.label.replacingOccurrences(of: #"\s*\([^)]*\)$"#, with: "", options: .regularExpression)
            out.append((b, label))
        }
        return out
    }

    func efforts(for base: String) -> [String] {
        models.compactMap { m in
            let id = ModelID(m.id)
            return id.base == base ? id.effort : nil
        }
    }
}

/// The two chips under the composer: model (agent + model) and effort.
struct ModelPicker: View {
    @EnvironmentObject var engine: Engine
    @Binding var model: String
    var onNeedsLogin: () -> Void
    @State private var open = false

    private var agent: Engine.AgentInfo? {
        engine.agents.first { $0.models.contains(where: { $0.id == model }) || $0.id == model }
    }
    private var current: ModelID { ModelID(model) }

    var body: some View {
        HStack(spacing: 4) {
            Button { open.toggle() } label: {
                HStack(spacing: 5) {
                    Circle().fill(agent?.ready == true ? Color.green : .orange).frame(width: 6, height: 6)
                    Text(agent?.label ?? "เลือกโมเดล").fontWeight(.medium)
                    if let a = agent, let l = a.baseModels.first(where: { $0.id == current.base })?.label {
                        Text(l).foregroundStyle(.secondary)
                    }
                    Image(systemName: "chevron.up.chevron.down").font(.system(size: 8))
                }
                .font(.caption)
                .padding(.horizontal, 8).padding(.vertical, 4)
                .background(.quaternary.opacity(0.5), in: Capsule())
            }
            .buttonStyle(.plain)
            .keyboardShortcut("i", modifiers: [.command, .shift])
            .help("เลือกเอเจนต์และโมเดล (⌘⇧I)")
            .popover(isPresented: $open, arrowEdge: .top) { popover }

            if let a = agent {
                let efforts = a.efforts(for: current.base)
                if !efforts.isEmpty {
                    Menu {
                        ForEach(efforts, id: \.self) { e in
                            Button { model = ModelID.compose(current.base, e) } label: {
                                if e == current.effort { Label(e, systemImage: "checkmark") } else { Text(e) }
                            }
                        }
                    } label: {
                        Label(current.effort ?? "effort", systemImage: "brain")
                            .font(.caption)
                    }
                    .menuStyle(.borderlessButton).fixedSize()
                    .help("ระดับการคิด (⌘⇧E สลับไประดับถัดไป)")
                    // Menus can't be opened from a shortcut, so ⌘⇧E steps
                    // through the levels instead.
                    Button("") { cycleEffort(efforts) }
                        .keyboardShortcut("e", modifiers: [.command, .shift])
                        .frame(width: 0, height: 0).opacity(0)
                }
            }
        }
    }

    private func cycleEffort(_ efforts: [String]) {
        guard !efforts.isEmpty else { return }
        let i = efforts.firstIndex(of: current.effort ?? "") ?? -1
        model = ModelID.compose(current.base, efforts[(i + 1) % efforts.count])
    }

    private var popover: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(engine.agents) { a in
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 6) {
                        Circle().fill(a.ready ? Color.green : .orange).frame(width: 7, height: 7)
                        Text(a.label).font(.caption.bold())
                        Spacer()
                        if !a.ready {
                            Button("login") { open = false; onNeedsLogin() }
                                .buttonStyle(.link).font(.caption2)
                        }
                    }
                    .padding(.horizontal, 10).padding(.top, 8).padding(.bottom, 2)
                    ForEach(a.baseModels, id: \.id) { m in
                        let selected = agent?.id == a.id && current.base == m.id
                        Button {
                            // Keep the effort the user had if the new model
                            // offers it, else the agent's default for that model.
                            let efforts = a.efforts(for: m.id)
                            let keep = efforts.contains(current.effort ?? "") ? current.effort : nil
                            let def = a.models.first { ModelID($0.id).base == m.id && $0.is_default }
                                .flatMap { ModelID($0.id).effort }
                            model = ModelID.compose(m.id, keep ?? def ?? efforts.first)
                            open = false
                        } label: {
                            HStack {
                                Text(m.label)
                                Spacer()
                                if selected { Image(systemName: "checkmark").font(.caption) }
                            }
                            .font(.callout)
                            .padding(.horizontal, 10).padding(.vertical, 5)
                            .background(selected ? Color.accentColor.opacity(0.15) : .clear,
                                        in: RoundedRectangle(cornerRadius: 5))
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .disabled(!a.ready)
                        .padding(.horizontal, 4)
                    }
                }
                Divider().padding(.top, 6)
            }
            Text("ถ้าตัวนี้ชนลิมิต Chorus ส่งงานต่อให้ตัวที่พร้อมเอง")
                .font(.caption2).foregroundStyle(.secondary)
                .padding(10)
        }
        .frame(width: 280)
        .padding(.vertical, 4)
    }
}
