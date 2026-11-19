import SwiftUI

// What an agent puts in front of the person mid-turn: a permission request, a
// question, versions to choose from. Cards above the composer, not windows, so
// the conversation stays visible while deciding. A credential is the one
// exception (a sheet): its value must never sit in the transcript.

struct PromptStack: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel

    var body: some View {
        VStack(spacing: 8) {
            if let ask = engine.pendingAsk { PermissionCard(ask: ask) }
            if let q = engine.pendingQuestion { QuestionCard(question: q) }
            if let v = engine.pendingVariants { VariantsCard(offer: v) }
        }
    }
}

private struct Card<Content: View>: View {
    let title: String
    let symbol: String
    let tint: Color
    @ViewBuilder var content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Label(title, systemImage: symbol).font(.callout.weight(.semibold)).foregroundStyle(tint)
            content
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(tint.opacity(0.08), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).strokeBorder(tint.opacity(0.25)))
    }
}

private struct PermissionCard: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel
    let ask: Engine.PermissionAsk

    var body: some View {
        Card(title: "\(shell.agentLabel(ask.agent)) ขออนุญาต", symbol: "hand.raised.fill", tint: .orange) {
            Text(ask.tool).font(.callout.weight(.medium)).textSelection(.enabled)
            if !ask.detail.isEmpty {
                Text(ask.detail).font(.caption.monospaced()).foregroundStyle(.secondary)
                    .lineLimit(6).textSelection(.enabled)
            }
            if let why = ask.tainted {
                Label("\(why) ข้อความที่อ่านมาอาจแอบสั่ง agent", systemImage: "exclamationmark.shield")
                    .font(.caption).foregroundStyle(.orange)
            }
            HStack {
                Button("ไม่อนุญาต", role: .destructive) { engine.answerPermission(.deny) }
                    .keyboardShortcut(.cancelAction)
                Spacer()
                Button("อนุญาตตลอด session นี้") { engine.answerPermission(.session) }
                    .help("ไม่ถามซ้ำกับคำขอเดียวกันจนกว่าจะปิด session คำสั่งใหม่ยังถาม")
                Button("อนุญาตครั้งนี้") { engine.answerPermission(.once) }
                    .buttonStyle(.borderedProminent)
                    .keyboardShortcut(.defaultAction)
            }
            .controlSize(.small)
        }
    }
}

private struct QuestionCard: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel
    let question: Engine.AgentQuestion
    @State private var other = ""

    var body: some View {
        Card(title: "\(shell.agentLabel(question.agent)) ถามคุณ", symbol: "questionmark.bubble.fill", tint: .accentColor) {
            Text(question.question).fixedSize(horizontal: false, vertical: true).textSelection(.enabled)
            if !question.reason.isEmpty {
                Text(question.reason).font(.caption).foregroundStyle(.secondary)
            }
            ForEach(Array(question.options.enumerated()), id: \.offset) { i, option in
                Button { engine.answerQuestion(option) } label: {
                    HStack {
                        Text(option).multilineTextAlignment(.leading)
                        Spacer()
                        if question.recommended == i { Text("แนะนำ").font(.caption).foregroundStyle(.secondary) }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .buttonStyle(.bordered)
            }
            if question.allowOther || question.options.isEmpty {
                HStack {
                    TextField("พิมพ์คำตอบ", text: $other).textFieldStyle(.roundedBorder).onSubmit(send)
                    Button("ส่ง", action: send).disabled(other.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            }
            HStack {
                Spacer()
                Button("ให้ agent ตัดสินใจเอง") { engine.answerQuestion(nil) }
                    .buttonStyle(.link)
                    .help("ปิดโดยไม่ตอบ agent จะเลือกเองและบอกว่าสมมติอะไรไว้")
            }
        }
    }

    private func send() {
        let t = other.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !t.isEmpty else { return }
        engine.answerQuestion(t)
    }
}

private struct VariantsCard: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel
    let offer: VariantOffer
    @State private var note = ""

    var body: some View {
        Card(title: "\(shell.agentLabel(offer.agent)) ให้เลือกแบบ", symbol: "rectangle.on.rectangle.angled", tint: .orange) {
            Text(offer.title).fixedSize(horizontal: false, vertical: true)
            ForEach(Array(offer.variants.enumerated()), id: \.offset) { i, v in
                Button {
                    engine.answerVariants(VariantChoice(chosen: i, comments: [], note: note))
                } label: {
                    VStack(alignment: .leading, spacing: 2) {
                        HStack {
                            Text(v.label).fontWeight(.medium)
                            if offer.recommended == i { Text("แนะนำ").font(.caption).foregroundStyle(.secondary) }
                        }
                        if !v.note.isEmpty { Text(v.note).font(.caption).foregroundStyle(.secondary) }
                        Text(v.target).font(.caption2.monospaced()).foregroundStyle(.tertiary).lineLimit(1)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .buttonStyle(.bordered)
            }
            TextField("ข้อความถึง agent (ไม่บังคับ)", text: $note).textFieldStyle(.roundedBorder)
            HStack {
                Button("ไม่เอาสักแบบ") { engine.answerVariants(VariantChoice(chosen: nil, comments: [], note: note)) }
                Spacer()
                Button("ปิด") { engine.answerVariants(nil) }.buttonStyle(.link)
                    .help("ปิดโดยไม่เลือก agent จะเลือกเองและบอกว่าสมมติอะไรไว้")
            }
        }
    }
}

/// A credential the agent asked for. The value goes to Rust and the Keychain
/// only: never into the transcript, never into a log.
struct SecretSheet: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel
    let request: SecretRequest

    @State private var value = ""
    @State private var remember: SecretRemember = .project
    @FocusState private var focused: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Label("\(shell.agentLabel(request.agent)) ขอ \(request.name)", systemImage: "key.fill").font(.title3.bold())
            Text(request.reason).fixedSize(horizontal: false, vertical: true)
            SecureField("ค่าของ \(request.name)", text: $value)
                .textFieldStyle(.roundedBorder)
                .focused($focused)
                .onSubmit(save)
            Picker("เก็บไว้", selection: $remember) {
                ForEach(SecretRemember.allCases) { r in Text(r.label).tag(r) }
            }
            .pickerStyle(.segmented)
            Text(remember.detail).font(.caption).foregroundStyle(.secondary)
            Text("agent จะเห็นแค่ชื่อตัวแปร ไม่เห็นค่า และใช้ได้ตั้งแต่เทิร์นถัดไป")
                .font(.caption).foregroundStyle(.secondary)
            HStack {
                Button("ไม่ให้") { engine.answerSecret(nil) }.keyboardShortcut(.cancelAction)
                Spacer()
                Button("บันทึก", action: save).keyboardShortcut(.defaultAction).disabled(value.isEmpty)
            }
        }
        .padding(20)
        .frame(width: 460)
        .interactiveDismissDisabled()
        .onAppear { focused = true }
    }

    private func save() {
        guard !value.isEmpty else { return }
        engine.answerSecret(SecretAnswer(value: value, remember: remember))
    }
}
