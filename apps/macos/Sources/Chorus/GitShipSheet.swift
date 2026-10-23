import SwiftUI
import AppKit
import CChorus

/// Commit → push → PR in one sheet. The logic is `chorus_core::gitflow`;
/// this only collects the choices and shows the result.
struct GitShipSheet: View {
    let workspace: String
    /// Files the session changed (Changes tab). Empty = unknown, show everything.
    var sessionFiles: [String] = []
    /// What the user asked for; seeds the commit message.
    var goal: String = ""
    var onClose: () -> Void

    struct FileRow: Decodable, Identifiable { let path: String; let code: String; let blocked: Bool; var id: String { path } }
    struct GitStatus: Decodable {
        let branch: String; let default_branch: String?; let on_default: Bool
        let upstream: String?; let ahead: Int; let has_remote: Bool; let gh_ready: Bool
        let files: [FileRow]; let hidden: Int?
    }
    struct GitInfo: Decodable { let status: GitStatus; let suggestion: String }
    private struct Reply<T: Decodable>: Decodable { let ok: Bool; let error: String?; let git: T? }
    private struct Generic: Decodable { let ok: Bool; let error: String?; let url: String?; let branch: String? }

    @State private var showAll = false
    @State private var info: GitInfo?
    @State private var loadError: String?
    @State private var chosen: Set<String> = []
    @State private var message = ""
    @State private var doPush = true
    @State private var doPR = false
    @State private var prBody = ""
    @State private var busy = false
    @State private var log: [String] = []
    @State private var failure: String?
    @State private var prURL: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Label("ส่งขึ้น Git", systemImage: "arrow.up.circle").font(.headline)
                Spacer()
                if let s = info?.status { Text(s.branch).font(.caption.monospaced()).foregroundStyle(.secondary) }
            }
            if let loadError {
                Text(loadError).foregroundStyle(.red)
            } else if let info {
                content(info.status)
            } else {
                ProgressView()
            }
            HStack {
                Spacer()
                Button(prURL == nil ? "ยกเลิก" : "ปิด", action: onClose)
                if prURL == nil {
                    Button(action: run) { busy ? AnyView(ProgressView().controlSize(.small)) : AnyView(Text(actionTitle)) }
                        .keyboardShortcut(.defaultAction)
                        .disabled(busy || chosen.isEmpty || message.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            }
        }
        .padding(18)
        .frame(width: 520)
        .onAppear(perform: load)
    }

    private var actionTitle: String {
        doPR ? "Commit · Push · เปิด PR" : (doPush ? "Commit · Push" : "Commit")
    }

    @ViewBuilder private func content(_ s: GitStatus) -> some View {
        if s.files.isEmpty {
            Text("ไม่มีไฟล์ที่เปลี่ยน").foregroundStyle(.secondary)
        } else {
            if let hidden = s.hidden, hidden > 0 {
                HStack(spacing: 6) {
                    Text("ซ่อน \(hidden) ไฟล์ที่ session นี้ไม่ได้แตะ").font(.caption).foregroundStyle(.secondary)
                    Button("แสดงทั้งหมด") { showAll = true; load() }.buttonStyle(.link).font(.caption)
                }
            }
            if s.on_default {
                Label("อยู่บน \(s.branch) — จะแตก branch ใหม่ให้ก่อน commit ไม่ลง \(s.branch) ตรง ๆ", systemImage: "info.circle")
                    .font(.caption).foregroundStyle(.secondary)
            }
            ScrollView {
                VStack(alignment: .leading, spacing: 3) {
                    ForEach(s.files) { f in
                        Toggle(isOn: Binding(
                            get: { chosen.contains(f.path) },
                            set: { on in if on { chosen.insert(f.path) } else { chosen.remove(f.path) } }
                        )) {
                            HStack(spacing: 6) {
                                Text(f.code.trimmingCharacters(in: .whitespaces)).font(.caption2.monospaced())
                                    .foregroundStyle(.secondary).frame(width: 22, alignment: .leading)
                                Text(f.path).font(.system(size: 11, design: .monospaced)).lineLimit(1)
                                if f.blocked { Label("ไฟล์ลับ", systemImage: "lock.fill").font(.caption2).foregroundStyle(.orange) }
                            }
                        }
                        .disabled(f.blocked)
                    }
                }
            }
            .frame(maxHeight: 130)
            TextEditor(text: $message)
                .font(.system(size: 12, design: .monospaced))
                .frame(height: 64)
                .overlay(RoundedRectangle(cornerRadius: 6).stroke(.quaternary))
            Toggle("Push ขึ้น remote", isOn: $doPush).disabled(!s.has_remote)
            Toggle("เปิด Pull Request (gh)", isOn: $doPR)
                .disabled(!s.has_remote || !s.gh_ready)
                .onChange(of: doPR) { _, on in if on { doPush = true } }
            if !s.has_remote { Text("repo นี้ไม่มี remote").font(.caption).foregroundStyle(.secondary) }
            else if !s.gh_ready { Text("ยังไม่ได้ login gh — รัน `gh auth login` ใน Terminal ก่อนถ้าอยากเปิด PR").font(.caption).foregroundStyle(.secondary) }
            if doPR {
                TextEditor(text: $prBody).font(.system(size: 12)).frame(height: 70)
                    .overlay(RoundedRectangle(cornerRadius: 6).stroke(.quaternary))
                    .overlay(alignment: .topLeading) {
                        if prBody.isEmpty { Text("รายละเอียด PR (ไม่บังคับ)").foregroundStyle(.tertiary).padding(6).allowsHitTesting(false) }
                    }
            }
        }
        ForEach(log, id: \.self) { Label($0, systemImage: "checkmark.circle.fill").font(.caption).foregroundStyle(.green) }
        if let failure { Label(failure, systemImage: "xmark.octagon.fill").font(.caption).foregroundStyle(.red).textSelection(.enabled) }
        if let prURL, let url = URL(string: prURL) { Link(prURL, destination: url).font(.caption) }
    }

    private static func call(_ f: () -> UnsafeMutablePointer<CChar>?) -> Data {
        guard let p = f() else { return Data() }
        defer { chorus_string_free(p) }
        return Data(String(cString: p).utf8)
    }

    private func load() {
        let ws = workspace
        DispatchQueue.global().async {
            let scopeJSON = (showAll || sessionFiles.isEmpty) ? "null"
                : (String(data: (try? JSONEncoder().encode(sessionFiles)) ?? Data("null".utf8), encoding: .utf8) ?? "null")
            let goalText = goal
            let data = Self.call { ws.withCString { w in scopeJSON.withCString { sc in goalText.withCString { chorus_git_status_scoped_json(w, sc, $0) } } } }
            let r = try? JSONDecoder().decode(Reply<GitInfo>.self, from: data)
            DispatchQueue.main.async {
                if let g = r?.git, r?.ok == true {
                    info = g
                    if !g.status.has_remote { doPush = false; doPR = false }
                    chosen = Set(g.status.files.filter { !$0.blocked }.map(\.path))
                    message = g.suggestion
                } else {
                    loadError = r?.error ?? "อ่านสถานะ git ไม่ได้"
                }
            }
        }
    }

    private func run() {
        busy = true; failure = nil; log = []
        let canPush = info?.status.has_remote ?? false
        let (ws, msg, paths, push, pr, body) = (workspace, message, Array(chosen), doPush && canPush, doPR && canPush, prBody)
        let base = info?.status.default_branch
        DispatchQueue.global().async {
            func step(_ f: () -> UnsafeMutablePointer<CChar>?) -> Generic? {
                try? JSONDecoder().decode(Generic.self, from: Self.call(f))
            }
            func fail(_ e: String?) { DispatchQueue.main.async { failure = e ?? "ไม่สำเร็จ"; busy = false } }
            let pathsJSON = String(data: (try? JSONEncoder().encode(paths)) ?? Data("[]".utf8), encoding: .utf8) ?? "[]"
            let c = step({ ws.withCString { w in msg.withCString { m in pathsJSON.withCString { chorus_git_commit_json(w, m, $0) } } } })
            guard c?.ok == true else { return fail(c?.error ?? "commit ไม่สำเร็จ") }
            DispatchQueue.main.async { log.append("commit แล้ว") }
            if push || pr {
                let p = step({ ws.withCString { chorus_git_push_json($0) } })
                guard p?.ok == true else { return fail("commit แล้วแต่ push ไม่สำเร็จ: " + (p?.error ?? "")) }
                DispatchQueue.main.async { log.append("push แล้ว") }
            }
            if pr {
                let title = msg.components(separatedBy: "\n").first ?? msg
                let r = step({ ws.withCString { w in title.withCString { t in body.withCString { b in
                    (base ?? "").withCString { chorus_git_pr_json(w, t, b, $0) } } } } })
                guard r?.ok == true, let url = r?.url else { return fail("push แล้วแต่เปิด PR ไม่สำเร็จ: " + (r?.error ?? "")) }
                DispatchQueue.main.async { prURL = url; log.append("เปิด PR แล้ว") }
            }
            DispatchQueue.main.async { busy = false }
        }
    }
}
