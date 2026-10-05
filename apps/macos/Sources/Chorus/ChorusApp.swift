import AppKit
import SwiftUI

@main
struct ChorusApp: App {
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
        .windowStyle(.hiddenTitleBar)
    }
}
