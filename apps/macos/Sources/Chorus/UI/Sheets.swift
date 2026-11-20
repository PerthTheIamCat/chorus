import AppKit
import SwiftUI

// The windows that open over the main one: ⌘K, welcome, hand over, search all
// sessions, compare, route review, Commit & PR. Each follows the design's
// sheet: title + subtitle + close on top, content, actions bottom-right with
// one blue button.

// MARK: ⌘K

struct CommandPalette: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel
    let dismiss: () -> Void
    let openTool: (ShellTool) -> Void
    @State private var query = ""
    @State private var index = 0
    @FocusState private var focused: Bool

    private struct Row: Identifiable {
        let id = UUID()
        let section: String
        let title: String
        var icon: String?
        var provider: String?
        var meta = ""
        var key: String?
        let run: () -> Void
    }

    private var rows: [Row] {
        var out: [Row] = []
        let a = shell.paletteItems { openTool($0) }
        for item in a {
            out.append(Row(section: "คำสั่ง", title: item.title, icon: item.icon, run: item.run))
        }
        for agent in engine.agents where agent.ready {
            out.append(Row(section: "สลับไปใช้", title: "เทิร์นถัดไปให้ \(agent.label)", provider: agent.id,
                           meta: quota(agent.id)) { shell.choose(agent.defaultModel) })
        }
        for f in shell.projects.projects.flatMap(\.folders) {
            for s in shell.sessions(in: f).prefix(30) {
                out.append(Row(section: "Session", title: shell.title(of: s), provider: s.agents.first,
                               meta: (f as NSString).lastPathComponent) { shell.open(SessionKey(folder: f, id: s.id)) })
            }
        }
        if !query.isEmpty {
            for f in shell.fileIndex.match(query, limit: 6) {
                out.append(Row(section: "ไฟล์", title: f, icon: "doc.text") { shell.text += "@\(f) " })
            }
        }
        let q = query.trimmingCharacters(in: .whitespaces)
        return q.isEmpty ? out.filter { $0.section != "Session" || out.firstIndex(where: { $0.section == "Session" }) != nil }.prefix(40).map { $0 }
            : out.filter { $0.title.localizedCaseInsensitiveContains(q) || $0.meta.localizedCaseInsensitiveContains(q) }
    }

    var body: some View {
        let list = rows
        ZStack(alignment: .top) {
            Color.black.opacity(0.35).ignoresSafeArea().onTapGesture(perform: dismiss)
            VStack(spacing: 0) {
                HStack(spacing: 10) {
                    Image(systemName: "magnifyingglass").font(.system(size: 16)).foregroundStyle(Palette.text2)
                    TextField("ค้นหา หรือสั่งงาน", text: $query)
                        .textFieldStyle(.plain).font(.system(size: 16)).focused($focused)
                        .onSubmit { run(list) }
                        .onKeyPress(.downArrow) { index = min(index + 1, max(list.count - 1, 0)); return .handled }
                        .onKeyPress(.upArrow) { index = max(index - 1, 0); return .handled }
                        .onKeyPress(.escape) { dismiss(); return .handled }
                    Text("คำสั่ง · session · ไฟล์").font(Typo.tiny).foregroundStyle(Palette.text3)
                }
                .padding(.horizontal, 16).frame(height: 54)
                Hairline()
                ScrollViewReader { proxy in
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: 1) {
                            ForEach(Array(list.enumerated()), id: \.element.id) { i, r in
                                if i == 0 || list[i - 1].section != r.section {
                                    GroupHeader(r.section).padding(.horizontal, 10).padding(.top, i == 0 ? 4 : 10).padding(.bottom, 3)
                                }
                                Button { index = i; run(list) } label: {
                                    HStack(spacing: 12) {
                                        Group {
                                            if let p = r.provider { ProviderAvatar(provider: p, size: 22) }
                                            else { Image(systemName: r.icon ?? "circle").frame(width: 22) }
                                        }
                                        Text(r.title).font(.system(size: 13.5)).foregroundStyle(Palette.text).lineLimit(1)
                                        Spacer()
                                        Text(r.meta).font(Typo.tiny).foregroundStyle(Palette.text3)
                                        if i == index { Text("↩").font(Typo.tiny).foregroundStyle(Palette.text3) }
                                    }
                                }
                                .buttonStyle(RowStyle(selected: i == index, height: 40))
                                .id(i)
                            }
                        }
                        .padding(8)
                    }
                    .onChange(of: index) { _, i in proxy.scrollTo(i) }
                }
                .frame(maxHeight: 460)
                Hairline()
                HStack(spacing: 16) {
                    HStack(spacing: 4) { Kbd("↑↓"); Text("เลื่อน") }
                    HStack(spacing: 4) { Kbd("↩"); Text("ทำ") }
                    Spacer()
                    HStack(spacing: 4) { Kbd("esc"); Text("ปิด") }
                }
                .font(Typo.tiny).foregroundStyle(Palette.text3)
                .padding(.horizontal, 16).frame(height: 38)
            }
            .frame(width: 660)
            .background(Palette.elevated, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).strokeBorder(Palette.line2))
            .shadow(color: .black.opacity(0.4), radius: 40, y: 20)
            .padding(.top, 90)
        }
        .onAppear { focused = true }
        .onChange(of: query) { _, _ in index = 0 }
    }

    private func run(_ list: [Row]) {
        guard list.indices.contains(index) else { return }
        dismiss()
        list[index].run()
    }

    private func quota(_ agent: String) -> String {
        let used = engine.accounts.filter { $0.agent == agent }.compactMap(\.worst).min()
        return used.map { "\(Int(($0 * 100).rounded()))%" } ?? ""
    }
}

// MARK: welcome

/// Design board "เปิดครั้งแรก": what Chorus does, drop a folder, which agents
/// this Mac has, and sessions to continue from other apps.
struct WelcomeView: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel
    @State private var targeted = false

    var body: some View {
        ScrollView {
            HStack(alignment: .center, spacing: 64) {
                VStack(alignment: .leading, spacing: 28) {
                    VStack(alignment: .leading, spacing: 14) {
                        Image(nsImage: NSApp.applicationIconImage).resizable().frame(width: 72, height: 72)
                        Text("หลาย agent · บริบทเดียว").font(.system(size: 28, weight: .semibold))
                        Text("เลือกโฟลเดอร์แล้วสั่งงานได้เลย ตัวไหนติดลิมิต Chorus ส่งไม้ให้ตัวถัดไปพร้อมทุกอย่างที่คุยมา")
                            .font(.system(size: 15)).foregroundStyle(Palette.text2).lineSpacing(3)
                    }
                    HStack(spacing: 16) {
                        Image(systemName: "folder").font(.system(size: 18)).frame(width: 44, height: 44)
                            .background(Palette.hover, in: RoundedRectangle(cornerRadius: 11))
                        VStack(alignment: .leading, spacing: 2) {
                            Text("ลากโฟลเดอร์มาวาง").font(.system(size: 14, weight: .medium))
                            Text("หลายโฟลเดอร์เป็นโปรเจกต์เดียวก็ได้").font(Typo.small).foregroundStyle(Palette.text3)
                        }
                        Spacer()
                        Button { shell.addProject() } label: { HStack { Text("เลือกโฟลเดอร์"); Text("⌘O").opacity(0.6) } }
                            .buttonStyle(PrimaryButtonStyle())
                            .keyboardShortcut("o", modifiers: .command)
                    }
                    .padding(22)
                    .overlay(RoundedRectangle(cornerRadius: 14).strokeBorder(targeted ? Palette.accent : Palette.line2,
                                                                             style: StrokeStyle(lineWidth: 1.5, dash: [6, 4])))
                    .onDrop(of: [.fileURL], isTargeted: $targeted) { providers in
                        for p in providers {
                            _ = p.loadObject(ofClass: URL.self) { url, _ in
                                guard let url else { return }
                                Task { @MainActor in
                                    shell.projects.upsert(ChorusProject(name: url.lastPathComponent, folders: [url.path]))
                                    shell.open(folder: url.path)
                                }
                            }
                        }
                        return true
                    }
                    VStack(alignment: .leading, spacing: 10) {
                        HStack(alignment: .firstTextBaseline, spacing: 8) {
                            Text("วงของคุณบนเครื่องนี้").font(Typo.headline)
                            Text("\(engine.agents.filter(\.ready).count) พร้อม · ตรวจแค่ว่ามีไฟล์ login ไม่อ่านค่า")
                                .font(Typo.caption).foregroundStyle(Palette.text3)
                        }
                        VStack(spacing: 0) {
                            ForEach(Array(engine.agents.enumerated()), id: \.element.id) { i, a in
                                if i > 0 { Hairline() }
                                HStack(spacing: 12) {
                                    ProviderAvatar(provider: a.id, size: 26)
                                    Text(a.label).font(.system(size: 13, weight: .medium)).frame(width: 110, alignment: .leading)
                                    Text(a.ready ? a.detail : (a.hint.isEmpty ? a.detail : a.hint))
                                        .font(Typo.caption).foregroundStyle(Palette.text3).lineLimit(1)
                                    Spacer()
                                    if a.ready { Tag(text: "พร้อม", tone: .ok, symbol: "checkmark") }
                                    else { Tag(text: a.auth == "needs_login" ? "ต้อง login" : "ไม่พบ CLI", tone: .warn) }
                                }
                                .padding(.horizontal, 14).frame(height: 50)
                            }
                        }
                        .background(Palette.surface, in: RoundedRectangle(cornerRadius: 12))
                        .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(Palette.line))
                    }
                    HStack(spacing: 16) {
                        HStack(spacing: 4) { Kbd("⌘K"); Text("ทำทุกอย่าง") }
                        HStack(spacing: 4) { Kbd("⌘N"); Text("session ใหม่") }
                        HStack(spacing: 4) { Kbd("⇧⌘F"); Text("ค้นทุก session") }
                    }
                    .font(Typo.small).foregroundStyle(Palette.text3)
                }
                .frame(maxWidth: 600)
            }
            .padding(48)
            .frame(maxWidth: .infinity, minHeight: 700)
        }
        .background(Palette.window)
    }
}

extension ShellModel {
    /// Open the first session of a folder (or a draft) after it was added.
    func open(folder: String) {
        workspace = folder
        engine.loadSessions(db: dbPath(for: folder))
        if let s = sessions(in: folder).first { open(SessionKey(folder: folder, id: s.id)) } else { newSession(in: folder) }
    }
}

// MARK: hand over

/// Design board "ส่งไม้ด้วยมือ": pick who takes over; the brief comes from the
/// ledger (no form), shown before it goes.
struct HandoffSheet: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel
    @Environment(\.dismiss) private var dismiss
    @State private var target: String?

    private var from: String { shell.agent(for: shell.model)?.id ?? "" }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            SheetHeader(title: "ส่งไม้", subtitle: "brief สร้างจาก ledger ให้แล้ว ไม่ต้องกรอกเอง") { dismiss() }
            VStack(alignment: .leading, spacing: 10) {
                HStack(spacing: 6) {
                    GroupHeader("จาก")
                    ProviderIcon(provider: from, size: 12)
                    Text(shell.agentLabel(from)).font(Typo.caption).foregroundStyle(Palette.text2)
                    GroupHeader("→ ให้")
                }
                HStack(spacing: 8) {
                    ForEach(engine.agents.filter { $0.ready && $0.id != from }) { a in
                        Button {
                            target = a.id
                            shell.choose(a.defaultModel)
                        } label: {
                            VStack(alignment: .leading, spacing: 6) {
                                HStack(spacing: 8) {
                                    ProviderAvatar(provider: a.id, size: 22)
                                    Text(a.label).font(.system(size: 13, weight: .semibold)).foregroundStyle(Palette.text)
                                    Spacer()
                                    if target == a.id { Image(systemName: "checkmark").foregroundStyle(Palette.accentText) }
                                }
                                Text(quota(a.id)).font(Typo.caption).foregroundStyle(Palette.text2)
                            }
                            .padding(12)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .background(target == a.id ? Palette.accentWash : .clear, in: RoundedRectangle(cornerRadius: 10))
                            .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(target == a.id ? Palette.accent : Palette.line2,
                                                                                     lineWidth: target == a.id ? 1.5 : 1))
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
            .padding(.horizontal, 20).padding(.bottom, 14)
            Hairline()
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    switch engine.handoffPrep {
                    case .ready(let p):
                        if let g = p.goal { brief("เป้าหมาย", [g]) }
                        brief("เสร็จแล้ว · \((p.done ?? []).count)", p.done ?? [], symbol: "checkmark", tint: Palette.ok)
                        brief("ค้างอยู่ · \((p.pending ?? []).count)", p.pending ?? [])
                        brief("ข้อจำกัด", p.constraints ?? [], symbol: "exclamationmark.shield", tint: Palette.warn)
                    case .preparing:
                        HStack { ProgressView().controlSize(.small); Text("กำลังสร้าง brief จาก ledger…").foregroundStyle(Palette.text2) }
                    case .failed(let e):
                        Text("สร้าง brief ไม่สำเร็จ: \(e) ส่งไม้ได้ตามปกติ ระบบจะสรุปตอนส่ง").foregroundStyle(Palette.warn)
                    case .none:
                        Text("เลือกผู้รับข้างบนเพื่อดู brief").foregroundStyle(Palette.text3)
                    }
                }
                .font(Typo.small)
                .padding(20)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .frame(minHeight: 260)
            Hairline()
            HStack {
                Text("agent ใหม่จะทวนความเข้าใจก่อนลงมือ").font(Typo.caption).foregroundStyle(Palette.text3)
                Spacer()
                Button("ยกเลิก") { engine.cancelHandoffPrep(); dismiss() }.buttonStyle(SecondaryButtonStyle())
                Button("ส่งให้ \(target.map(shell.agentLabel) ?? "…")") {
                    if let t = target { engine.handoffAuto(to: t, level: shell.mode == .allow ? ShellModel.Level.allow : 0) }
                    dismiss()
                }
                .buttonStyle(PrimaryButtonStyle())
                .keyboardShortcut(.defaultAction)
                .disabled(target == nil || engine.busy)
            }
            .padding(16)
        }
        .frame(width: 720)
        .background(Palette.elevated)
    }

    @ViewBuilder
    private func brief(_ title: String, _ items: [String], symbol: String? = nil, tint: Color = Palette.text3) -> some View {
        if !items.isEmpty {
            VStack(alignment: .leading, spacing: 5) {
                GroupHeader(title)
                ForEach(items, id: \.self) { i in
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        if let symbol { Image(systemName: symbol).font(.system(size: 10, weight: .bold)).foregroundStyle(tint) }
                        Text(i).foregroundStyle(Palette.text)
                    }
                }
            }
        }
    }

    private func quota(_ agent: String) -> String {
        let accs = engine.accounts.filter { $0.agent == agent }
        if !accs.isEmpty && accs.allSatisfy(\.limited) { return "ติดลิมิต" }
        guard let used = accs.compactMap(\.worst).min() else { return "พร้อม" }
        return "เหลือ \(100 - Int((used * 100).rounded()))%"
    }
}

// MARK: search all sessions

/// Design board "ค้นทุก session + นำเข้า": results on the left, a preview on
/// the right; sessions from Claude Code / Codex / Gemini CLI can be imported.
struct SearchSheet: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel
    @Environment(\.dismiss) private var dismiss

    enum Scope: String, CaseIterable, Identifiable { case project = "ใน Chorus", others = "จากแอปอื่น"; var id: String { rawValue } }
    @State private var scope: Scope = .project
    @State private var query = ""
    @State private var hits: [Engine.SearchHit] = []
    @State private var external: [Engine.ExternalSession] = []
    @State private var pickedExternal: String?
    @State private var searching = false
    @State private var error: String?
    @State private var importing = false
    @FocusState private var focused: Bool

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 10) {
                Image(systemName: "magnifyingglass").font(.system(size: 16)).foregroundStyle(Palette.text2)
                TextField("ค้นทุก session แล้วกด ↩", text: $query).textFieldStyle(.plain).font(.system(size: 16))
                    .focused($focused).onSubmit(search)
                Picker("ขอบเขต", selection: $scope) { ForEach(Scope.allCases) { Text($0.rawValue).tag($0) } }
                    .pickerStyle(.segmented).labelsHidden().fixedSize()
                Button { dismiss() } label: { Image(systemName: "xmark") }.buttonStyle(IconButtonStyle())
                    .keyboardShortcut(.cancelAction).accessibilityLabel("ปิด")
            }
            .padding(.horizontal, 16).frame(height: 56)
            Hairline()
            if let error { Text(error).font(Typo.caption).foregroundStyle(Palette.warn).padding(8) }
            if searching {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if scope == .project {
                if hits.isEmpty {
                    ContentUnavailableView(query.isEmpty ? "ค้นเนื้อหาทุกแชทในโปรเจกต์นี้" : "ไม่เจอ", systemImage: "magnifyingglass")
                } else {
                    List(hits) { h in
                        Button { shell.open(SessionKey(folder: shell.workspace, id: h.session_id)); dismiss() } label: {
                            HStack(alignment: .top, spacing: 10) {
                                ProviderAvatar(provider: h.actor, size: 20)
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(h.session_title).font(.system(size: 13, weight: .medium)).lineLimit(1)
                                    Text(h.snippet).font(Typo.caption).foregroundStyle(Palette.text2).lineLimit(2)
                                }
                            }
                            .padding(.vertical, 4)
                        }
                        .buttonStyle(.plain)
                    }
                    .scrollContentBackground(.hidden)
                }
            } else {
                HStack(spacing: 0) {
                    List(external, selection: $pickedExternal) { s in
                        HStack(alignment: .top, spacing: 10) {
                            ProviderAvatar(provider: SidebarView.providerKey(s.provider), size: 20)
                            VStack(alignment: .leading, spacing: 2) {
                                Text(s.title).font(.system(size: 13, weight: .medium)).lineLimit(1)
                                Text(s.cwd ?? "").font(Typo.monoSmall).foregroundStyle(Palette.text3).lineLimit(1).truncationMode(.head)
                                if let sn = s.snippet { Text(sn).font(Typo.caption).foregroundStyle(Palette.text2).lineLimit(1) }
                            }
                        }
                        .padding(.vertical, 4)
                        .tag(s.path)
                    }
                    .scrollContentBackground(.hidden)
                    .frame(width: 400)
                    Hairline(vertical: true)
                    if let s = external.first(where: { $0.path == pickedExternal }) {
                        VStack(alignment: .leading, spacing: 12) {
                            HStack(spacing: 10) {
                                ProviderAvatar(provider: SidebarView.providerKey(s.provider), size: 26)
                                VStack(alignment: .leading) {
                                    Text(s.title).font(Typo.headline)
                                    Text("\(s.provider) · \(s.cwd ?? "")").font(Typo.caption).foregroundStyle(Palette.text3)
                                }
                            }
                            if let sn = s.snippet { Text(sn).font(Typo.ui).foregroundStyle(Palette.text2) }
                            Spacer()
                            HStack {
                                Text("คัดลอกเข้า ledger ของ Chorus · ไฟล์ต้นทางไม่ถูกแก้").font(Typo.caption).foregroundStyle(Palette.text3)
                                Spacer()
                                Button(importing ? "กำลังนำเข้า…" : "นำเข้าแล้วคุยต่อ") { bringIn(s) }
                                    .buttonStyle(PrimaryButtonStyle()).disabled(importing)
                            }
                        }
                        .padding(18)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    } else {
                        ContentUnavailableView("เลือก session ทางซ้าย", systemImage: "square.and.arrow.down")
                    }
                }
            }
        }
        .frame(width: 980, height: 640)
        .background(Palette.elevated)
        .onAppear { focused = true }
        .onChange(of: scope) { _, s in if s == .others && external.isEmpty { search() } }
    }

    private func search() {
        error = nil
        searching = true
        if scope == .project {
            guard !query.trimmingCharacters(in: .whitespaces).isEmpty else { searching = false; return }
            engine.searchSessions(dbPath: shell.currentDB, query: query) { hits = $0; searching = false }
        } else {
            engine.externalSessions(query: query) { s, e in external = s; error = e; searching = false }
        }
    }

    private func bringIn(_ s: Engine.ExternalSession) {
        guard !shell.workspace.isEmpty else { error = "เปิดโปรเจกต์ก่อนแล้วค่อยนำเข้า"; return }
        importing = true
        engine.importExternal(s, db: shell.currentDB) { res in
            importing = false
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

// MARK: compare

struct CompareConfirmSheet: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel

    var body: some View {
        let agents = engine.agents.filter(\.ready)
        let source = shell.comparePrompt
        VStack(alignment: .leading, spacing: 0) {
            SheetHeader(title: "เทียบ agent", subtitle: "คำถามเดียวกัน แยก fork ต่อ agent · ใช้โควตาของแต่ละตัวหนึ่งเทิร์น") {
                shell.confirmCompare = false
            }
            VStack(alignment: .leading, spacing: 14) {
                Text(source.text).lineLimit(5).font(Typo.ui)
                    .padding(12).frame(maxWidth: .infinity, alignment: .leading)
                    .background(Palette.hover, in: RoundedRectangle(cornerRadius: 8))
                HStack(spacing: 8) {
                    ForEach(agents) { a in
                        HStack(spacing: 6) { ProviderAvatar(provider: a.id, size: 20); Text(a.label) }
                            .font(Typo.small).padding(.horizontal, 10).frame(height: 32)
                            .overlay(Capsule().strokeBorder(Palette.line2))
                    }
                }
                Toggle("ตัดสินด้วยหลักฐาน: รันเทสต์ในแต่ละสำเนา แล้วให้อีกเจ้าจัดอันดับ", isOn: $shell.compareJudge)
                Toggle("เทียบ agent เดี่ยว กับ Orchestra ของ agent เดียวกัน", isOn: $shell.compareOrchestra)
            }
            .toggleStyle(.checkbox).font(Typo.small)
            .padding(.horizontal, 20)
            HStack {
                Spacer()
                Button("ยกเลิก") { shell.confirmCompare = false }.buttonStyle(SecondaryButtonStyle())
                Button("เทียบเลย") { shell.confirmCompare = false; shell.performCompare() }
                    .buttonStyle(PrimaryButtonStyle()).keyboardShortcut(.defaultAction)
            }
            .padding(20)
        }
        .frame(width: 560)
        .background(Palette.elevated)
    }
}

/// Design board "เทียบ agent": one column per agent with time, tokens, cost,
/// diff stats, the summary, a short diff and the test result; "ใช้ผลนี้" opens
/// that fork.
struct CompareSheet: View {
    @EnvironmentObject private var shell: ShellModel
    @Environment(\.dismiss) private var dismiss
    let json: String

    private var payload: CompareResultsSheet.Payload? {
        json.data(using: .utf8).flatMap { try? JSONDecoder().decode(CompareResultsSheet.Payload.self, from: $0) }
    }

    var body: some View {
        let branches = payload?.branches ?? []
        let best = branches.min { ($0.score?.rank ?? 99) < ($1.score?.rank ?? 99) }
        VStack(spacing: 0) {
            HStack {
                SheetHeader(title: "เทียบ agent", subtitle: payload?.verdict?.summary) { dismiss() }
            }
            Hairline()
            if let e = payload?.error { Text(e).foregroundStyle(Palette.bad).padding() }
            ScrollView {
                HStack(alignment: .top, spacing: 16) {
                    ForEach(branches) { b in column(b, best: b.session_id == best?.session_id && best?.score?.rank != nil) }
                }
                .padding(20)
            }
        }
        .frame(minWidth: 1000, minHeight: 640)
        .background(Palette.window)
    }

    private func column(_ b: CompareResultsSheet.Branch, best: Bool) -> some View {
        let hunks = parseHunks(b.diff ?? "")
        let add = hunks.reduce(0) { $0 + $1.addCount }, del = hunks.reduce(0) { $0 + $1.delCount }
        return VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 10) {
                ProviderAvatar(provider: b.agent, size: 32)
                VStack(alignment: .leading) {
                    Text(shell.agentLabel(b.agent)).font(Typo.headline)
                    Text(b.agent).font(Typo.monoSmall).foregroundStyle(Palette.text3)
                }
                Spacer()
                if best { Tag(text: "อันดับ 1", tone: .ok) }
            }
            .padding(14)
            Hairline()
            HStack {
                stat("เวลา", b.elapsed_ms.map(CompareResultsSheet.duration) ?? "—")
                stat("token", b.context_used.map(CompareResultsSheet.compactCount) ?? "—")
                stat("ราคาเทียบ API", b.cost_usd.map { String(format: "$%.2f", $0) } ?? "—")
                VStack(alignment: .leading, spacing: 2) {
                    Text("diff").font(Typo.tiny).foregroundStyle(Palette.text3)
                    HStack(spacing: 4) { Text("+\(add)").foregroundStyle(Palette.ok); Text("−\(del)").foregroundStyle(Palette.bad) }
                        .font(Typo.mono)
                }
            }
            .padding(14)
            Hairline()
            VStack(alignment: .leading, spacing: 10) {
                if let e = b.error { Text(e).foregroundStyle(Palette.bad).font(Typo.small) }
                Text(CompareResultsSheet.markdown(b.message)).font(Typo.ui).lineLimit(8).textSelection(.enabled)
                if !hunks.isEmpty {
                    VStack(alignment: .leading, spacing: 0) {
                        ForEach(Array(hunks.prefix(2).flatMap { $0.lines.prefix(8) }.enumerated()), id: \.offset) { i, l in
                            DiffPreviewLine(text: l)
                        }
                    }
                    .padding(.vertical, 6)
                    .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(Palette.line))
                }
                if let s = b.score {
                    let passed = s.tests?.passed ?? s.passed
                    Text(passed == true ? "✓ เทสต์ผ่าน" : passed == false ? "✕ เทสต์ล้ม" : "ไม่ได้รันเทสต์")
                        .font(Typo.small).foregroundStyle(passed == true ? Palette.ok : passed == false ? Palette.bad : Palette.text3)
                    if let r = s.reason { Text(r).font(Typo.caption).foregroundStyle(Palette.text3) }
                }
            }
            .padding(14)
            Spacer(minLength: 0)
            Hairline()
            HStack {
                Spacer()
                Button("ใช้ผลนี้") { shell.open(SessionKey(folder: shell.workspace, id: b.session_id)); dismiss() }
                    .buttonStyle(PrimaryButtonStyle(small: true))
            }
            .padding(12)
        }
        .frame(minWidth: 300, maxWidth: .infinity)
        .background(Palette.surface, in: RoundedRectangle(cornerRadius: 12))
        .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(Palette.line))
    }

    private func stat(_ k: String, _ v: String) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(k).font(Typo.tiny).foregroundStyle(Palette.text3)
            Text(v).font(Typo.mono).foregroundStyle(Palette.text)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

private struct DiffPreviewLine: View {
    let text: String
    var body: some View {
        let add = text.hasPrefix("+"), del = text.hasPrefix("-")
        Text(text.isEmpty ? " " : text).font(Typo.monoSmall)
            .foregroundStyle(add || del ? Palette.text : Palette.text2)
            .lineLimit(1)
            .padding(.horizontal, 10).frame(maxWidth: .infinity, minHeight: 19, alignment: .leading)
            .background(add ? Palette.addWash : del ? Palette.delWash : .clear)
    }
}

// MARK: route review

struct RouteReviewSheet: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel
    let proposal: ShellModel.PendingRoute

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            SheetHeader(title: "เลือกโมเดลก่อนส่ง", subtitle: proposal.reason) { shell.cancelReviewedRoute() }
            VStack(alignment: .leading, spacing: 14) {
                Text(proposal.send.text).lineLimit(4).font(Typo.ui)
                    .padding(12).frame(maxWidth: .infinity, alignment: .leading)
                    .background(Palette.hover, in: RoundedRectangle(cornerRadius: 8))
                Picker("โมเดล", selection: $shell.reviewModel) {
                    ForEach(engine.agents.filter(\.ready)) { a in
                        Section(a.label) { ForEach(a.models, id: \.id) { m in Text(m.label).tag(m.id) } }
                    }
                }
            }
            .padding(.horizontal, 20)
            HStack {
                Spacer()
                Button("ยกเลิก") { shell.cancelReviewedRoute() }.buttonStyle(SecondaryButtonStyle())
                Button("ส่ง") { shell.sendReviewedRoute() }.buttonStyle(PrimaryButtonStyle())
                    .keyboardShortcut(.defaultAction)
                    .disabled(shell.agent(for: shell.reviewModel)?.ready != true)
            }
            .padding(20)
        }
        .frame(width: 500)
        .background(Palette.elevated)
    }
}

// MARK: Commit & PR

/// Design board "Commit & PR". The git work itself (status, commit, push, PR)
/// stays in `GitShipSheet`'s engine calls; this is its frame.
struct ShipSheet: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(spacing: 0) {
            SheetHeader(title: "Commit & PR", subtitle: "เฉพาะก้อนที่คุณเก็บ · push ทำนอก sandbox ด้วยสิทธิ์ของคุณ") { dismiss() }
            Hairline()
            GitShipSheet(workspace: shell.workspace, sessionFiles: engine.diffFiles,
                         goal: TaskSection.parse(engine.taskState).first?.items.first ?? "") { dismiss() }
        }
        .frame(width: 640)
        .background(Palette.elevated)
    }
}
