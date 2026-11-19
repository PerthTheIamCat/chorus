import AppKit
import SwiftUI
import UniformTypeIdentifiers
import UserNotifications

// ContentView, split out of ContentView.swift: first-run setup card and router-model upkeep.
// Moved verbatim; only `private` was dropped so the pieces can see each other.

extension ContentView {
    /// Keep the evaluated candidates in quality order. Unknown local models
    /// remain available for manual selection, but are not chosen automatically.
    static let preferredRouterModels = [
        "chorus-openthai-systemone:0.8b", "tev1:0.8b", "qwen3:1.7b", "qwen2.5:3b"
    ]

    func reconcileRouterModel() {
        guard autoRoute, routerModelAuto, let installed = engine.routerModels else { return }
        if let best = Self.preferredRouterModels.first(where: { installed.contains($0) }) {
            if routerModel != best { routerModel = best }
        } else if !routerAutoInstallAttempted && !engine.routerDownloading {
            routerAutoInstallAttempted = true
            engine.pullRouterModel(Self.preferredRouterModels[0])
        }
    }

    func migrateRouterModelPreference() {
        let defaults = UserDefaults.standard
        guard defaults.object(forKey: "chorus.routerModelAuto") == nil else { return }
        // Before the automatic option existed, these were the two shipped
        // defaults. Preserve a different value that the user typed manually.
        if let saved = defaults.string(forKey: "chorus.routerModel"),
           saved != "qwen2.5:3b", saved != Self.preferredRouterModels[0] {
            routerModelAuto = false
        } else {
            routerModelAuto = true
        }
    }

    var setupNeed: SetupNeed? {
        let dismissed = Set(dismissedSetupCards.split(separator: ",").map(String.init))
        func visible(_ need: SetupNeed) -> Bool { !dismissed.contains(need.id) }

        if !engine.agents.isEmpty && !engine.agents.contains(where: \.ready),
           let missing = engine.agents.first(where: { !$0.ready }) {
            let need = SetupNeed.agent(missing)
            if visible(need) { return need }
        }
        if autoRoute {
            if let ot = engine.localModels.first(where: { $0.id.contains("openthai") }),
               !ot.isInstalled {
                let need = SetupNeed.localRouterModel(ot.spec)
                if visible(need) { return need }
            }
        }
        if !workspace.isEmpty && !suggestionsPaused {
            if engine.catalog.isEmpty, engine.catalogError != nil {
                let need = SetupNeed.catalogUnavailable
                if visible(need) { return need }
            }
            let order = ["serena"]
            for id in order {
                if let entry = engine.catalog.first(where: { $0.id == id && (!$0.isInstalled || ($0.isHealthy == true && !$0.isEnabledForProject)) }) {
                    let need = SetupNeed.integration(entry)
                    if visible(need) { return need }
                }
            }
        }
        if let missing = engine.agents.first(where: { !$0.ready }) {
            let need = SetupNeed.agent(missing)
            if visible(need) { return need }
        }
        return nil
    }

    func dismissSetup(_ need: SetupNeed) {
        var ids = Set(dismissedSetupCards.split(separator: ",").map(String.init))
        ids.insert(need.id)
        dismissedSetupCards = ids.sorted().joined(separator: ",")
        if case .integration = need { suggestionsPaused = true }
    }

    func integrationDescription(_ id: String) -> String {
        switch id {
        case "serena": "ช่วย agent ค้นหาและแก้โค้ดในโปรเจกต์"
        default: "เพิ่มความสามารถให้ agent ในโปรเจกต์นี้"
        }
    }

    var ollamaAppURL: URL? {
        let roots = [URL(fileURLWithPath: "/Applications/Ollama.app"),
                     FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Applications/Ollama.app")]
        return roots.first { FileManager.default.fileExists(atPath: $0.path) }
    }

    var ollamaCLIURL: URL? {
        let inherited = (ProcessInfo.processInfo.environment["PATH"] ?? "")
            .split(separator: ":").map(String.init)
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        let folders = inherited + ["/opt/homebrew/bin", "/usr/local/bin", "\(home)/.local/bin"]
        return folders.lazy.map { URL(fileURLWithPath: $0).appendingPathComponent("ollama") }
            .first { FileManager.default.isExecutableFile(atPath: $0.path) }
    }

    @ViewBuilder
    var setupCard: some View {
        if let need = setupNeed {
            switch need {
            case .agent(let agent):
                SetupPromptCard(
                    title: "\(agent.label) ยังไม่พร้อมใช้",
                    detail: agent.auth == "needs_login"
                        ? "เปิด Terminal เพื่อเข้าสู่ระบบ แล้ว Chorus จะตรวจสถานะให้อีกครั้ง"
                        : "เริ่มคำสั่งติดตั้งใน Terminal แล้วกลับมาที่ Chorus เพื่อตรวจสถานะ",
                    symbol: "person.crop.circle.badge.exclamationmark",
                    actionTitle: agent.auth == "needs_login" ? "เริ่มเข้าสู่ระบบ" : "เริ่มติดตั้ง",
                    onAction: { engine.runLoginCommand(agent.hint) },
                    onDismiss: { dismissSetup(need) },
                    secondaryTitle: "ดูสถานะ",
                    onSecondary: { settingsSection = .agents; showSettings = true })
            case .localRouterModel(let spec):
                SetupPromptCard(
                    title: "ติดตั้ง AI Auto Router (คลิกเดียว)",
                    detail: engine.localModelError ?? (engine.isDownloadingLocalModel
                        ? (engine.localModelDownloadStatusText.isEmpty ? "กำลังดาวน์โหลด \(spec.display_name)..." : engine.localModelDownloadStatusText)
                        : "ติดตั้ง \(spec.display_name) (\(ByteCountFormatter.string(fromByteCount: spec.expected_size_bytes, countStyle: .file))) สำหรับเลือกโมเดลในเครื่องอัตโนมัติ ไม่ต้องพึ่งพา Ollama"),
                    symbol: "sparkles",
                    actionTitle: engine.isDownloadingLocalModel ? "กำลังดาวน์โหลด..." : "ติดตั้ง \(spec.display_name)",
                    working: engine.isDownloadingLocalModel,
                    onAction: {
                        if !engine.isDownloadingLocalModel {
                            engine.downloadLocalModel(spec)
                        }
                    },
                    onDismiss: { dismissSetup(need) },
                    secondaryTitle: "ตั้งค่าโมเดล",
                    onSecondary: { settingsSection = .storage; showSettings = true })
            case .catalogUnavailable:
                SetupPromptCard(
                    title: "ยังตรวจเครื่องมือเสริมไม่ได้",
                    detail: engine.catalogError ?? "ลองโหลดรายการเครื่องมืออีกครั้ง",
                    symbol: "puzzlepiece.extension",
                    actionTitle: "ตรวจใหม่",
                    onAction: { engine.loadCatalog(project: workspace) },
                    onDismiss: { dismissSetup(need) },
                    secondaryTitle: "เปิดเครื่องมือเสริม",
                    onSecondary: { settingsSection = .integrations; showSettings = true })
            case .integration(let entry):
                let missing = entry.isInstalled ? nil : entry.missingInstaller
                SetupPromptCard(
                    title: "แนะนำ \(entry.title) สำหรับโปรเจกต์นี้",
                    detail: engine.catalogActionError ?? (missing.map {
                        "ต้องติดตั้ง \($0) ก่อนเพิ่มเครื่องมือนี้"
                    } ?? (entry.isInstalled
                        ? "ติดตั้งแล้ว แต่ยังไม่ได้เปิดใช้กับโปรเจกต์นี้"
                        : "ยังไม่ได้เพิ่มผ่าน Chorus · \(integrationDescription(entry.id))")),
                    symbol: "puzzlepiece.extension",
                    actionTitle: missing.map { "วิธีติดตั้ง \($0)" }
                        ?? (entry.isInstalled ? "เปิดใช้" : "ติดตั้งและเปิดใช้"),
                    working: entry.isWorking,
                    onAction: {
                        if missing != nil, let url = entry.installerURL { NSWorkspace.shared.open(url) }
                        else if entry.isInstalled {
                            engine.catalogAction(project: workspace, id: entry.id, action: "enable")
                        } else {
                            // Show what will be created before anything is installed.
                            engine.requestedInstall = entry.id
                            settingsSection = .integrations
                            showSettings = true
                        }
                    },
                    onDismiss: { dismissSetup(need) },
                    secondaryTitle: "ดูรายละเอียด",
                    onSecondary: { settingsSection = .integrations; showSettings = true })
            }
        }
    }
}
