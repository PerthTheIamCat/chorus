import SwiftUI

// Things an agent waits on, drawn inline at the end of the thread where it
// asked (design: amber "Codex ขอรันคำสั่ง" card) — not modal windows. ↩ and esc
// answer the permission card. A secret is the one exception: it opens a sheet
// with a SecureField so the value never sits in the thread.

struct PromptStack: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel

    var body: some View {
        VStack(spacing: 12) {
            if let ask = engine.pendingAsk { PermissionCard(ask: ask) }
            if let q = engine.pendingQuestion { QuestionCard(question: q) }
            if let v = engine.pendingVariants { VariantsCard(offer: v) }
            if let agents = engine.pendingPlanAgents, !engine.busy { PlanCard(agents: agents) }
        }
        .padding(.leading, Gutter.text)
    }
}

/// The amber frame every waiting card shares.
private struct WaitingCard<Content: View>: View {
    let title: String
    let symbol: String
    var tone: Color = Palette.warn
    var wash: Color = Palette.warnWash
    var trailing: String?
    @ViewBuilder var content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Image(systemName: symbol).foregroundStyle(tone)
                Text(title).font(.system(size: 13, weight: .semibold)).foregroundStyle(Palette.text)
                Spacer()
                if let trailing { Text(trailing).font(Typo.tiny).foregroundStyle(Palette.text3) }
            }
            content
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(wash, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).strokeBorder(tone.opacity(0.7)))
    }
}

private struct PermissionCard: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel
    let ask: Engine.PermissionAsk

    var body: some View {
        WaitingCard(title: "\(shell.agentLabel(ask.agent)) ขอ\(ask.tool.isEmpty ? "อนุญาต" : "รันคำสั่ง")",
                    symbol: "exclamationmark.triangle",
                    trailing: "\(shell.mode.label) · Seatbelt เปิด") {
            Text(ask.detail.isEmpty ? ask.tool : ask.detail)
                .font(Typo.mono).foregroundStyle(Palette.text)
                .lineLimit(8).textSelection(.enabled)
                .padding(.horizontal, 10).padding(.vertical, 8)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(Palette.window, in: RoundedRectangle(cornerRadius: 7, style: .continuous))
                .overlay(RoundedRectangle(cornerRadius: 7, style: .continuous).strokeBorder(Palette.line))
            if !ask.detail.isEmpty && !ask.tool.isEmpty {
                Text(ask.tool).font(Typo.caption).foregroundStyle(Palette.text3)
            }
            if let why = ask.tainted {
                Label("\(why) ข้อความที่อ่านมาอาจแอบสั่ง agent", systemImage: "exclamationmark.shield")
                    .font(Typo.caption).foregroundStyle(Palette.warn)
            }
            HStack(spacing: 8) {
                Button { engine.answerPermission(.once) } label: {
                    HStack(spacing: 6) { Text("อนุญาตครั้งนี้"); Text("↩").opacity(0.6) }
                }
                .buttonStyle(PrimaryButtonStyle())
                .keyboardShortcut(.defaultAction)
                Button("อนุญาตตลอด session นี้") { engine.answerPermission(.session) }
                    .buttonStyle(SecondaryButtonStyle())
                    .help("ไม่ถามซ้ำกับคำขอเดียวกันจนปิด session คำสั่งใหม่ยังถาม")
                Spacer()
                Button { engine.answerPermission(.deny) } label: {
                    HStack(spacing: 6) { Text("ปฏิเสธ"); Text("esc").opacity(0.6) }
                }
                .buttonStyle(QuietButtonStyle())
                .keyboardShortcut(.cancelAction)
            }
        }
    }
}

private struct QuestionCard: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel
    let question: Engine.AgentQuestion
    @State private var other = ""

    var body: some View {
        WaitingCard(title: "\(shell.agentLabel(question.agent)) ถามคุณ", symbol: "questionmark.bubble",
                    tone: Palette.accent, wash: Palette.accentWash) {
            Text(question.question).font(Typo.body).foregroundStyle(Palette.text)
                .fixedSize(horizontal: false, vertical: true).textSelection(.enabled)
            if !question.reason.isEmpty {
                Text(question.reason).font(Typo.caption).foregroundStyle(Palette.text3)
            }
            ForEach(Array(question.options.enumerated()), id: \.offset) { i, option in
                Button { engine.answerQuestion(option) } label: {
                    HStack {
                        Text(option).multilineTextAlignment(.leading)
                        Spacer()
                        if question.recommended == i { Tag(text: "แนะนำ", tone: .accent) }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .buttonStyle(SecondaryButtonStyle())
            }
            if question.allowOther || question.options.isEmpty {
                HStack {
                    TextField("พิมพ์คำตอบ", text: $other).textFieldStyle(.roundedBorder).onSubmit(send)
                    Button("ส่ง", action: send).buttonStyle(PrimaryButtonStyle(small: true))
                        .disabled(other.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            }
            HStack {
                Spacer()
                Button("ให้ agent ตัดสินใจเอง") { engine.answerQuestion(nil) }
                    .buttonStyle(QuietButtonStyle(small: true))
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
        WaitingCard(title: "\(shell.agentLabel(offer.agent)) ให้เลือกแบบ", symbol: "rectangle.on.rectangle.angled",
                    tone: Palette.accent, wash: Palette.accentWash) {
            Text(offer.title).font(Typo.body).fixedSize(horizontal: false, vertical: true)
            ForEach(Array(offer.variants.enumerated()), id: \.offset) { i, v in
                Button { engine.answerVariants(VariantChoice(chosen: i, comments: [], note: note)) } label: {
                    VStack(alignment: .leading, spacing: 2) {
                        HStack {
                            Text(v.label).fontWeight(.medium)
                            if offer.recommended == i { Tag(text: "แนะนำ", tone: .accent) }
                        }
                        if !v.note.isEmpty { Text(v.note).font(Typo.caption).foregroundStyle(Palette.text3) }
                        Text(v.target).font(Typo.monoSmall).foregroundStyle(Palette.text3).lineLimit(1)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.vertical, 6)
                }
                .buttonStyle(SecondaryButtonStyle())
            }
            TextField("ข้อความถึง agent (ไม่บังคับ)", text: $note).textFieldStyle(.roundedBorder)
            HStack {
                Button("ไม่เอาสักแบบ") { engine.answerVariants(VariantChoice(chosen: nil, comments: [], note: note)) }
                    .buttonStyle(SecondaryButtonStyle(small: true))
                Spacer()
                Button("ปิด") { engine.answerVariants(nil) }.buttonStyle(QuietButtonStyle(small: true))
            }
        }
    }
}

/// Plan mode finished: approve, pick who does it, or type changes.
private struct PlanCard: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel
    let agents: String

    var body: some View {
        WaitingCard(title: "แผนพร้อมแล้ว", symbol: "list.bullet.clipboard", tone: Palette.accent, wash: Palette.accentWash,
                    trailing: "อ่านอย่างเดียว · ไม่มีไฟล์ถูกแก้") {
            Text("อนุมัติแล้วข้อความ \"แผนผ่าน\" ลง ledger — agent ตัวถัดไปรู้ว่าแผนนี้ถูกอนุมัติแล้ว")
                .font(Typo.caption).foregroundStyle(Palette.text3)
            HStack(spacing: 8) {
                Button("ไม่เอา") { engine.pendingPlanAgents = nil }.buttonStyle(SecondaryButtonStyle())
                Spacer()
                Text("หรือพิมพ์บอกว่าจะแก้ตรงไหน").font(Typo.caption).foregroundStyle(Palette.text3)
                Button {
                    shell.approvePlan(agents: agents)
                } label: {
                    HStack(spacing: 6) { Image(systemName: "play.fill").font(.system(size: 9)); Text("เริ่มทำตามแผน") }
                }
                .buttonStyle(PrimaryButtonStyle())
                .keyboardShortcut(.return, modifiers: .command)
            }
        }
    }
}

struct SecretSheet: View {
    @EnvironmentObject private var engine: Engine
    @EnvironmentObject private var shell: ShellModel
    let request: SecretRequest
    @State private var value = ""
    @State private var remember: SecretRemember = .project
    @FocusState private var focused: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            SheetHeader(title: "\(shell.agentLabel(request.agent)) ขอ \(request.name)",
                        subtitle: "agent จะเห็นแค่ชื่อตัวแปร ไม่เห็นค่า · ใช้ได้ตั้งแต่เทิร์นถัดไป") { engine.answerSecret(nil) }
            VStack(alignment: .leading, spacing: 14) {
                Text(request.reason).font(Typo.body).fixedSize(horizontal: false, vertical: true)
                SecureField("ค่าของ \(request.name)", text: $value)
                    .textFieldStyle(.roundedBorder).focused($focused).onSubmit(save)
                Picker("เก็บไว้", selection: $remember) {
                    ForEach(SecretRemember.allCases) { r in Text(r.label).tag(r) }
                }
                .pickerStyle(.segmented)
                Text(remember.detail).font(Typo.caption).foregroundStyle(Palette.text3)
            }
            .padding(.horizontal, 20)
            HStack {
                Spacer()
                Button("ไม่ให้") { engine.answerSecret(nil) }.buttonStyle(SecondaryButtonStyle())
                Button("บันทึก", action: save).buttonStyle(PrimaryButtonStyle())
                    .keyboardShortcut(.defaultAction).disabled(value.isEmpty)
            }
            .padding(20)
        }
        .frame(width: 480)
        .background(Palette.elevated)
        .interactiveDismissDisabled()
        .onAppear { focused = true }
    }

    private func save() {
        guard !value.isEmpty else { return }
        engine.answerSecret(SecretAnswer(value: value, remember: remember))
    }
}
