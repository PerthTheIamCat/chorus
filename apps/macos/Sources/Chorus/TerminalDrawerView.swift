import SwiftUI
import AppKit

/// Embedded Terminal Drawer & Live PTY Shell (Ticket 13 & 20).
/// Provides an in-app interactive developer terminal (`⌘~`) for live command execution,
/// instant git checks, and agent tool log inspection.
struct TerminalDrawerView: View {
    let workspace: String
    var onClose: () -> Void

    @State private var outputText: String = "Chorus Terminal Shell v\(Updater.appVersion) (zsh)\nWorkspace: "
    @State private var commandInput: String = ""
    @State private var commandHistory: [String] = []
    @State private var historyIndex: Int = -1
    @State private var isRunningCommand: Bool = false
    @State private var activeProcess: Process? = nil

    private let quickPresets: [String] = [
        "git status",
        "git diff --stat",
        "git log -n 5 --oneline",
        "cargo check",
        "ls -la"
    ]

    init(workspace: String, onClose: @escaping () -> Void) {
        self.workspace = workspace
        self.onClose = onClose
    }

    public var body: some View {
        VStack(spacing: 0) {
            // Header Bar
            HStack(spacing: 8) {
                Image(systemName: "terminal.fill")
                    .font(.caption)
                    .foregroundStyle(.teal)

                Text("เทอร์มินัล")
                    .font(.caption.weight(.semibold))

                if !workspace.isEmpty {
                    Text((workspace as NSString).lastPathComponent)
                        .font(.caption2.monospaced())
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(Color.primary.opacity(0.06), in: Capsule())
                }

                Spacer()

                // Quick presets
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 4) {
                        ForEach(quickPresets, id: \.self) { cmd in
                            Button {
                                executeCommand(cmd)
                            } label: {
                                Text(cmd)
                                    .font(.system(size: 10, design: .monospaced))
                                    .padding(.horizontal, 6)
                                    .padding(.vertical, 2)
                            }
                            .buttonStyle(.plain)
                            .background(Color.primary.opacity(0.05), in: RoundedRectangle(cornerRadius: 4))
                            .disabled(isRunningCommand || workspace.isEmpty)
                        }
                    }
                }
                .frame(maxWidth: 380)

                Button {
                    outputText = "Chorus Terminal Shell v\(Updater.appVersion) (zsh)\n"
                } label: {
                    Image(systemName: "trash")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
                .buttonStyle(.plain)
                .help("ล้างหน้าจอ")

                Button {
                    if let proc = activeProcess, proc.isRunning {
                        proc.terminate()
                    }
                    onClose()
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                .buttonStyle(.plain)
                .help("ปิดเทอร์มินัล (⌘~)")
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .background(Color(NSColor.windowBackgroundColor).opacity(0.95))
            .overlay(Divider(), alignment: .bottom)

            // Terminal Screen Output
            ScrollViewReader { proxy in
                ScrollView {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(outputText)
                            .font(.system(.caption, design: .monospaced))
                            .foregroundStyle(Color.primary)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .textSelection(.enabled)

                        Color.clear.frame(height: 1).id("terminalBottom")
                    }
                    .padding(10)
                }
                .background(Color(NSColor.textBackgroundColor).opacity(0.4))
                .onChange(of: outputText) { _, _ in
                    proxy.scrollTo("terminalBottom", anchor: .bottom)
                }
            }

            Divider()

            // Command Input Field
            HStack(spacing: 6) {
                Text("$")
                    .font(.caption.weight(.bold).monospaced())
                    .foregroundStyle(.teal)

                TextField("พิมพ์คำสั่ง shell (เช่น cargo check, git status)...", text: $commandInput)
                    .textFieldStyle(.plain)
                    .font(.system(.caption, design: .monospaced))
                    .onSubmit {
                        let trimmed = commandInput.trimmingCharacters(in: .whitespacesAndNewlines)
                        if !trimmed.isEmpty {
                            executeCommand(trimmed)
                            commandInput = ""
                        }
                    }

                if isRunningCommand {
                    ProgressView()
                        .controlSize(.mini)

                    Button("หยุด (Ctrl+C)") {
                        activeProcess?.terminate()
                    }
                    .font(.caption2)
                    .buttonStyle(.bordered)
                    .controlSize(.mini)
                } else {
                    Button {
                        let trimmed = commandInput.trimmingCharacters(in: .whitespacesAndNewlines)
                        if !trimmed.isEmpty {
                            executeCommand(trimmed)
                            commandInput = ""
                        }
                    } label: {
                        Image(systemName: "return")
                            .font(.caption)
                    }
                    .buttonStyle(.plain)
                    .disabled(commandInput.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || workspace.isEmpty)
                }
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 7)
            .background(Color(NSColor.controlBackgroundColor))
        }
        .frame(minHeight: 180, maxHeight: 320)
        .onAppear {
            if !workspace.isEmpty {
                outputText += "\(workspace)\n"
            }
        }
    }

    private func executeCommand(_ command: String) {
        guard !workspace.isEmpty else {
            outputText += "\n[ข้อผิดพลาด] ยังไม่ได้เปิดโปรเจกต์ใด\n"
            return
        }

        commandHistory.append(command)
        historyIndex = commandHistory.count

        outputText += "\n$ \(command)\n"
        isRunningCommand = true

        DispatchQueue.global(qos: .userInitiated).async {
            let process = Process()
            let pipe = Pipe()

            process.currentDirectoryURL = URL(fileURLWithPath: workspace)
            process.executableURL = URL(fileURLWithPath: "/bin/zsh")
            process.arguments = ["-l", "-c", command]
            process.standardOutput = pipe
            process.standardError = pipe

            // Inherit user environment PATH
            var env = ProcessInfo.processInfo.environment
            let userPaths = "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:~/.cargo/bin"
            let existingPath = env["PATH"] ?? ""
            env["PATH"] = "\(userPaths):\(existingPath)"
            process.environment = env

            DispatchQueue.main.async {
                self.activeProcess = process
            }

            let fileHandle = pipe.fileHandleForReading
            fileHandle.readabilityHandler = { handle in
                let data = handle.availableData
                if !data.isEmpty, let chunk = String(data: data, encoding: .utf8) {
                    DispatchQueue.main.async {
                        self.outputText += self.stripANSI(chunk)
                    }
                }
            }

            do {
                try process.run()
                process.waitUntilExit()
            } catch {
                DispatchQueue.main.async {
                    self.outputText += "\nเกิดข้อผิดพลาดในการรัน: \(error.localizedDescription)\n"
                }
            }

            fileHandle.readabilityHandler = nil

            DispatchQueue.main.async {
                self.isRunningCommand = false
                self.activeProcess = nil
                self.outputText += "\n[คำสั่งเสร็จสิ้น - Exit code \(process.terminationStatus)]\n"
            }
        }
    }

    private func stripANSI(_ text: String) -> String {
        // Strip ANSI escape codes (e.g. \u{001B}[32m) for clean legible display
        text.replacingOccurrences(of: #"\x1b\[[0-9;]*[a-zA-Z]"#, with: "", options: .regularExpression)
    }
}
