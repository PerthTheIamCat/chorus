import SwiftUI

/// An agent's `chorus_ask_user`: the options as buttons, a field for a typed
/// answer when the agent accepts one. Drawing only — validation, masking and
/// what the agent is told on "ข้าม" all live in Rust (`ask_user.rs`).
struct AgentQuestionSheet: View {
    let question: Engine.AgentQuestion
    let agentLabel: String
    /// `nil` = closed without an answer.
    let answer: (String?) -> Void

    @State private var typed = ""
    @FocusState private var fieldFocused: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Label("\(agentLabel) ถาม", systemImage: "questionmark.bubble")
                .font(.title3.bold())
            Text(question.question)
                .font(.body)
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)

            if !question.options.isEmpty {
                VStack(alignment: .leading, spacing: 6) {
                    ForEach(Array(question.options.enumerated()), id: \.offset) { i, option in
                        Button { answer(option) } label: {
                            HStack(alignment: .firstTextBaseline, spacing: 8) {
                                Text("\(i + 1)").font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                                Text(option).multilineTextAlignment(.leading)
                                Spacer(minLength: 0)
                            }
                            .frame(maxWidth: .infinity, alignment: .leading)
                        }
                        .buttonStyle(InteractiveStyle())
                        // Rust caps options at 8, so ⌘1…⌘8 always exist.
                        .keyboardShortcut(KeyEquivalent(Character("\(i + 1)")), modifiers: .command)
                    }
                }
            }

            if question.allowOther {
                TextField(question.options.isEmpty ? "พิมพ์คำตอบ" : "หรือพิมพ์คำตอบเอง",
                          text: $typed, axis: .vertical)
                    .lineLimit(1...5)
                    .textFieldStyle(.roundedBorder)
                    .focused($fieldFocused)
                    .onSubmit(send)
            }

            HStack {
                Button("ข้าม") { answer(nil) }
                    .keyboardShortcut(.cancelAction)
                    .help("ปิดโดยไม่ตอบ — agent จะตัดสินใจเองและบอกว่าสมมติอะไรไว้")
                Spacer()
                if question.allowOther {
                    Button("ส่งคำตอบ", action: send)
                        .keyboardShortcut(.defaultAction)
                        .disabled(typed.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }
        }
        .padding(20)
        .frame(width: 480)
        .interactiveDismissDisabled()
        .onAppear { fieldFocused = question.options.isEmpty && question.allowOther }
    }

    private func send() {
        let t = typed.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !t.isEmpty else { return }
        answer(t)
    }
}
