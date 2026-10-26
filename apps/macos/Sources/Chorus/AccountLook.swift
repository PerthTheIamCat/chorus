import SwiftUI

// How an account looks (name, icon, colour) and the sheet that edits it.
// The look is cosmetic and lives in Rust (`accounts::Meta`); the account's
// identity stays its directory name, so renaming never breaks a login or a
// relay chain. This file only draws and collects input.

extension Color {
    /// `#RRGGBB`; anything else is gray.
    init(hex: String) {
        var s = hex.trimmingCharacters(in: .whitespaces)
        if s.hasPrefix("#") { s.removeFirst() }
        guard s.count == 6, let v = UInt32(s, radix: 16) else { self = .gray; return }
        self.init(.sRGB,
                  red: Double((v >> 16) & 0xFF) / 255,
                  green: Double((v >> 8) & 0xFF) / 255,
                  blue: Double(v & 0xFF) / 255)
    }
}

/// Icon choices besides the provider's own mark.
enum AccountSymbols {
    static let all = [
        "person.fill", "briefcase.fill", "house.fill", "building.2.fill", "person.2.fill", "graduationcap.fill",
        "star.fill", "bolt.fill", "flame.fill", "leaf.fill", "shield.fill", "flask.fill",
    ]
}

enum AccountColors {
    static let all = ["#D97757", "#E5484D", "#F5A524", "#30A46C", "#10A37F", "#0091FF", "#6E56CF", "#D6409F"]
}

/// Name suggestions shown as chips when adding an account. `slug` is the
/// folder name (ASCII); `label` is what the user reads.
struct AccountPreset: Identifiable {
    let label: String
    let slug: String
    let symbol: String
    var id: String { slug }

    static let all = [
        AccountPreset(label: "ส่วนตัว", slug: "personal", symbol: "person.fill"),
        AccountPreset(label: "งาน", slug: "work", symbol: "briefcase.fill"),
        AccountPreset(label: "ทีม", slug: "team", symbol: "person.2.fill"),
        AccountPreset(label: "ลูกค้า", slug: "client", symbol: "building.2.fill"),
        AccountPreset(label: "สำรอง", slug: "backup", symbol: "shield.fill"),
        AccountPreset(label: "ทดลอง", slug: "test", symbol: "flask.fill"),
    ]
}

/// The tile in front of an account's name: the provider mark, or a chosen
/// symbol, on the account's colour (the provider's own accent by default).
struct AccountAvatar: View {
    let agent: String
    var icon: String?
    var color: String?
    var size: CGFloat = 28

    var body: some View {
        let tint = color.map { Color(hex: $0) } ?? ProviderIcon.brandColor(for: agent)
        RoundedRectangle(cornerRadius: size * 0.3, style: .continuous)
            .fill(tint.opacity(0.18))
            .overlay {
                if let icon, icon.hasPrefix("sf:") {
                    Image(systemName: String(icon.dropFirst(3)))
                        .font(.system(size: size * 0.46, weight: .semibold))
                        .foregroundStyle(tint)
                } else {
                    ProviderIcon(provider: agent, size: size * 0.52)
                        .foregroundStyle(tint)
                }
            }
            .frame(width: size, height: size)
            .accessibilityHidden(true)
    }
}

/// What the sheet is for.
enum AccountEditorTarget: Identifiable {
    case create(agent: String)
    case edit(Engine.Account)

    var id: String {
        switch self {
        case .create(let agent): "create-\(agent)"
        case .edit(let a): "edit-\(a.name)"
        }
    }
}

struct AccountEditorSheet: View {
    @EnvironmentObject var engine: Engine
    @Environment(\.dismiss) private var dismiss
    let target: AccountEditorTarget

    @State private var label = ""
    @State private var slug: String?
    @State private var icon: String?
    @State private var color: String?
    @FocusState private var focused: Bool

    private var agent: String {
        switch target {
        case .create(let agent): agent
        case .edit(let a): a.agent
        }
    }

    private var creating: Bool {
        if case .create = target { return true }
        return false
    }

    private var trimmed: String { label.trimmingCharacters(in: .whitespacesAndNewlines) }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            header

            VStack(alignment: .leading, spacing: 6) {
                Text("ชื่อบัญชี").font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                TextField(creating ? "เช่น งาน, ส่วนตัว" : "ชื่อที่แสดง", text: $label)
                    .textFieldStyle(.roundedBorder)
                    .focused($focused)
                    .onSubmit(save)
                    .onChange(of: label) { _, _ in
                        // Typing your own name drops the preset's folder name.
                        if let slug, AccountPreset.all.first(where: { $0.slug == slug })?.label != trimmed {
                            self.slug = nil
                        }
                    }
                if creating {
                    FlowChips(items: AccountPreset.all) { preset in
                        Button {
                            label = preset.label
                            slug = preset.slug
                            if icon == nil || icon?.hasPrefix("sf:") == true { icon = "sf:" + preset.symbol }
                        } label: {
                            Label(preset.label, systemImage: preset.symbol).font(.caption)
                        }
                        .buttonStyle(InteractiveStyle(cornerRadius: 14, selected: slug == preset.slug,
                                                      padding: .init(top: 4, leading: 9, bottom: 4, trailing: 9)))
                    }
                }
            }

            VStack(alignment: .leading, spacing: 6) {
                Text("ไอคอน").font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                LazyVGrid(columns: Array(repeating: GridItem(.fixed(36), spacing: 8), count: 7), alignment: .leading, spacing: 8) {
                    pick(nil)
                    ForEach(AccountSymbols.all, id: \.self) { pick("sf:" + $0) }
                }
            }

            VStack(alignment: .leading, spacing: 6) {
                Text("สี").font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                HStack(spacing: 8) {
                    swatch(nil)
                    ForEach(AccountColors.all, id: \.self) { swatch($0) }
                }
            }

            if creating {
                Text("Chorus จะสร้างโฟลเดอร์ login แยกให้บัญชีนี้แล้วเปิด Terminal ให้ login — ไม่เก็บรหัสหรือ token เอง")
                    .font(.caption).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            } else {
                Text("เปลี่ยนได้แค่ชื่อที่แสดงกับหน้าตา — login และชื่อโฟลเดอร์เดิมไม่ถูกแตะ")
                    .font(.caption).foregroundStyle(.secondary)
            }

            HStack {
                Spacer()
                Button("ยกเลิก", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button(creating ? "เพิ่มแล้ว login" : "บันทึก", action: save)
                    .keyboardShortcut(.defaultAction)
                    .buttonStyle(.borderedProminent)
                    .disabled(creating && trimmed.isEmpty)
            }
        }
        .padding(20)
        .frame(width: 400)
        .onAppear {
            if case .edit(let a) = target {
                label = a.label ?? (a.account.isEmpty ? "" : a.account)
                icon = a.icon
                color = a.color
            }
            focused = true
        }
    }

    private var header: some View {
        HStack(spacing: 12) {
            AccountAvatar(agent: agent, icon: icon, color: color, size: 44)
            VStack(alignment: .leading, spacing: 2) {
                Text(creating ? "เพิ่มบัญชี \(agentLabel)" : "แก้ไขบัญชี").font(.headline)
                Text(trimmed.isEmpty ? "ยังไม่ได้ตั้งชื่อ" : trimmed)
                    .font(.subheadline).foregroundStyle(.secondary).lineLimit(1)
            }
        }
    }

    private var agentLabel: String {
        engine.agents.first { $0.id == agent }?.label ?? agent
    }

    private func pick(_ value: String?) -> some View {
        Button { icon = value } label: {
            AccountAvatar(agent: agent, icon: value, color: color, size: 32)
                .overlay(
                    RoundedRectangle(cornerRadius: 10, style: .continuous)
                        .strokeBorder(icon == value ? Color.accentColor : .clear, lineWidth: 2))
        }
        .buttonStyle(.chorusPlain)
        .help(value == nil ? "โลโก้ของเจ้านี้" : "ไอคอน")
        .accessibilityLabel(value == nil ? "โลโก้ของเจ้านี้" : "ไอคอน")
        .accessibilityAddTraits(icon == value ? .isSelected : [])
    }

    private func swatch(_ value: String?) -> some View {
        Button { color = value } label: {
            Circle()
                .fill(value.map { Color(hex: $0) } ?? ProviderIcon.brandColor(for: agent))
                .frame(width: 20, height: 20)
                .overlay(Circle().strokeBorder(Color.primary.opacity(0.35), lineWidth: color == value ? 2 : 0).padding(-3))
        }
        .buttonStyle(.chorusPlain)
        .help(value == nil ? "สีของเจ้านี้" : "สี")
        .accessibilityLabel(value == nil ? "สีของเจ้านี้" : "สี")
        .accessibilityAddTraits(color == value ? .isSelected : [])
    }

    private func save() {
        switch target {
        case .create:
            guard !trimmed.isEmpty else { return }
            engine.createAccount(agent: agent, label: trimmed, slug: slug, icon: icon, color: color)
        case .edit(let a):
            engine.setAccountLook(agent: a.agent, name: a.account, label: trimmed, icon: icon, color: color)
        }
        dismiss()
    }
}

/// Chips that wrap onto more lines instead of scrolling sideways.
struct FlowChips<Item: Identifiable, Chip: View>: View {
    let items: [Item]
    @ViewBuilder let chip: (Item) -> Chip

    var body: some View {
        FlowLayout(spacing: 6) {
            ForEach(items) { chip($0) }
        }
    }
}

struct FlowLayout: Layout {
    var spacing: CGFloat = 6

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        arrange(width: proposal.width ?? .infinity, subviews: subviews).size
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let laid = arrange(width: bounds.width, subviews: subviews)
        for (i, origin) in laid.origins.enumerated() {
            subviews[i].place(at: CGPoint(x: bounds.minX + origin.x, y: bounds.minY + origin.y), proposal: .unspecified)
        }
    }

    private func arrange(width: CGFloat, subviews: Subviews) -> (size: CGSize, origins: [CGPoint]) {
        var origins: [CGPoint] = []
        var x: CGFloat = 0, y: CGFloat = 0, rowHeight: CGFloat = 0, maxX: CGFloat = 0
        for sub in subviews {
            let s = sub.sizeThatFits(.unspecified)
            if x > 0, x + s.width > width { x = 0; y += rowHeight + spacing; rowHeight = 0 }
            origins.append(CGPoint(x: x, y: y))
            x += s.width + spacing
            rowHeight = max(rowHeight, s.height)
            maxX = max(maxX, x - spacing)
        }
        return (CGSize(width: maxX, height: y + rowHeight), origins)
    }
}
