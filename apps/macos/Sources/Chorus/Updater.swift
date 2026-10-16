import Sparkle
import SwiftUI

/// Thin wrapper over Sparkle. No update logic lives here: Sparkle reads the
/// appcast (SUFeedURL in Info.plist), verifies the EdDSA signature
/// (SUPublicEDKey) and installs. We only expose the "Check for Updates…" action.
@MainActor
final class Updater: ObservableObject {
    private let controller = SPUStandardUpdaterController(
        startingUpdater: true, updaterDelegate: nil, userDriverDelegate: nil)
    @Published private(set) var canCheck = false

    init() {
        controller.updater.publisher(for: \.canCheckForUpdates).assign(to: &$canCheck)
    }

    func checkForUpdates() { controller.checkForUpdates(nil) }
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
