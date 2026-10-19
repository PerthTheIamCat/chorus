import SwiftUI

/// Cross-session Full Text Search Sheet (Ticket 04 & 21, triggered via ⌘F).
/// Fast sub-millisecond search across historical sessions in SQLite, plus
/// the sessions Claude Code / Codex / Gemini CLI left on this machine, which
/// import into the ledger and continue here.
struct ChatSearchView: View {
    enum Scope: String, CaseIterable, Identifiable {
        case chorus = "ใน Chorus"
        case external = "จากแอปอื่น"
        var id: String { rawValue }
    }

    @ObservedObject var engine: Engine
    let dbPath: String
    var onSelectSession: (Int64) -> Void
    var onClose: () -> Void

    @State private var query = ""
    @State private var hits: [Engine.SearchHit] = []
    @State private var isSearching = false
    @State private var scope: Scope = .chorus
    @State private var external: [Engine.ExternalSession] = []
    @State private var externalError: String?
    @State private var importing: String?
    @State private var importNote: String?

    var body: some View {
        VStack(spacing: 0) {
            // Header Search Input
            HStack(spacing: 10) {
                Image(systemName: "magnifyingglass")
                    .foregroundColor(.secondary)
                    .font(.system(size: 15))

                TextField(scope == .chorus ? "Search all sessions, messages, and code snippets..." : "ค้นหา session ของ Claude Code, Codex, Gemini CLI…", text: $query)
                    .textFieldStyle(.plain)
                    .font(.system(size: 14))
                    .onSubmit {
                        if scope == .external { loadExternal() } else { performSearch() }
                    }

                if !query.isEmpty {
                    Button(action: { query = ""; hits = []; if scope == .external { loadExternal() } }) {
                        Image(systemName: "xmark.circle.fill")
                            .foregroundColor(.secondary)
                    }
                    .buttonStyle(.plain)
                }

                Button("Done", action: onClose)
                    .keyboardShortcut(.cancelAction)
            }
            .padding(14)
            .background(Color(NSColor.controlBackgroundColor))

            Picker("", selection: $scope) {
                ForEach(Scope.allCases) { Text($0.rawValue).tag($0) }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .padding(.horizontal, 14)
            .padding(.bottom, 10)
            .background(Color(NSColor.controlBackgroundColor))

            Divider()

            // Search Results
            if scope == .external {
                externalList
            } else if isSearching {
                VStack {
                    Spacer()
                    ProgressView("Searching sessions...")
                    Spacer()
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if hits.isEmpty {
                VStack(spacing: 10) {
                    Spacer()
                    Image(systemName: "text.magnifyingglass")
                        .font(.system(size: 32))
                        .foregroundColor(.secondary)
                    Text(query.isEmpty ? "Type a search query to search across all sessions" : "No results found for \"\(query)\"")
                        .font(.system(size: 13))
                        .foregroundColor(.secondary)
                    Spacer()
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                List(hits, id: \.id) { hit in
                    Button(action: {
                        onSelectSession(hit.session_id)
                        onClose()
                    }) {
                        VStack(alignment: .leading, spacing: 4) {
                            HStack {
                                Image(systemName: "bubble.left.and.bubble.right")
                                    .foregroundColor(.blue)
                                    .font(.system(size: 11))
                                Text(hit.session_title)
                                    .font(.system(size: 12, weight: .semibold))
                                Spacer()
                                Text("Turn \(hit.seq)")
                                    .font(.system(size: 10, design: .monospaced))
                                    .foregroundColor(.secondary)
                            }
                            Text(hit.snippet)
                                .font(.system(size: 11, design: .monospaced))
                                .foregroundColor(.secondary)
                                .lineLimit(2)
                        }
                        .padding(.vertical, 4)
                    }
                    .buttonStyle(.plain)
                }
            }
        }
        .frame(width: 580, height: 420)
        .background(Color(NSColor.windowBackgroundColor))
        .onChange(of: scope) { _, newScope in
            if newScope == .external { loadExternal() } else if !query.isEmpty { performSearch() }
        }
        .onChange(of: query) { _, newQ in
            if scope == .external {
                // Searching other tools' sessions reads their files, so it
                // runs on Return rather than on every keystroke.
                if newQ.isEmpty { loadExternal() }
            } else if newQ.count >= 2 {
                performSearch()
            } else if newQ.isEmpty {
                hits = []
            }
        }
    }

    // MARK: other tools' sessions

    @ViewBuilder private var externalList: some View {
        if isSearching {
            VStack { Spacer(); ProgressView("กำลังหา session ในเครื่อง…"); Spacer() }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if external.isEmpty {
            VStack(spacing: 10) {
                Spacer()
                Image(systemName: "tray")
                    .font(.system(size: 32))
                    .foregroundColor(.secondary)
                Text(externalError ?? (query.isEmpty ? "ไม่เจอ session ของ Claude Code, Codex หรือ Gemini CLI ในเครื่องนี้" : "ไม่เจอ session ที่ตรงกับ \"\(query)\""))
                    .font(.system(size: 13))
                    .foregroundColor(.secondary)
                    .multilineTextAlignment(.center)
                Spacer()
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .padding(.horizontal, 20)
        } else {
            VStack(spacing: 0) {
                List(external, id: \.path) { s in
                    Button(action: { importAndOpen(s) }) {
                        VStack(alignment: .leading, spacing: 4) {
                            HStack(spacing: 6) {
                                ProviderIcon(provider: s.provider, size: 13)
                                Text(s.title)
                                    .font(.system(size: 12, weight: .semibold))
                                    .lineLimit(1)
                                Spacer()
                                if importing == s.path {
                                    ProgressView().controlSize(.small)
                                } else {
                                    Text(Self.relative(s.updated_ms))
                                        .font(.system(size: 10))
                                        .foregroundColor(.secondary)
                                }
                            }
                            HStack(spacing: 6) {
                                Text(s.agentName)
                                    .font(.system(size: 10, weight: .medium, design: .monospaced))
                                    .foregroundColor(.secondary)
                                if let cwd = s.cwd {
                                    Text(cwd)
                                        .font(.system(size: 10, design: .monospaced))
                                        .foregroundColor(.secondary)
                                        .lineLimit(1)
                                        .truncationMode(.head)
                                }
                            }
                            if let snippet = s.snippet {
                                Text(snippet)
                                    .font(.system(size: 11, design: .monospaced))
                                    .foregroundColor(.secondary)
                                    .lineLimit(2)
                            }
                        }
                        .padding(.vertical, 4)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .disabled(importing != nil)
                    .help("นำเข้าบทสนทนานี้มาเป็น session ใน Chorus แล้วคุยต่อกับ agent ตัวไหนก็ได้")
                }
                if let note = importNote {
                    Divider()
                    Text(note)
                        .font(.system(size: 11))
                        .foregroundColor(.secondary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal, 14)
                        .padding(.vertical, 8)
                }
            }
        }
    }

    private func loadExternal() {
        isSearching = true
        importNote = nil
        engine.externalSessions(query: query) { sessions, error in
            self.external = sessions
            self.externalError = error
            self.isSearching = false
        }
    }

    private func importAndOpen(_ s: Engine.ExternalSession) {
        guard !dbPath.isEmpty else {
            importNote = "ยังไม่ได้เปิดโปรเจกต์ — เปิดโปรเจกต์ก่อนแล้วค่อยนำเข้า"
            return
        }
        importing = s.path
        importNote = nil
        engine.importExternal(s, db: dbPath) { res in
            self.importing = nil
            guard let res, res.ok else {
                self.importNote = res?.error ?? "นำเข้าไม่สำเร็จ"
                return
            }
            // Leave the sheet open only when there is something to read.
            if let skipped = res.not_appended, skipped > 0 {
                self.importNote = "session นี้คุยต่อใน Chorus ไปแล้ว — \(skipped) ข้อความใหม่จากแอปเดิมไม่ได้ต่อท้าย เพื่อไม่ให้สองบทสนทนาปนกัน"
            } else {
                onClose()
            }
        }
    }

    private static func relative(_ ms: Int64) -> String {
        let f = RelativeDateTimeFormatter()
        f.unitsStyle = .short
        return f.localizedString(for: Date(timeIntervalSince1970: TimeInterval(ms) / 1000), relativeTo: Date())
    }

    private func performSearch() {
        guard !query.trimmingCharacters(in: .whitespaces).isEmpty else { return }
        isSearching = true
        engine.searchSessions(dbPath: dbPath, query: query) { results in
            self.hits = results
            self.isSearching = false
        }
    }
}
