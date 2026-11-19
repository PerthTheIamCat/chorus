import AppKit
import SwiftUI
import UniformTypeIdentifiers
import UserNotifications

/// Title + optional subtitle + trailing actions — the top of every inspector tab.
struct PaneHeader<Trailing: View>: View {
    let title: String
    var subtitle: String?
    @ViewBuilder var trailing: () -> Trailing

    var body: some View {
        HStack(alignment: .center) {
            VStack(alignment: .leading, spacing: 1) {
                Text(title).font(.headline).lineLimit(1)
                if let subtitle { Text(subtitle).font(.caption).foregroundStyle(.secondary).lineLimit(1) }
            }
            Spacer(minLength: 4)
            trailing()
        }
        .padding(.horizontal, 12).padding(.top, 10).padding(.bottom, 6)
    }
}

struct SectionLabel: View {
    let text: String
    let count: Int?
    let tint: Color
    init(_ text: String, count: Int?, tint: Color) { self.text = text; self.count = count; self.tint = tint }

    var body: some View {
        HStack(spacing: 5) {
            Text(text).font(.caption.weight(.semibold)).foregroundStyle(.secondary)
            if let count {
                Text("\(count)").font(.caption2.weight(.bold)).foregroundStyle(tint)
                    .padding(.horizontal, 5).padding(.vertical, 1)
                    .background(tint.opacity(0.15), in: Capsule())
            }
        }
        .padding(.top, 4)
    }
}

/// An empty tab that says what will appear and how it gets there.
struct EmptyPane: View {
    let icon: String
    let title: String
    let hint: String

    var body: some View {
        VStack(spacing: 8) {
            Image(systemName: icon).font(.system(size: 28)).foregroundStyle(.tertiary)
            Text(title).font(.callout.weight(.medium))
            Text(hint).font(.caption).foregroundStyle(.secondary).multilineTextAlignment(.center)
        }
        .padding(24)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

/// A file's row in the Changes pane: name, totals, collapse toggle. Its
/// hunks are sibling rows of the parent's lazy stack, not children here.
struct FileChangesHeader: View {
    let file: String
    let hunks: [DiffHunk]
    let open: Bool
    /// Rust says this path is a test file: an agent that edits tests deserves a second look.
    var isTest: Bool = false
    let toggle: () -> Void

    var body: some View {
        Button { withAnimation(.easeOut(duration: 0.15)) { toggle() } } label: {
            HStack(spacing: 6) {
                Image(systemName: "chevron.right").font(.system(size: 9, weight: .bold))
                    .rotationEffect(.degrees(open ? 90 : 0)).foregroundStyle(.secondary)
                Image(systemName: "doc.text").foregroundStyle(.secondary)
                Text((file as NSString).lastPathComponent).font(.callout.weight(.medium)).lineLimit(1)
                if isTest {
                    Text("แก้เทสต์")
                        .font(.system(size: 9, weight: .semibold))
                        .foregroundStyle(.orange)
                        .padding(.horizontal, 5).padding(.vertical, 1)
                        .background(Color.orange.opacity(0.15), in: Capsule())
                        .help("ไฟล์นี้เป็นเทสต์ — ตรวจว่าแก้เพราะ spec เปลี่ยนจริง ไม่ใช่แก้ให้ผ่าน")
                }
                Spacer(minLength: 4)
                Text("+\(hunks.reduce(0) { $0 + $1.addCount })").foregroundStyle(.green)
                Text("−\(hunks.reduce(0) { $0 + $1.delCount })").foregroundStyle(.red)
            }
            .font(.caption.monospacedDigit())
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .buttonStyle(InteractiveStyle(cornerRadius: 7, padding: .init(top: 5, leading: 6, bottom: 5, trailing: 6)))
        .background(Color.primary.opacity(0.035), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
        .help(file)
        .padding(.top, 4)
    }
}
