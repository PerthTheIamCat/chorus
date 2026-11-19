import AppKit
import SwiftUI
import UniformTypeIdentifiers
import UserNotifications

struct CompareResultsSheet: View {
    let json: String
    @Environment(\.dismiss) private var dismiss

    struct Branch: Codable, Identifiable {
        let agent: String
        let session_id: Int64
        let message: String
        let stop_reason: String
        let error: String?
        // Absent in payloads from an older engine.
        var diff: String? = nil
        var elapsed_ms: Int64? = nil
        var context_used: Int64? = nil
        var worktree: String? = nil
        var worktree_branch: String? = nil
        var cost_usd: Double? = nil
        var score: Score? = nil
        var id: Int64 { session_id }
    }
    /// One check of a scored lane; `passed` is nil when it did not run.
    struct Check: Codable { var name: String? = nil; var ran: Bool? = nil; var passed: Bool? = nil; var note: String? = nil }
    struct Score: Codable {
        var rank: Int? = nil
        var reason: String? = nil
        var mode: String? = nil
        var passed: Bool? = nil
        var verify: Check? = nil
        var tests: Check? = nil
        var discriminating: Check? = nil
        var rewinds: Int? = nil
    }
    struct Verdict: Codable { var judge: String? = nil; var judged: Bool? = nil; var summary: String? = nil; var note: String? = nil }
    struct Payload: Codable { let ok: Bool?; let branches: [Branch]? ; let error: String?; var verdict: Verdict? = nil }

    private var payload: Payload? {
        guard let d = json.data(using: .utf8) else { return nil }
        return try? JSONDecoder().decode(Payload.self, from: d)
    }

    /// "1m 05s" / "12.3s".
    static func duration(_ ms: Int64) -> String {
        let s = Double(ms) / 1000
        return s < 60 ? String(format: "%.1fs", s) : String(format: "%dm %02ds", Int(s) / 60, Int(s) % 60)
    }

    static func compactCount(_ n: Int64) -> String {
        n >= 1000 ? String(format: "%.1fk", Double(n) / 1000) : "\(n)"
    }

    /// Agents answer in Markdown (file links included), so render it; fall
    /// back to the raw text if it does not parse.
    static func markdown(_ text: String) -> AttributedString {
        let opts = AttributedString.MarkdownParsingOptions(interpretedSyntax: .inlineOnlyPreservingWhitespace)
        return (try? AttributedString(markdown: text, options: opts)) ?? AttributedString(text)
    }

    /// Stats line for one column: time · context tokens · cost.
    static func stats(_ b: Branch) -> String {
        var parts: [String] = []
        if let ms = b.elapsed_ms, ms > 0 { parts.append(duration(ms)) }
        if let ctx = b.context_used, ctx > 0 { parts.append("\(compactCount(ctx)) tokens") }
        if let c = b.cost_usd { parts.append(String(format: "$%.4f", c)) }
        return parts.joined(separator: " · ")
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Compare").font(.title3.bold())
            if let err = payload?.error {
                Text(err).foregroundStyle(.red)
            }
            if let v = payload?.verdict {
                let who = (v.judge ?? "").isEmpty ? "" : "ผู้ตัดสิน \(v.judge ?? "")"
                let said = [who, v.summary ?? "", v.note ?? ""].filter { !$0.isEmpty }.joined(separator: " · ")
                if !said.isEmpty {
                    Text(said).font(.callout).foregroundStyle(.secondary)
                }
            }
            ScrollView([.horizontal, .vertical]) {
                HStack(alignment: .top, spacing: 12) {
                    ForEach(payload?.branches ?? []) { b in column(b) }
                }
            }
            HStack { Spacer(); Button("Done") { dismiss() }.keyboardShortcut(.defaultAction) }
        }
        .padding(20)
        .frame(minWidth: 620, minHeight: 360)
    }

    /// Rank, the checks that ran and why — evidence the human weighs; the
    /// choice stays theirs.
    @ViewBuilder private func scoreBadge(_ sc: Score) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack(spacing: 6) {
                if let r = sc.rank, r > 0 { Text("#\(r)").font(.caption.weight(.bold)) }
                checkChip("build", sc.verify)
                checkChip("tests", sc.tests)
                if sc.discriminating?.ran == true { checkChip("เทสต์แยก", sc.discriminating) }
            }
            if let why = sc.reason, !why.isEmpty {
                Text(why).font(.caption2).foregroundStyle(.secondary)
            }
        }
    }

    @ViewBuilder private func checkChip(_ name: String, _ c: Check?) -> some View {
        let ran = c?.ran ?? false
        let ok = c?.passed
        Label(name, systemImage: !ran ? "minus.circle" : (ok == true ? "checkmark.circle.fill" : "xmark.circle.fill"))
            .font(.caption2)
            .foregroundStyle(!ran ? Color.secondary : (ok == true ? Color.green : Color.red))
            .help(ran ? (c?.note ?? "") : (c?.note ?? "ไม่ได้รัน"))
    }

    @ViewBuilder private func column(_ b: Branch) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(b.agent).font(.headline)
            if let sc = b.score { scoreBadge(sc) }
            let stats = Self.stats(b)
            if !stats.isEmpty {
                Text(stats).font(.caption.monospacedDigit()).foregroundStyle(.secondary)
            }
            Text("resume: --session \(b.session_id)")
                .font(.caption2).foregroundStyle(.secondary)
            if let wt = b.worktree {
                Button {
                    NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: wt)])
                } label: {
                    Label("สำเนาแยกของ agent นี้ — เปิดใน Finder", systemImage: "folder").font(.caption2)
                }
                .buttonStyle(.link)
                .help("\(wt)\nbranch \(b.worktree_branch ?? "")")
            }
            Divider()
            if let e = b.error {
                Text(e).font(.caption).foregroundStyle(.red).textSelection(.enabled)
            } else {
                Text(Self.markdown(b.message)).font(.callout).textSelection(.enabled)
                Text("[\(b.stop_reason)]").font(.caption2).foregroundStyle(.tertiary)
            }
            if let diff = b.diff, !diff.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                let hunks = parseHunks(diff)
                let files = Set(hunks.map(\.file)).count
                let adds = hunks.reduce(0) { $0 + $1.addCount }, dels = hunks.reduce(0) { $0 + $1.delCount }
                DisclosureGroup {
                    ScrollView(.horizontal) {
                        VStack(alignment: .leading, spacing: 0) {
                            ForEach(Array(diff.components(separatedBy: "\n").enumerated()), id: \.offset) { _, line in
                                Text(line.isEmpty ? " " : line)
                                    .font(.system(size: 10, design: .monospaced))
                                    .foregroundStyle(line.hasPrefix("+") && !line.hasPrefix("+++") ? Color.green
                                        : line.hasPrefix("-") && !line.hasPrefix("---") ? Color.red : .secondary)
                            }
                        }
                        .textSelection(.enabled)
                    }
                    .frame(maxHeight: 220)
                } label: {
                    Label("\(files) ไฟล์ · +\(adds) −\(dels)", systemImage: "doc.badge.gearshape").font(.caption.weight(.medium))
                }
            } else if b.error == nil, b.diff != nil {
                Text("ไม่ได้แก้ไฟล์").font(.caption2).foregroundStyle(.tertiary)
            }
        }
        .frame(width: 280, alignment: .leading)
        .padding(10)
        .background(.quaternary.opacity(0.3), in: RoundedRectangle(cornerRadius: 8))
    }
}

/// A minimal unified-diff model: each hunk carries the file preamble it
/// needs to be a valid standalone patch.
struct DiffHunk: Identifiable {
    /// Stable across re-parses: the diff is re-parsed on every render, and a
    /// fresh UUID each time silently dropped every keep/reject the user set.
    var id: String { file + "|" + header }
    let file: String
    let preamble: String   // "diff --git … / --- … / +++ …"
    let header: String     // "@@ -a,b +c,d @@ …"
    let lines: [String]    // body lines (context / +/-)

    /// Counted once at parse time: views read these on every render.
    let addCount: Int
    let delCount: Int

    init(file: String, preamble: String, header: String, lines: [String]) {
        self.file = file
        self.preamble = preamble
        self.header = header
        self.lines = lines
        addCount = lines.reduce(0) { $0 + ($1.hasPrefix("+") ? 1 : 0) }
        delCount = lines.reduce(0) { $0 + ($1.hasPrefix("-") ? 1 : 0) }
    }

    var patch: String { preamble + header + "\n" + lines.joined(separator: "\n") + "\n" }

    /// Compute line number within the file for a given line index in the hunk (1-based).
    func fileLineNumber(for lineIndex: Int) -> Int {
        let parts = header.components(separatedBy: "@@")
        guard parts.count >= 2 else { return lineIndex + 1 }
        let spec = parts[1].trimmingCharacters(in: .whitespaces)
        guard let plusRange = spec.range(of: "+") else { return lineIndex + 1 }
        let afterPlus = spec[plusRange.upperBound...]
        let countPart = afterPlus.split(separator: " ").first?.split(separator: ",").first
        guard let startLine = countPart.flatMap({ Int($0) }) else { return lineIndex + 1 }

        var currentNewLine = startLine
        for i in 0..<min(lineIndex, lines.count) {
            let l = lines[i]
            if !l.hasPrefix("-") {
                currentNewLine += 1
            }
        }
        return currentNewLine
    }
}

func parseHunks(_ diff: String) -> [DiffHunk] {
    var hunks: [DiffHunk] = []
    var file = ""
    var preamble = ""
    var inPreamble = false
    var curHeader: String?
    var curLines: [String] = []

    func flush() {
        if let h = curHeader {
            hunks.append(DiffHunk(file: file, preamble: preamble, header: h, lines: curLines))
        }
        curHeader = nil
        curLines = []
    }

    for line in diff.split(separator: "\n", omittingEmptySubsequences: false).map(String.init) {
        if line.hasPrefix("diff --git") {
            flush()
            preamble = line + "\n"
            inPreamble = true
            if let r = line.range(of: " b/") { file = String(line[r.upperBound...]) }
        } else if inPreamble && (line.hasPrefix("index ") || line.hasPrefix("--- ") || line.hasPrefix("+++ ")
                                 || line.hasPrefix("new file") || line.hasPrefix("deleted file")
                                 || line.hasPrefix("similarity") || line.hasPrefix("rename ")) {
            preamble += line + "\n"
        } else if line.hasPrefix("@@") {
            flush()
            inPreamble = false
            curHeader = line
        } else if curHeader != nil {
            curLines.append(line)
        }
    }
    flush()
    return hunks
}

struct HunkView: View {
    let hunk: DiffHunk
    let rejected: Bool
    let toggle: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text(hunk.header.components(separatedBy: "@@").dropFirst().first?.trimmingCharacters(in: .whitespaces) ?? "")
                    .font(.caption2.monospaced()).foregroundStyle(.tertiary).lineLimit(1)
                Spacer()
                Button(action: toggle) {
                    Label(rejected ? "ตีกลับ" : "เก็บ", systemImage: rejected ? "xmark" : "checkmark")
                        .font(.caption2.weight(.semibold))
                        .foregroundStyle(rejected ? .red : .green)
                }
                .buttonStyle(.plain)
                .help(rejected ? "กดอีกครั้งเพื่อเก็บไว้" : "กดเพื่อเลือกตีกลับก้อนนี้")
            }
            VStack(alignment: .leading, spacing: 0) {
                ForEach(Array(hunk.lines.prefix(60).enumerated()), id: \.offset) { _, l in
                    Text(l.isEmpty ? " " : l).font(.system(size: 10.5, design: .monospaced))
                        .foregroundStyle(l.hasPrefix("+") ? Color.green : l.hasPrefix("-") ? .red : .secondary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, 6)
                        .background(l.hasPrefix("+") ? Color.green.opacity(0.08)
                                    : l.hasPrefix("-") ? Color.red.opacity(0.08) : .clear)
                }
                if hunk.lines.count > 60 {
                    Text("… อีก \(hunk.lines.count - 60) บรรทัด").font(.caption2).foregroundStyle(.tertiary).padding(6)
                }
            }
            .padding(.vertical, 4)
            .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 6))
            .opacity(rejected ? 0.4 : 1)
            .strikethrough(rejected)
        }
        .padding(.horizontal, 4)
    }
}

/// Just enough of the handoff brief to render the moment it describes.
struct HandoffBrief {
    let goal: String
    let done: [String]
    let pending: [String]
    let files: [String]
    let diffBytes: Int
    let from: String
    let to: String
    let reason: String

    init?(json: String) {
        guard let data = json.data(using: .utf8),
              let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return nil }
        let meta = obj["handoff"] as? [String: Any] ?? [:]
        goal = obj["goal"] as? String ?? ""
        done = obj["done"] as? [String] ?? []
        pending = obj["pending"] as? [String] ?? []
        files = obj["files"] as? [String] ?? []
        diffBytes = (obj["diff"] as? String)?.utf8.count ?? 0
        from = meta["from"] as? String ?? "?"
        to = meta["to"] as? String ?? "?"
        reason = (meta["reason"] as? String ?? "handoff").replacingOccurrences(of: "_", with: " ")
    }
}
