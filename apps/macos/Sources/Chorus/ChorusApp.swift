import AppKit
import CChorus
import SwiftUI
import UserNotifications

/// Closing the last window quits the app, and quitting reaps the agent tree.
/// Without this the app lingers windowless and the bridges it spawned
/// (claude-agent-acp, serena, chrome-devtools-mcp) outlive it.
final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        if Bundle.main.bundleIdentifier != nil {
            UNUserNotificationCenter.current().delegate = NotificationClickHandler.shared
        }
    }
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }
    func applicationWillTerminate(_ notification: Notification) { chorus_shutdown() }
}

@main
struct ChorusApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @StateObject private var engine: Engine
    @StateObject private var shell: ShellModel
    @StateObject private var updater = Updater.shared

    init() {
        let e = Engine()
        _engine = StateObject(wrappedValue: e)
        _shell = StateObject(wrappedValue: ShellModel(engine: e))
    }

    var body: some Scene {
        WindowGroup("Chorus") {
            // A bare `swift build` executable has no app bundle, so launching it
            // can leave the window on screen but never made key; ShellRoot
            // forces activation when it appears.
            RootView()
                .environmentObject(engine)
                .environmentObject(shell)
                .chorusAppearance()
        }
        // Not `.hiddenTitleBar`: with a NavigationSplitView and a toolbar it
        // shifts hit-testing away from what is drawn, so sidebar clicks landed
        // on the row next to the one under the pointer.
        .windowToolbarStyle(.unified)
        .defaultSize(width: 1440, height: 900)
        .commands {
            AboutCommand()
            CheckForUpdatesCommand(updater: updater)
            ChorusCommands(center: AppCommandCenter.shared)
        }

        // ⌘, and the sidebar gear open the standard Settings window.
        Settings {
            SettingsRoot()
                .environmentObject(engine)
                .environmentObject(shell)
                .chorusAppearance()
        }
    }
}
