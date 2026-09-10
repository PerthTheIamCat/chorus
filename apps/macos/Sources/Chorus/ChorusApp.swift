import SwiftUI

@main
struct ChorusApp: App {
    @StateObject private var engine = Engine()

    var body: some Scene {
        WindowGroup("Chorus") {
            ContentView()
                .environmentObject(engine)
                .frame(minWidth: 820, minHeight: 560)
        }
        .windowStyle(.hiddenTitleBar)
    }
}
