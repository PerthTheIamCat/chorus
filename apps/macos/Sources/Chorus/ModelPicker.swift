import SwiftUI

/// Codex reports one model id per reasoning level (`gpt-5.6-luna[high]`), so a
/// flat list is 27 rows of the same five models. Split it back into the two
/// choices people actually make — which model, how hard it thinks — the way the
/// Claude and Codex apps put a model chip and an effort chip next to send.
struct ModelID: Hashable {
    let base: String
    let effort: String?

    /// Only these are reasoning levels. Claude's `opus[1m]` uses the same
    /// brackets for context size — that is a different model, not an effort.
    static let efforts: Set<String> = ["minimal", "low", "medium", "high", "xhigh", "max", "ultra"]

    init(_ id: String) {
        if let open = id.firstIndex(of: "["), id.hasSuffix("]"),
           Self.efforts.contains(String(id[id.index(after: open)..<id.index(before: id.endIndex)])) {
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
            // "5.6 Luna (high)" → "5.6 Luna", but keep "Opus (1M context)":
            // only an effort suffix is redundant once effort has its own chip.
            var label = m.label
            if let e = ModelID(m.id).effort, label.hasSuffix("(\(e))") {
                label = String(label.dropLast(e.count + 2)).trimmingCharacters(in: .whitespaces)
            }
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
                    if let agent { ProviderIcon(provider: agent.id, size: 14) }
                    Text(agent?.label ?? "เลือกโมเดล").fontWeight(.medium)
                    if let a = agent, let l = a.baseModels.first(where: { $0.id == current.base })?.label {
                        Text(l).foregroundStyle(.secondary)
                    } else if agent == nil, !model.isEmpty {
                        Text(current.base).foregroundStyle(.secondary)
                    }
                    Image(systemName: "chevron.up.chevron.down").font(.system(size: 8))
                }
                .font(.caption)
            }
            .buttonStyle(InteractiveStyle(cornerRadius: 99, selected: open,
                                          padding: .init(top: 4, leading: 9, bottom: 4, trailing: 9)))
            .keyboardShortcut("i", modifiers: [.command, .shift])
            .help("เลือกเอเจนต์และโมเดล (⌘⇧I)")
            .popover(isPresented: $open, arrowEdge: .top) { popover }

            if let a = agent {
                let efforts = a.efforts(for: current.base)
                if !efforts.isEmpty {
                    ChipPicker(icon: "brain", value: current.effort ?? "effort", options: efforts,
                               selected: current.effort) { model = ModelID.compose(current.base, $0) }
                        .help("ระดับการคิด (⌘⇧E สลับไประดับถัดไป)")
                    // Popovers can't be opened from a shortcut here, so ⌘⇧E
                    // steps through the levels instead.
                    Button("") { cycleEffort(efforts) }
                        .keyboardShortcut("e", modifiers: [.command, .shift])
                        .frame(width: 0, height: 0).opacity(0).allowsHitTesting(false)
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
            ScrollView {
                VStack(alignment: .leading, spacing: 2) {
                    ForEach(engine.agents) { a in
                        HStack(spacing: 6) {
                            ProviderIcon(provider: a.id, size: 14)
                            Text(a.label).font(.caption.bold()).foregroundStyle(.secondary)
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
                                    Text(m.label).font(.callout.weight(selected ? .semibold : .regular))
                                    Spacer()
                                    if selected { Image(systemName: "checkmark").font(.caption.bold()) }
                                }
                                .frame(maxWidth: .infinity, alignment: .leading)
                            }
                            .buttonStyle(InteractiveStyle(cornerRadius: 7, selected: selected))
                            .disabled(!a.ready)
                        }
                    }
                }
                .padding(6)
            }
            .frame(maxHeight: 460)
            Divider()
            HStack {
                Spacer()
                Button { engine.refreshModels(force: true) } label: {
                    if engine.refreshingModels { ProgressView().controlSize(.mini) }
                    else { Image(systemName: "arrow.clockwise") }
                }
                .buttonStyle(InteractiveStyle(cornerRadius: 6, padding: .init(top: 3, leading: 6, bottom: 3, trailing: 6)))
                .help("ถามรายการโมเดลล่าสุดจากแต่ละเอเจนต์")
            }
            .padding(6)
        }
        .frame(width: 240)
    }
}

/// A capsule chip that opens a small list of options — the effort and
/// permission choices. A popover (not a `Menu`) so it gets the same
/// hover/press/selected treatment as everything else.
struct ChipPicker: View {
    let icon: String
    let value: String
    let options: [String]
    var labels: [String: String] = [:]
    let selected: String?
    let pick: (String) -> Void
    @State private var open = false

    var body: some View {
        Button { open.toggle() } label: {
            HStack(spacing: 4) {
                Image(systemName: icon)
                Text(labels[value] ?? value)
                Image(systemName: "chevron.down").font(.system(size: 7, weight: .semibold))
            }
            .font(.caption)
        }
        .buttonStyle(InteractiveStyle(cornerRadius: 99, selected: open,
                                      padding: .init(top: 4, leading: 9, bottom: 4, trailing: 9)))
        .popover(isPresented: $open, arrowEdge: .top) {
            VStack(alignment: .leading, spacing: 2) {
                ForEach(options, id: \.self) { o in
                    Button { pick(o); open = false } label: {
                        HStack {
                            Text(labels[o] ?? o)
                            Spacer()
                            if o == selected { Image(systemName: "checkmark").font(.caption.bold()) }
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .buttonStyle(InteractiveStyle(cornerRadius: 7, selected: o == selected))
                }
            }
            .padding(6)
            .frame(width: 180)
        }
    }
}
