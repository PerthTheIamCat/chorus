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

