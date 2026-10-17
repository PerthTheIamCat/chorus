import Combine
import SwiftUI

/// Every app shortcut, in one place. Each key combo has exactly one meaning
/// and lives on a menu item, so the menu bar doubles as the shortcut list.
/// Views subscribe to `AppCommandCenter.shared.fire` instead of registering
/// their own `.keyboardShortcut` — two registrations of one combo is how
/// ⇧⌘E, ⌘↩ and Esc ended up meaning two things at once.
///
/// Esc is deliberately *not* a menu equivalent: a menu item would swallow
/// it from sheets and the composer's completion popup. Its one meaning is
/// "close the open file" (code viewer), registered on that view's button.
enum AppCommand: Equatable {
    case newSession
    case settings
    case commandPalette
    case searchSessions
    case toggleSidebar
    case toggleExplorer
    case toggleInspector
    case inspectorTab(Int)
    case toggleOrchestrator
    case toggleLivePreview
    case toggleTerminal
    case toggleFocusMode
    case send
    case stop
    case chooseModel
    case nextEffort
    case sendReview
    case openInEditor

    /// Every command, for the uniqueness test and anyone listing shortcuts.
    static var all: [AppCommand] {
        [.newSession, .settings, .commandPalette, .searchSessions,
         .toggleSidebar, .toggleExplorer, .toggleInspector]
            + ContentView.RightTab.allCases.indices.map { .inspectorTab($0) }
            + [.toggleOrchestrator, .toggleLivePreview, .toggleTerminal, .toggleFocusMode,
               .send, .stop, .chooseModel, .nextEffort, .sendReview, .openInEditor]
    }

    /// The one key combo for each command. The menu reads this table, so a
    /// combo cannot be bound twice without the test noticing.
    var shortcut: KeyboardShortcut {
        switch self {
        case .newSession: KeyboardShortcut("n", modifiers: .command)
        case .settings: KeyboardShortcut(",", modifiers: .command)
        case .commandPalette: KeyboardShortcut("k", modifiers: .command)
        // ⇧⌘F, not ⌘F: plain ⌘F stays Find inside text views.
        case .searchSessions: KeyboardShortcut("f", modifiers: [.command, .shift])
        case .toggleSidebar: KeyboardShortcut("s", modifiers: [.command, .control])
        case .toggleExplorer: KeyboardShortcut("b", modifiers: .command)
        case .toggleInspector: KeyboardShortcut("i", modifiers: [.command, .option])
        case .inspectorTab(let i): KeyboardShortcut(KeyEquivalent(Character("\(i + 1)")), modifiers: [.command, .option])
        case .toggleOrchestrator: KeyboardShortcut("o", modifiers: [.command, .option])
        case .toggleLivePreview: KeyboardShortcut("p", modifiers: [.command, .option])
        // ⌃` (not ⌘`): ⌘` is the system's "cycle windows".
        case .toggleTerminal: KeyboardShortcut("`", modifiers: .control)
        case .toggleFocusMode: KeyboardShortcut("m", modifiers: [.command, .shift])
        case .send: KeyboardShortcut(.return, modifiers: .command)
        case .stop: KeyboardShortcut(".", modifiers: .command)
        case .chooseModel: KeyboardShortcut("i", modifiers: [.command, .shift])
        case .nextEffort: KeyboardShortcut("e", modifiers: [.command, .shift])
        case .sendReview: KeyboardShortcut(.return, modifiers: [.command, .shift])
        case .openInEditor: KeyboardShortcut("e", modifiers: [.command, .option])
        }
    }
}


/// Bridge between the menu bar (a `Commands`, outside the view tree) and the
/// window's views. ContentView mirrors the few flags the menu needs to grey
/// items out; it holds no logic of its own.
@MainActor
final class AppCommandCenter: ObservableObject {
    static let shared = AppCommandCenter()

    @Published var hasSession = false
    @Published var busy = false
    @Published var fileOpen = false
    @Published var reviewVisible = false
    @Published var focusMode = false

    let fire = PassthroughSubject<AppCommand, Never>()

    func send(_ command: AppCommand) { fire.send(command) }
}

struct ChorusCommands: Commands {
    @ObservedObject var center: AppCommandCenter

    var body: some Commands {
        CommandGroup(replacing: .appSettings) {
            Button("Settings…") { center.send(.settings) }
                .keyboardShortcut(AppCommand.settings.shortcut)
        }

        CommandGroup(replacing: .newItem) {
            Button("New Session") { center.send(.newSession) }
                .keyboardShortcut(AppCommand.newSession.shortcut)
        }

        CommandGroup(after: .textEditing) {
            Divider()
            Button("Search All Sessions…") { center.send(.searchSessions) }
                .keyboardShortcut(AppCommand.searchSessions.shortcut)
        }

        CommandGroup(before: .toolbar) {
            Button("Command Palette…") { center.send(.commandPalette) }
                .keyboardShortcut(AppCommand.commandPalette.shortcut)
            Divider()
            Button("Toggle Projects Sidebar") { center.send(.toggleSidebar) }
                .keyboardShortcut(AppCommand.toggleSidebar.shortcut)
            Button("Toggle Files") { center.send(.toggleExplorer) }
                .keyboardShortcut(AppCommand.toggleExplorer.shortcut)
            Button("Toggle Inspector") { center.send(.toggleInspector) }
                .keyboardShortcut(AppCommand.toggleInspector.shortcut)
            Menu("Inspector Tab") {
                ForEach(Array(ContentView.RightTab.allCases.enumerated()), id: \.offset) { i, tab in
                    Button(tab.rawValue) { center.send(.inspectorTab(i)) }
                        .keyboardShortcut(AppCommand.inspectorTab(i).shortcut)
                }
            }
            Divider()
            Button("Orchestrator") { center.send(.toggleOrchestrator) }
                .keyboardShortcut(AppCommand.toggleOrchestrator.shortcut)
            Button("Live Preview") { center.send(.toggleLivePreview) }
                .keyboardShortcut(AppCommand.toggleLivePreview.shortcut)
            Button("Toggle Terminal") { center.send(.toggleTerminal) }
                .keyboardShortcut(AppCommand.toggleTerminal.shortcut)
            Button(center.focusMode ? "Exit Focus Mode" : "Focus Mode") { center.send(.toggleFocusMode) }
                .keyboardShortcut(AppCommand.toggleFocusMode.shortcut)
            Divider()
        }

        CommandMenu("Chat") {
            Button("Send") { center.send(.send) }
                .keyboardShortcut(AppCommand.send.shortcut)
                .disabled(!center.hasSession)
            Button("Stop Turn") { center.send(.stop) }
                .keyboardShortcut(AppCommand.stop.shortcut)
                .disabled(!center.busy)
            Divider()
            Button("Choose Agent and Model…") { center.send(.chooseModel) }
                .keyboardShortcut(AppCommand.chooseModel.shortcut)
            Button("Next Thinking Level") { center.send(.nextEffort) }
                .keyboardShortcut(AppCommand.nextEffort.shortcut)
            Divider()
            Button("Send Review Comments to Agent") { center.send(.sendReview) }
                .keyboardShortcut(AppCommand.sendReview.shortcut)
                .disabled(!center.reviewVisible)
            Button("Open File in External Editor") { center.send(.openInEditor) }
                .keyboardShortcut(AppCommand.openInEditor.shortcut)
                .disabled(!center.fileOpen)
        }
    }
}
