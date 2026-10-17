import Sparkle
import SwiftUI

/// Thin wrapper over Sparkle. No update logic lives here: Sparkle reads the
/// appcast (SUFeedURL in Info.plist), verifies the EdDSA signature
/// (SUPublicEDKey) and installs. We only expose the "Check for Updates…" action.
@MainActor
final class Updater: ObservableObject {
    /// One updater for the app: the menu command and Settings › About share it.
    static let shared = Updater()

    private let controller = SPUStandardUpdaterController(
        startingUpdater: true, updaterDelegate: nil, userDriverDelegate: nil)
    @Published private(set) var canCheck = false
    @Published private(set) var lastCheck: Date?

    private init() {
        controller.updater.publisher(for: \.canCheckForUpdates).assign(to: &$canCheck)
        controller.updater.publisher(for: \.lastUpdateCheckDate).assign(to: &$lastCheck)
    }

    func checkForUpdates() { controller.checkForUpdates(nil) }

    /// The bundle's version, `"dev"` for a bare `swift build` binary (no
    /// Info.plist). Never a hardcoded number: that goes stale silently.
    static let appVersion: String = {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "dev"
    }()
}

struct CheckForUpdatesCommand: Commands {
    @ObservedObject var updater: Updater

    var body: some Commands {
        CommandGroup(after: .appInfo) {
            Button("Check for Updates…") { updater.checkForUpdates() }
                .disabled(!updater.canCheck)
        }
    }
}
