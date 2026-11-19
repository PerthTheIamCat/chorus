import AppKit
import SwiftUI

/// Where the agent's questions and choices appear: at the bottom of the chat,
/// where its work stopped. Drawing only — validation, masking and what the
/// agent is told live in Rust (`ask_user.rs`, `variants.rs`).
struct AgentPromptCards: View {
    @ObservedObject var engine: Engine
    let agentLabel: (String) -> String
    /// The card just appeared: keep the bottom of the chat in view.
    let onShow: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            if let q = engine.pendingQuestion {
                InlineQuestionCard(question: q, agentLabel: agentLabel(q.agent)) { engine.answerQuestion($0) }
                    .id(q.id)
            }
            if let offer = engine.pendingVariants {
                VariantChoiceCard(offer: offer, agentLabel: agentLabel(offer.agent)) { engine.answerVariants($0) }
                    .id(offer.id)
            }
        }
        .onAppear(perform: onShow)
        .onChange(of: engine.pendingQuestion?.id) { _, _ in onShow() }
        .onChange(of: engine.pendingVariants?.id) { _, _ in onShow() }
    }
}

private struct PromptCardBackground: ViewModifier {
    func body(content: Content) -> some View {
        content
            .padding(14)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Color.orange.opacity(0.08), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .strokeBorder(Color.orange.opacity(0.35), lineWidth: 1))
    }
}

private extension View {
    func promptCard() -> some View { modifier(PromptCardBackground()) }
}

// MARK: question

/// An agent's `chorus_ask_user`: why it stopped, the options as buttons (its
/// pick marked), a field for a typed answer when it accepts one.
struct InlineQuestionCard: View {
    let question: Engine.AgentQuestion
    let agentLabel: String
    /// `nil` = closed without an answer.
    let answer: (String?) -> Void

    @State private var typed = ""
    @FocusState private var fieldFocused: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Label("\(agentLabel) หยุดรอคุณ", systemImage: "questionmark.bubble.fill")
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(.orange)
            if !question.reason.isEmpty {
                Text(question.reason)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
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
                                if question.recommended == i {
                                    Text("แนะนำ")
                                        .font(.caption2.weight(.semibold))
                                        .foregroundStyle(Color.accentColor)
                                }
                            }
                            .frame(maxWidth: .infinity, alignment: .leading)
                        }
                        .buttonStyle(InteractiveStyle(selected: question.recommended == i, bordered: true))
                        // Rust caps options at 8, so ⌘1…⌘8 always exist.
                        .keyboardShortcut(KeyEquivalent(Character("\(i + 1)")), modifiers: .command)
                    }
                }
            }

            if question.allowOther {
                HStack(alignment: .bottom, spacing: 8) {
                    TextField(question.options.isEmpty ? "พิมพ์คำตอบ" : "หรือพิมพ์คำตอบเอง",
                              text: $typed, axis: .vertical)
                        .lineLimit(1...5)
                        .textFieldStyle(.roundedBorder)
                        .focused($fieldFocused)
                        .onSubmit(send)
                    Button("ส่งคำตอบ", action: send)
                        .disabled(typed.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            }

            HStack {
                Button("ข้าม") { answer(nil) }
                    .help("ปิดโดยไม่ตอบ — agent จะตัดสินใจเองและบอกว่าสมมติอะไรไว้")
                Spacer()
            }
        }
        .promptCard()
        .onAppear { fieldFocused = question.options.isEmpty && question.allowOther }
    }

    private func send() {
        let t = typed.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !t.isEmpty else { return }
        answer(t)
    }
}

// MARK: secret

/// An agent's `chorus_request_secret`. A sheet, not a card: the field is
/// private and the value must not sit in the transcript. It goes to Rust and
/// then the Keychain; the agent is told only the variable name.
struct SecretRequestSheet: View {
    let request: SecretRequest
    let agentLabel: String
    /// `nil` = cancelled.
    let answer: (SecretAnswer?) -> Void

    @State private var value = ""
    @State private var remember: SecretRemember = .project
    @FocusState private var focused: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Label("\(agentLabel) ขอ \(request.name)", systemImage: "key.fill")
                .font(.title3.bold())
            Text(request.reason)
                .font(.body)
                .fixedSize(horizontal: false, vertical: true)

            SecureField("ค่าของ \(request.name)", text: $value)
                .textFieldStyle(.roundedBorder)
                .focused($focused)
                .onSubmit(save)

            VStack(alignment: .leading, spacing: 6) {
                Picker("เก็บไว้", selection: $remember) {
                    ForEach(SecretRemember.allCases) { r in Text(r.label).tag(r) }
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                Text(remember.detail)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            Text("agent จะเห็นแค่ชื่อตัวแปร ไม่เห็นค่า และจะใช้ได้ตั้งแต่เทิร์นถัดไป · ถ้า agent พิมพ์ค่านั้นออกมาเอง ค่าจะไปถึงผู้ให้บริการของ agent")
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

            HStack {
                Button("ไม่ให้") { answer(nil) }
                    .keyboardShortcut(.cancelAction)
                Spacer()
                Button("บันทึก", action: save)
                    .keyboardShortcut(.defaultAction)
                    .disabled(value.isEmpty)
            }
        }
        .padding(20)
        .frame(width: 460)
        .interactiveDismissDisabled()
        .onAppear { focused = true }
    }

    private func save() {
        guard !value.isEmpty else { return }
        answer(SecretAnswer(value: value, remember: remember))
    }
}

// MARK: variants

/// An agent's `chorus_offer_variants`: two to four versions side by side,
/// click an image to pin a comment, pick one (or none).
struct VariantChoiceCard: View {
    let offer: VariantOffer
    let agentLabel: String
    /// `nil` = closed without choosing.
    let answer: (VariantChoice?) -> Void

    @State private var chosen: Int?
    @State private var pins: [VariantPin] = []
    @State private var note = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Label("\(agentLabel) ให้เลือกแบบ", systemImage: "rectangle.on.rectangle.angled")
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(.orange)
            Text(offer.title).font(.body.weight(.medium))

            HStack(alignment: .top, spacing: 12) {
                ForEach(Array(offer.variants.enumerated()), id: \.offset) { i, v in
                    column(i, v)
                }
            }

            if !pins.isEmpty {
                VStack(alignment: .leading, spacing: 6) {
                    ForEach($pins) { $pin in
                        HStack(spacing: 8) {
                            Text("\(number(of: pin))")
                                .font(.caption2.weight(.bold))
                                .foregroundStyle(.white)
                                .frame(width: 18, height: 18)
                                .background(Color.accentColor, in: Circle())
                            Text(offer.variants[pin.variant].label)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                            TextField("คอมเมนต์ตรงจุดนี้", text: $pin.text)
                                .textFieldStyle(.roundedBorder)
                            Button { pins.removeAll { $0.id == pin.id } } label: {
                                Image(systemName: "xmark.circle.fill")
                            }
                            .buttonStyle(.chorusPlain)
                            .help("ลบหมุดนี้")
                        }
                    }
                }
            }

            TextField("ข้อความถึง agent (ไม่บังคับ)", text: $note)
                .textFieldStyle(.roundedBorder)

            HStack {
                Button("ปิด") { answer(nil) }
                    .help("ปิดโดยไม่เลือก — agent จะเลือกเองและบอกว่าสมมติอะไรไว้")
                Spacer()
                Button("ไม่เอาสักแบบ") { send(chosen: nil) }
                    .help("ส่งคอมเมนต์และข้อความ โดยไม่เลือกแบบไหน")
                Button(chosen.map { "เลือก \(offer.variants[$0].label)" } ?? "เลือกแบบหนึ่งก่อน") {
                    send(chosen: chosen)
                }
                .disabled(chosen == nil)
            }
        }
        .promptCard()
    }

    private func number(of pin: VariantPin) -> Int {
        (pins.firstIndex { $0.id == pin.id } ?? 0) + 1
    }

    private func send(chosen: Int?) {
        answer(VariantChoice(chosen: chosen, comments: pins, note: note))
    }

    @ViewBuilder
    private func column(_ i: Int, _ v: VariantOffer.Variant) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 6) {
                Text(v.label).font(.callout.weight(.semibold)).lineLimit(1)
                if offer.recommended == i {
                    Text("แนะนำ")
                        .font(.caption2.weight(.semibold))
                        .foregroundStyle(Color.accentColor)
                }
            }
            if v.kind == "image" {
                VariantImage(path: v.target, pins: pinMarks(for: i)) { x, y in
                    pins.append(VariantPin(variant: i, x: x, y: y))
                }
                .frame(maxHeight: 220)
            } else {
                Button {
                    if let url = URL(string: v.target) { NSWorkspace.shared.open(url) }
                } label: {
                    Label(v.target, systemImage: "safari")
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .buttonStyle(InteractiveStyle(bordered: true))
                .help("เปิดใน browser")
            }
            if !v.note.isEmpty {
                Text(v.note).font(.caption).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Button(chosen == i ? "เลือกอยู่" : "เลือกแบบนี้") { chosen = (chosen == i ? nil : i) }
                .buttonStyle(InteractiveStyle(selected: chosen == i, bordered: true))
        }
        .frame(maxWidth: .infinity, alignment: .topLeading)
    }

    private func pinMarks(for variant: Int) -> [VariantImage.Mark] {
        pins.enumerated().compactMap { n, p in
            guard p.variant == variant, let x = p.x, let y = p.y else { return nil }
            return VariantImage.Mark(number: n + 1, x: x, y: y)
        }
    }
}

/// One image with numbered pins; a click adds a pin at that spot (0…1 of the
/// image, so the agent gets "40% across, 30% down", not screen pixels).
struct VariantImage: View {
    struct Mark: Identifiable {
        let number: Int
        let x: Double
        let y: Double
        var id: Int { number }
    }

    let path: String
    let pins: [Mark]
    let onPin: (Double, Double) -> Void

    @State private var image: NSImage?

    var body: some View {
        Group {
            if let image {
                Image(nsImage: image)
                    .resizable()
                    .aspectRatio(contentMode: .fit)
                    .overlay(
                        GeometryReader { geo in
                            ZStack(alignment: .topLeading) {
                                Color.clear
                                    .contentShape(Rectangle())
                                    .gesture(SpatialTapGesture().onEnded { tap in
                                        onPin(
                                            min(max(tap.location.x / max(geo.size.width, 1), 0), 1),
                                            min(max(tap.location.y / max(geo.size.height, 1), 0), 1))
                                    })
                                ForEach(pins) { m in
                                    Text("\(m.number)")
                                        .font(.caption2.weight(.bold))
                                        .foregroundStyle(.white)
                                        .frame(width: 18, height: 18)
                                        .background(Color.accentColor, in: Circle())
                                        .overlay(Circle().strokeBorder(Color.white, lineWidth: 1.5))
                                        .position(x: m.x * geo.size.width, y: m.y * geo.size.height)
                                        .allowsHitTesting(false)
                                }
                            }
                        })
                    .clipShape(RoundedRectangle(cornerRadius: 6, style: .continuous))
                    .overlay(RoundedRectangle(cornerRadius: 6, style: .continuous)
                        .strokeBorder(Color.primary.opacity(0.15), lineWidth: 1))
                    .help("คลิกเพื่อปักหมุดคอมเมนต์")
            } else {
                Label("เปิดรูปไม่ได้", systemImage: "photo")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, minHeight: 80)
            }
        }
        .onAppear { if image == nil { image = NSImage(contentsOfFile: path) } }
    }
}
