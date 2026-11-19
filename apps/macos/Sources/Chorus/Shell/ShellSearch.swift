import SwiftUI

// Search every conversation of the project, or find sessions from the other
// tools on this Mac (Claude Code, Codex, Gemini CLI) and bring one in.

struct SearchTool: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel
    let dismiss: () -> Void

    enum Scope: String, CaseIterable, Identifiable {
        case project = "ในโปรเจกต์", others = "จากแอปอื่น"
        var id: String { rawValue }
    }

    @State private var scope: Scope = .project
    @State private var query = ""
    @State private var hits: [Engine.SearchHit] = []
    @State private var external: [Engine.ExternalSession] = []
    @State private var searching = false
    @State private var error: String?
    @State private var importing: String?

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                TextField("พิมพ์คำที่จะค้น แล้วกด ↩", text: $query)
                    .textFieldStyle(.roundedBorder)
                    .onSubmit(search)
                Picker("ขอบเขต", selection: $scope) {
                    ForEach(Scope.allCases) { Text($0.rawValue).tag($0) }
                }
                .pickerStyle(.segmented).labelsHidden().frame(width: 200)
            }
            .padding(10)
            Divider()
            if searching {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if scope == .project {
                if hits.isEmpty {
                    ContentUnavailableView(query.isEmpty ? "ค้นเนื้อหาทุกแชทในโปรเจกต์นี้" : "ไม่เจอ",
                                           systemImage: "magnifyingglass")
                } else {
                    List(hits) { h in
                        Button { shell.open(SessionKey(folder: shell.workspace, id: h.session_id)); dismiss() } label: {
                            VStack(alignment: .leading, spacing: 2) {
                                HStack {
                                    Text(h.session_title).font(.callout.weight(.medium)).lineLimit(1)
                                    Spacer()
                                    Text(h.actor).font(.caption).foregroundStyle(.secondary)
                                }
                                Text(h.snippet).font(.caption).foregroundStyle(.secondary).lineLimit(2)
                            }
                        }
                        .buttonStyle(.plain)
                    }
                }
            } else {
                if let error { Text(error).font(.caption).foregroundStyle(.orange).padding(8) }
                if external.isEmpty {
                    ContentUnavailableView("ค้น session ของ Claude Code, Codex และ Gemini CLI ในเครื่องนี้",
                                           systemImage: "square.and.arrow.down")
                } else {
                    List(external) { s in
                        HStack {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(s.title).font(.callout.weight(.medium)).lineLimit(1)
                                Text("\(s.provider) · \(s.cwd ?? "")").font(.caption).foregroundStyle(.secondary)
                                    .lineLimit(1).truncationMode(.head)
                            }
                            Spacer()
                            if importing == s.path {
                                ProgressView().controlSize(.small)
                            } else {
                                Button("นำเข้า") { bringIn(s) }
                            }
                        }
                    }
                }
            }
        }
        .onChange(of: scope) { _, _ in if scope == .others && external.isEmpty { search() } }
    }

    private func search() {
        error = nil
        searching = true
        if scope == .project {
            guard !query.trimmingCharacters(in: .whitespaces).isEmpty else { searching = false; return }
            engine.searchSessions(dbPath: shell.currentDB, query: query) { r in hits = r; searching = false }
        } else {
            engine.externalSessions(query: query) { sessions, err in
                external = sessions
                error = err
                searching = false
            }
        }
    }

    private func bringIn(_ s: Engine.ExternalSession) {
        guard !shell.workspace.isEmpty else { error = "เปิดโปรเจกต์ก่อนแล้วค่อยนำเข้า"; return }
        importing = s.path
        engine.importExternal(s, db: shell.currentDB) { res in
            importing = nil
            guard let res, res.ok else { error = res?.error ?? "นำเข้าไม่สำเร็จ"; return }
            if let skipped = res.not_appended, skipped > 0 {
                error = "session นี้คุยต่อใน Chorus ไปแล้ว \(skipped) ข้อความใหม่จากแอปเดิมไม่ได้ต่อท้าย"
            } else {
                shell.reloadSessions()
                dismiss()
            }
        }
    }
}
