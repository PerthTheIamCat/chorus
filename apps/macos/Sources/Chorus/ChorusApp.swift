import AppKit
import CChorus
import SwiftUI

/// Closing the last window quits the app, and quitting reaps the agent tree.
/// Without this the app lingers windowless and the bridges it spawned
/// (claude-agent-acp, serena, graft) outlive it.
final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }
    func applicationWillTerminate(_ notification: Notification) { chorus_shutdown() }
}

@main
struct ChorusApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @StateObject private var engine = Engine()

    var body: some Scene {
        WindowGroup("Chorus") {
            ContentView()
                .environmentObject(engine)
                .frame(minWidth: 820, minHeight: 560)
                .onAppear {
                    // A bare `swift build` executable has no app bundle, so
                    // launching it (especially from a background/detached
                    // process, not a foreground double-click) can leave the
                    // window on screen but never made key — every field
                    // then silently refuses keyboard input. Force it.
                    NSApp.setActivationPolicy(.regular)
                    NSApp.activate(ignoringOtherApps: true)
                    NSApp.windows.first?.makeKeyAndOrderFront(nil)
                }
        }
        // Not `.hiddenTitleBar`: with a NavigationSplitView and a toolbar it
        // shifts hit-testing away from what is drawn, so sidebar clicks landed
        // on the row next to the one under the pointer.
        .windowToolbarStyle(.unified)
    }
}
