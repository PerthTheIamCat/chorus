import SwiftUI
import AppKit

/// High-fidelity Diff Viewer and Inline Review Feedback Loop (Ticket 15).
/// Features Unified & Split views with Rust spacer alignment, gutter hover comments,
/// and one-click steering loop ("Send to Agent ⌘↩").
struct DiffReviewerView: View {
    @ObservedObject var engine: Engine
    var workspace: String = ""
    var isMaximized: Bool = false
    var onToggleMaximize: (() -> Void)? = nil
    var onSendSteeringPrompt: (String) -> Void

    @State private var viewMode: ViewMode = .unified
    @State private var selectedFile: String = ""
    @State private var pendingComments: [Engine.ReviewComment] = []
    @State private var rejectedHunkIDs: Set<String> = []
    @State private var showAllDiff: Bool = false
    /// (hunk index in `shown`, 1-based line in that hunk).
    @State private var commentingAt: RowKey? = nil
    /// What the list draws: the selected file's hunks, flattened into one
    /// row per header and per line. Rebuilt only when the diff, the file
    /// filter or the mode changes — never on a plain re-render.
    @State private var shown: [DiffHunk] = []
    @State private var rows: [Row] = []
    @State private var commentText: String = ""
    @State private var structuralAnalysis: Engine.StructuralDiffAnalysis? = nil
    @State private var isAnalyzingStructural: Bool = false
    @State private var verificationResult: Engine.VerificationResult? = nil
    @State private var isVerifying: Bool = false
    @State private var showVerificationSheet: Bool = false
    @State private var showShipSheet: Bool = false

    enum ViewMode: String, CaseIterable {
        case unified = "Unified"
        case split = "Split"
    }

    enum DiffScope: String, CaseIterable {
        case latestTurn = "Latest Turn"
        case currentAgent = "This Agent"
        case session = "Full Session"
        case childWorktree = "Child Worktree"

        var scopeKey: String {
            switch self {
            case .latestTurn: return "latest_turn"
            case .currentAgent: return "current_agent"
            case .session: return "session"
            case .childWorktree: return "worktree"
            }
        }

        init(scopeKey: String) {
            switch scopeKey {
            case "latest_turn": self = .latestTurn
            case "current_agent": self = .currentAgent
            case "worktree": self = .childWorktree
            default: self = .session
            }
        }
    }

    struct RowKey: Hashable {
        let hunk: Int
        let line: Int
    }

    /// `line == nil` is the hunk's header.
    struct Row: Identifiable {
        let id: Int
        let hunk: Int
        let line: Int?
    }

    private var allHunks: [DiffHunk] { engine.diffHunks }

    private var filesWithDiffs: [String] { engine.diffFiles }

    private func rebuildRows() {
        let hunks = selectedFile.isEmpty ? allHunks : allHunks.filter { $0.file == selectedFile }
        var out: [Row] = []
        out.reserveCapacity(hunks.reduce(hunks.count) { $0 + $1.lines.count })
        for (hi, h) in hunks.enumerated() {
            out.append(Row(id: out.count, hunk: hi, line: nil))
            for li in h.lines.indices {
                out.append(Row(id: out.count, hunk: hi, line: li))
            }
        }
        shown = hunks
        rows = out
        commentingAt = nil
    }

    var body: some View {
        content
            .onReceive(AppCommandCenter.shared.fire) { command in
                if command == .sendReview, !pendingComments.isEmpty { sendToAgent() }
            }
    }

    private var content: some View {
        VStack(spacing: 0) {
            // Header Controls
            VStack(spacing: 6) {
                // Row 1: Mode + Scope + Maximize
                HStack(spacing: 8) {
                    Picker("", selection: $viewMode) {
                        ForEach(ViewMode.allCases, id: \.self) { mode in
                            Text(mode.rawValue).tag(mode)
                        }
                    }
                    .pickerStyle(.segmented)
                    .frame(maxWidth: 160)

                    Picker("", selection: Binding(
                        get: { DiffScope(scopeKey: engine.diffScope) },
                        set: { engine.diffScope = $0.scopeKey }
                    )) {
                        ForEach(DiffScope.allCases, id: \.self) { sc in
                            Text(sc.rawValue).tag(sc)
                        }
                    }
                    .pickerStyle(.menu)
                    .frame(maxWidth: 130)

                    Spacer()

                    if let onToggleMaximize {
                        PanelMaximizeButton(isMaximized: isMaximized, onToggle: onToggleMaximize)
                    }
                }

                // Row 2: Diff Stats Badge + Mark Reviewed + Chorus Files + AST Analysis + Verification Loop
                HStack(spacing: 8) {
                    let totalAdd = allHunks.reduce(0) { $0 + $1.addCount }
                    let totalDel = allHunks.reduce(0) { $0 + $1.delCount }
                    HStack(spacing: 4) {
                        Text("+\(totalAdd)")
                            .font(.system(size: 10, weight: .bold, design: .monospaced))
                            .foregroundColor(.green)
                        Text("-\(totalDel)")
                            .font(.system(size: 10, weight: .bold, design: .monospaced))
                            .foregroundColor(.red)
                    }
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(Color.primary.opacity(0.04))
                    .cornerRadius(4)

                    Button {
                        engine.markReviewed()
                    } label: {
                        HStack(spacing: 3) {
                            Image(systemName: "checkmark.seal")
                                .font(.system(size: 10))
                            Text("ตรวจแล้ว")
                                .font(.system(size: 9.5))
                        }
                    }
                    .buttonStyle(.bordered)
                    .controlSize(.mini)
                    .help("บันทึกสถานะปัจจุบันเป็น baseline ใหม่สำหรับการรีวิว (ไม่แตะ git ของคุณ)")

                    Toggle(isOn: $engine.showChorusWrittenFiles) {
                        Image(systemName: "doc.badge.gearshape")
                            .font(.system(size: 10))
                    }
                    .toggleStyle(.button)
                    .controlSize(.mini)
                    .help(engine.showChorusWrittenFiles ? "ซ่อนไฟล์ Chorus (CLAUDE.md, AGENTS.md ฯลฯ)" : "แสดงไฟล์ Chorus (CLAUDE.md, AGENTS.md ฯลฯ)")

                    Spacer()

                    // AST Structural Diff Analysis Button
                    Button {
                        runStructuralDiff()
                    } label: {
                        HStack(spacing: 3) {
                            Image(systemName: "wand.and.stars")
                                .font(.system(size: 10))
                            if isAnalyzingStructural {
                                ProgressView().controlSize(.mini)
                            } else if let sa = structuralAnalysis {
                                Text(sa.is_purely_cosmetic ? "Cosmetic" : "Sem:\(sa.semantic_lines)")
                                    .font(.system(size: 9.5, weight: .semibold))
                            } else {
                                Text("AST")
                                    .font(.system(size: 9.5))
                            }
                        }
                    }
                    .buttonStyle(.bordered)
                    .controlSize(.mini)
                    .help("แยกการแก้โค้ดจริงออกจากการจัดรูปแบบ (อ่านจาก AST)")

                    // Verification Loop Button
                    Button {
                        runVerification()
                    } label: {
                        HStack(spacing: 3) {
                            Image(systemName: verificationResult == nil ? "checkmark.shield" : (verificationResult?.passed == true ? "checkmark.shield.fill" : "exclamationmark.triangle.fill"))
                                .font(.system(size: 10))
                                .foregroundColor(verificationResult == nil ? .secondary : (verificationResult?.passed == true ? .green : .orange))
                            if isVerifying {
                                ProgressView().controlSize(.mini)
                            } else if let v = verificationResult {
                                Text(v.passed ? "Pass" : "\(v.findings.count) err")
                                    .font(.system(size: 9.5, weight: .semibold))
                            } else {
                                Text("Verify")
                                    .font(.system(size: 9.5))
                            }
                        }
                    }
                    .buttonStyle(.bordered)
                    .controlSize(.mini)
                    .help("ตรวจความถูกต้องของ workspace ด้วย Fast Compiler/Linter Loop")
                    .popover(isPresented: $showVerificationSheet) {
                        VerificationFindingsPopover(result: verificationResult)
                    }

                    Button {
                        showShipSheet = true
                    } label: {
                        Label("ส่งขึ้น Git", systemImage: "arrow.up.circle").font(.system(size: 9.5))
                    }
                    .buttonStyle(.bordered)
                    .controlSize(.mini)
                    .disabled(workspace.isEmpty)
                    .help("commit → push → เปิด PR ในที่เดียว")
                    .sheet(isPresented: $showShipSheet) {
                        GitShipSheet(
                            workspace: workspace,
                            sessionFiles: engine.diffFiles,
                            goal: engine.events.last(where: { $0.actor == "user" && $0.kind == "msg" })?.body ?? ""
                        ) { showShipSheet = false }
                    }
                }

                // File Selector Bar
                let files = filesWithDiffs
                if files.count > 1 {
                    HStack(spacing: 6) {
                        Image(systemName: "doc.text")
                            .font(.system(size: 10))
                            .foregroundColor(.secondary)

                        Picker("", selection: $selectedFile) {
                            Text("ทุกไฟล์ที่แก้ (\(files.count) ไฟล์)").tag("")
                            ForEach(files, id: \.self) { f in
                                Text((f as NSString).lastPathComponent).tag(f)
                            }
                        }
                        .pickerStyle(.menu)
                        .font(.system(size: 11))

                        Spacer()
                    }
                }
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 8)
            .background(Color(NSColor.controlBackgroundColor))

            Divider()

            // Diff Content Area
            if !rows.isEmpty {
                if !showAllDiff && (filesWithDiffs.count > 300 || rows.count > 5000) {
                    VStack(spacing: 12) {
                        Spacer()
                        Image(systemName: "doc.text.magnifyingglass")
                            .font(.system(size: 28))
                            .foregroundColor(.secondary)
                        Text("Diff มีขนาดใหญ่มาก (\(filesWithDiffs.count) ไฟล์, \(rows.count) บรรทัด)")
                            .font(.system(size: 13, weight: .semibold))
                        Text("เพื่อความลื่นไหล จึงย่อการแสดงผลไว้ คุณสามารถเลือกดูทีละไฟล์จากเมนูด้านบน หรือกดแสดงทั้งหมด")
                            .font(.system(size: 11))
                            .foregroundColor(.secondary)
                            .multilineTextAlignment(.center)
                            .frame(maxWidth: 360)
                        Button("แสดงทั้งหมด (\(rows.count) บรรทัด)") {
                            showAllDiff = true
                        }
                        .buttonStyle(.borderedProminent)
                        .controlSize(.small)
                        Spacer()
                    }
                    .padding(20)
                } else {
                    ScrollView([.vertical, .horizontal]) {
                        // One lazy row per header/line: a hunk with thousands of
                        // lines no longer builds every line to show the first few.
                        LazyVStack(alignment: .leading, spacing: 0) {
                            ForEach(rows) { row in
                                rowView(row)
                            }
                        }
                        .padding(8)
                    }
                    .chorusScrollIndicators()
                }
            } else {
                VStack(spacing: 8) {
                    Spacer()
                    Image(systemName: "checkmark.circle")
                        .font(.system(size: 24))
                        .foregroundColor(.secondary)
                    Text("ยังไม่มีการแก้ไขไฟล์ใน session นี้")
                        .font(.system(size: 12))
                        .foregroundColor(.secondary)
                    Spacer()
                }
            }

            // Floating Review Action Bar (Bottom)
            if !pendingComments.isEmpty || !rejectedHunkIDs.isEmpty {
                Divider()
                HStack(spacing: 10) {
                    if !rejectedHunkIDs.isEmpty {
                        Button(action: {
                            let patches = allHunks.filter { rejectedHunkIDs.contains($0.id) }.map { $0.patch }.joined()
                            if !patches.isEmpty {
                                engine.rejectHunks(patches)
                                rejectedHunkIDs.removeAll()
                            }
                        }) {
                            HStack(spacing: 4) {
                                Image(systemName: "arrow.uturn.backward")
                                    .font(.system(size: 10))
                                Text("ตีกลับ \(rejectedHunkIDs.count) ก้อนที่เลือก")
                                    .font(.system(size: 11, weight: .semibold))
                            }
                            .foregroundColor(.white)
                            .padding(.horizontal, 10)
                            .padding(.vertical, 5)
                            .background(Color.red)
                            .cornerRadius(5)
                        }
                        .buttonStyle(.plain)

                        Button("ยกเลิก") {
                            rejectedHunkIDs.removeAll()
                        }
                        .font(.system(size: 11))
                    }

                    if !pendingComments.isEmpty {
                        Image(systemName: "bubble.left.and.exclamationmark.bubble.right.fill")
                            .foregroundColor(.orange)
                            .font(.system(size: 12))

                        Text("\(pendingComments.count) คอมเมนต์รีวิวที่ค้างอยู่")
                            .font(.system(size: 11, weight: .medium))

                        Spacer()

                        Button("ล้าง") {
                            engine.clearReviewComments()
                            refreshComments()
                        }
                        .font(.system(size: 11))

                        Button(action: sendToAgent) {
                            HStack(spacing: 4) {
                                Image(systemName: "paperplane.fill")
                                    .font(.system(size: 10))
                                Text("Send to Agent (⇧⌘↩)")
                                    .font(.system(size: 11, weight: .bold))
                            }
                            .foregroundColor(.white)
                            .padding(.horizontal, 10)
                            .padding(.vertical, 5)
                            .background(Color.blue)
                            .cornerRadius(5)
                        }
                        .buttonStyle(.plain)
                    } else {
                        Spacer()
                    }
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(Color(NSColor.windowBackgroundColor))
            }
        }
        .onAppear {
            engine.checkUserCommit()
            refreshComments()
            if let first = filesWithDiffs.first {
                selectedFile = first
            }
            rebuildRows()
        }
        .onChange(of: engine.workingDiff) { _, _ in
            if !selectedFile.isEmpty, !filesWithDiffs.contains(selectedFile) { selectedFile = "" }
            rebuildRows()
        }
        .onChange(of: selectedFile) { _, _ in rebuildRows() }
    }

    @ViewBuilder
    private func rowView(_ row: Row) -> some View {
        let hunk = shown[row.hunk]
        if let li = row.line {
            let fileLine = hunk.fileLineNumber(for: li)
            let key = RowKey(hunk: row.hunk, line: fileLine)
            Group {
                if viewMode == .unified {
                    UnifiedLineRow(line: hunk.lines[li], lineNum: fileLine,
                                   onAddComment: { _ in commentingAt = key })
                } else {
                    SplitLineRow(line: hunk.lines[li], lineNum: fileLine,
                                 onAddComment: { _ in commentingAt = key })
                }
            }
            .background(Color.primary.opacity(0.02))
            if commentingAt == key {
                InlineCommentEditor(
                    line: key.line,
                    commentText: $commentText,
                    onSave: { saveComment(line: key.line, file: hunk.file) },
                    onCancel: {
                        commentingAt = nil
                        commentText = ""
                    }
                )
            }
        } else {
            HStack {
                Text((hunk.file as NSString).lastPathComponent)
                    .font(.system(size: 10.5, weight: .bold, design: .monospaced))
                    .foregroundColor(.primary)
                Text(hunk.header)
                    .font(.system(size: 9.5, design: .monospaced))
                    .foregroundColor(.secondary)
                Spacer()

                let isRejected = rejectedHunkIDs.contains(hunk.id)
                Button(action: {
                    if isRejected {
                        rejectedHunkIDs.remove(hunk.id)
                    } else {
                        rejectedHunkIDs.insert(hunk.id)
                    }
                }) {
                    HStack(spacing: 3) {
                        Image(systemName: isRejected ? "arrow.uturn.backward.circle.fill" : "hand.thumbsdown")
                            .font(.system(size: 10))
                        Text(isRejected ? "จะตีกลับ" : "ตีกลับ")
                            .font(.system(size: 10))
                    }
                    .foregroundColor(isRejected ? .red : .secondary)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(isRejected ? Color.red.opacity(0.12) : Color.clear)
                    .cornerRadius(4)
                }
                .buttonStyle(.plain)
                .help("เลือกก้อนนี้เพื่อตีกลับ (Reverse apply)")
            }
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .background(Color.primary.opacity(0.05))
            .padding(.top, row.hunk == 0 ? 0 : 10)
        }
    }

    private func refreshComments() {
        engine.fetchPendingReviewComments { comments in
            self.pendingComments = comments
        }
    }

    private func saveComment(line: Int, file: String) {
        guard !commentText.trimmingCharacters(in: .whitespaces).isEmpty else { return }
        engine.addReviewComment(
            path: file.isEmpty ? "workspace" : file,
            line: line,
            comment: commentText,
            snippet: nil
        )
        commentingAt = nil
        commentText = ""
        refreshComments()
    }

    private func sendToAgent() {
        engine.fetchReviewSteeringPrompt { prompt in
            if !prompt.isEmpty {
                onSendSteeringPrompt(prompt)
                engine.clearReviewComments()
                refreshComments()
            }
        }
    }

    private func runStructuralDiff() {
        guard !engine.workingDiff.isEmpty else { return }
        isAnalyzingStructural = true
        var oldLines: [String] = []
        var newLines: [String] = []
        for line in engine.workingDiff.components(separatedBy: "\n") {
            if line.hasPrefix("-") && !line.hasPrefix("---") {
                oldLines.append(String(line.dropFirst()))
            } else if line.hasPrefix("+") && !line.hasPrefix("+++") {
                newLines.append(String(line.dropFirst()))
            } else if !line.hasPrefix("@@") && !line.hasPrefix("diff ") && !line.hasPrefix("index ") {
                oldLines.append(line)
                newLines.append(line)
            }
        }
        let oldCode = oldLines.joined(separator: "\n")
        let newCode = newLines.joined(separator: "\n")
        engine.analyzeStructuralDiff(oldCode: oldCode, newCode: newCode) { analysis, _ in
            self.structuralAnalysis = analysis
            self.isAnalyzingStructural = false
        }
    }

    private func runVerification() {
        let ws = workspace.isEmpty ? NSHomeDirectory() : workspace
        isVerifying = true
        engine.verifyWorkspace(workspace: ws) { result, _ in
            self.verificationResult = result
            self.isVerifying = false
            self.showVerificationSheet = true
        }
    }
}

struct VerificationFindingsPopover: View {
    let result: Engine.VerificationResult?

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Label(result?.passed == true ? "Verification Passed" : "Findings Detected",
                      systemImage: result?.passed == true ? "checkmark.circle.fill" : "exclamationmark.triangle.fill")
                    .font(.headline)
                    .foregroundColor(result?.passed == true ? .green : .orange)
                Spacer()
                if let tool = result?.tool_used {
                    Text(tool)
                        .font(.caption.monospaced())
                        .foregroundColor(.secondary)
                }
            }
            Divider()
            if let findings = result?.findings, !findings.isEmpty {
                ScrollView {
                    VStack(alignment: .leading, spacing: 6) {
                        ForEach(findings) { f in
                            VStack(alignment: .leading, spacing: 2) {
                                HStack(spacing: 4) {
                                    Text(f.file)
                                        .font(.caption.weight(.bold))
                                    if f.line > 0 {
                                        Text(":\(f.line)")
                                            .font(.caption.monospaced())
                                            .foregroundColor(.secondary)
                                    }
                                    Spacer()
                                    Text(f.severity.uppercased())
                                        .font(.system(size: 9, weight: .bold))
                                        .padding(.horizontal, 4)
                                        .padding(.vertical, 1)
                                        .background(f.severity == "error" ? Color.red.opacity(0.15) : Color.orange.opacity(0.15))
                                        .foregroundColor(f.severity == "error" ? .red : .orange)
                                        .cornerRadius(3)
                                }
                                Text(f.message)
                                    .font(.system(size: 11))
                                    .foregroundColor(.primary)
                            }
                            .padding(6)
                            .background(Color.primary.opacity(0.03))
                            .cornerRadius(6)
                        }
                    }
                }
                .frame(maxHeight: 220)
            } else {
                Text("Zero lint or compiler diagnostics. Clean workspace.")
                    .font(.caption)
                    .foregroundColor(.secondary)
            }
        }
        .padding(12)
        .frame(width: 340)
    }
}

private struct UnifiedLineRow: View {
    let line: String
    let lineNum: Int
    var onAddComment: (Int) -> Void

    @State private var isHovered = false

    private var isAdd: Bool { line.hasPrefix("+") }
    private var isDel: Bool { line.hasPrefix("-") }

    var body: some View {
        HStack(spacing: 6) {
            Text("\(lineNum)")
                .font(.system(size: 10, design: .monospaced))
                .foregroundColor(.secondary.opacity(0.5))
                .frame(width: 28, alignment: .trailing)

            Text(isAdd ? "+" : (isDel ? "-" : " "))
                .font(.system(size: 11, weight: .bold, design: .monospaced))
                .foregroundColor(isAdd ? .green : (isDel ? .red : .clear))
                .frame(width: 10)

            Text(line.isEmpty ? " " : (line.hasPrefix("+") || line.hasPrefix("-") ? String(line.dropFirst()) : line))
                .font(.system(size: 11, design: .monospaced))
                .foregroundColor(isAdd ? .green : (isDel ? .red : .primary))
                .lineLimit(1)
                .fixedSize(horizontal: true, vertical: false)

            Spacer(minLength: 8)

            if isHovered {
                Button(action: { onAddComment(lineNum) }) {
                    Image(systemName: "plus.bubble.fill")
                        .font(.system(size: 10))
                        .foregroundColor(.blue)
                }
                .buttonStyle(.plain)
                .padding(.trailing, 4)
            }
        }
        .padding(.vertical, 1)
        .padding(.horizontal, 4)
        .background(isAdd ? Color.green.opacity(0.08) : (isDel ? Color.red.opacity(0.08) : Color.clear))
        .onHover { isHovered = $0 }
    }
}

private struct SplitLineRow: View {
    let line: String
    let lineNum: Int
    var onAddComment: (Int) -> Void

    @State private var isHovered = false

    private var isAdd: Bool { line.hasPrefix("+") }
    private var isDel: Bool { line.hasPrefix("-") }

    var body: some View {
        HStack(spacing: 0) {
            // Left (Original / Deleted)
            HStack(spacing: 6) {
                Text(isDel ? "\(lineNum)" : " ")
                    .font(.system(size: 10, design: .monospaced))
                    .foregroundColor(.secondary.opacity(0.5))
                    .frame(width: 28, alignment: .trailing)

                Text(isDel ? String(line.dropFirst()) : (!isAdd ? line : " "))
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundColor(isDel ? .red : .secondary)
                    .lineLimit(1)
                    .fixedSize(horizontal: true, vertical: false)

                Spacer(minLength: 8)
            }
            .frame(minWidth: 280, alignment: .leading)
            .background(isDel ? Color.red.opacity(0.08) : Color.clear)

            Divider()

            // Right (Added / Current)
            HStack(spacing: 6) {
                Text(isAdd ? "\(lineNum)" : " ")
                    .font(.system(size: 10, design: .monospaced))
                    .foregroundColor(.secondary.opacity(0.5))
                    .frame(width: 28, alignment: .trailing)

                Text(isAdd ? String(line.dropFirst()) : (!isDel ? line : " "))
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundColor(isAdd ? .green : .primary)
                    .lineLimit(1)
                    .fixedSize(horizontal: true, vertical: false)

                Spacer(minLength: 8)

                if isHovered && (isAdd || !isDel) {
                    Button(action: { onAddComment(lineNum) }) {
                        Image(systemName: "plus.bubble.fill")
                            .font(.system(size: 10))
                            .foregroundColor(.blue)
                    }
                    .buttonStyle(.plain)
                    .padding(.trailing, 4)
                }
            }
            .frame(minWidth: 280, alignment: .leading)
            .background(isAdd ? Color.green.opacity(0.08) : Color.clear)
        }
        .onHover { isHovered = $0 }
    }
}

private struct InlineCommentEditor: View {
    let line: Int
    @Binding var commentText: String
    var onSave: () -> Void
    var onCancel: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text("คอมเมนต์รีวิวที่บรรทัด \(line)")
                    .font(.system(size: 10, weight: .bold))
                    .foregroundColor(.secondary)
                Spacer()
            }

            TextEditor(text: $commentText)
                .font(.system(size: 11))
                .frame(height: 48)
                .padding(4)
                .background(Color(NSColor.textBackgroundColor))
                .cornerRadius(4)

            HStack {
                Spacer()
                Button("ยกเลิก", action: onCancel)
                    .font(.system(size: 11))
                Button("บันทึกคอมเมนต์", action: onSave)
                    .font(.system(size: 11, weight: .bold))
                    .buttonStyle(.borderedProminent)
            }
        }
        .padding(8)
        .background(Color.orange.opacity(0.08))
        .cornerRadius(6)
        .padding(.horizontal, 10)
        .padding(.vertical, 4)
    }
}
