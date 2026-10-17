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
            if let e = ModelID(m.id).effort {
                let suffixLower = "(\(e.lowercased()))"
                let suffixCap = "(\(e.capitalized))"
                if label.hasSuffix(suffixCap) {
                    label = String(label.dropLast(suffixCap.count)).trimmingCharacters(in: .whitespaces)
                } else if label.hasSuffix(suffixLower) {
                    label = String(label.dropLast(suffixLower.count)).trimmingCharacters(in: .whitespaces)
                }
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

extension Engine {
    /// Every base model the agents report, from the `chorus models` cache
    /// (CLAUDE.md 7.19) — never a hardcoded list. `keeping` is a stored pick
    /// that is no longer reported; it stays in the list so a Picker bound to it
    /// does not render blank (7.14).
    func knownBaseModels(keeping stored: String = "") -> [(id: String, label: String)] {
        var seen = Set<String>()
        var out: [(id: String, label: String)] = []
        for a in agents {
            for m in a.baseModels where seen.insert(m.id).inserted {
                out.append((m.id, "\(a.label) · \(m.label)"))
            }
        }
        if !stored.isEmpty, !seen.contains(stored) { out.append((stored, stored)) }
        return out
    }
}

/// The two chips under the composer: model (agent + model) and effort.
struct ModelPicker: View {
    @EnvironmentObject var engine: Engine
    @Environment(\.compactControls) private var compact
    @Binding var model: String
    var onNeedsLogin: () -> Void
    /// Only the composer's picker answers ⇧⌘I / ⇧⌘E from the Chat menu; a
    /// second picker in a sheet would otherwise react to the same keystroke.
    var respondsToMenu = true
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
                    if !compact || agent == nil {
                        Text(agent?.label ?? "เลือกโมเดล").fontWeight(.medium).lineLimit(1).fixedSize()
                    }
                    if !compact, let a = agent, let l = a.baseModels.first(where: { $0.id == current.base })?.label {
                        Text(l).foregroundStyle(.secondary).lineLimit(1).fixedSize()
                    } else if !compact, agent == nil, !model.isEmpty {
                        Text(current.base).foregroundStyle(.secondary).lineLimit(1).fixedSize()
                    }
                    Image(systemName: "chevron.up.chevron.down").font(.system(size: 8))
                }
                .font(.caption)
            }
            .buttonStyle(InteractiveStyle(cornerRadius: 99, selected: open,
                                          padding: .init(top: 4, leading: 9, bottom: 4, trailing: 9)))
            .help(compact ? "\(agent?.label ?? "เลือกโมเดล") · \(current.base) — เลือก agent และโมเดล (⌘⇧I)" : "เลือก agent และโมเดล (⌘⇧I)")
            .popover(isPresented: $open, arrowEdge: .top) { popover }

            if let a = agent {
                let efforts = a.efforts(for: current.base)
                if !efforts.isEmpty {
                    ChipPicker(icon: "brain", value: current.effort ?? "effort", options: efforts,
                               selected: current.effort) { model = ModelID.compose(current.base, $0) }
                        .help("ระดับการคิด (⌘⇧E สลับไประดับถัดไป)")
                }
            }
        }
        .onReceive(AppCommandCenter.shared.fire) { command in
            guard respondsToMenu else { return }
            switch command {
            case .chooseModel: open = true
            case .nextEffort:
                if let a = agent { cycleEffort(a.efforts(for: current.base)) }
            default: break
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
                .help("ถามรายการโมเดลล่าสุดจากแต่ละ agent")
            }
            .padding(6)
        }
        .frame(width: 240)
    }
}

/// The composer row ran out of width: pills drop their text and keep the
/// icon (the label moves to the tooltip). Set by `ComposerField`.
private struct CompactControlsKey: EnvironmentKey {
    static let defaultValue = false
}

extension EnvironmentValues {
    var compactControls: Bool {
        get { self[CompactControlsKey.self] }
        set { self[CompactControlsKey.self] = newValue }
    }
}

/// Icon + one-line text that drops to the icon alone in a compact row.
struct CompactLabel<Icon: View>: View {
    @Environment(\.compactControls) private var compact
    let text: String
    var spacing: CGFloat = 4
    @ViewBuilder var icon: () -> Icon

    var body: some View {
        HStack(spacing: spacing) {
            icon()
            if !compact { Text(text).lineLimit(1).fixedSize() }
        }
    }
}

/// A capsule chip that opens a small list of options — the effort and
/// permission choices. A popover (not a `Menu`) so it gets the same
/// hover/press/selected treatment as everything else.
struct ChipPicker: View {
    @Environment(\.compactControls) private var compact
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
                if !compact { Text(labels[value] ?? value).lineLimit(1).fixedSize() }
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
