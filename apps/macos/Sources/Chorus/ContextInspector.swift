import SwiftUI

// What the agent saw this turn. Everything here is a number or a sentence
// Rust already computed (`chorus_context_inspect`); this file only decodes
// and draws it (CLAUDE.md 2.3).

struct ContextInspection: Codable, Equatable {
    var budget: Int64
    var text: String
    var tokens: Int64
    var report: ContextReportData
    var target: ContextTargetPreview?
}

struct ContextReportData: Codable, Equatable {
    var budget: Int64
    var total_tokens: Int64
    var tiers: [ContextTier]
    var conversation: ContextConversation
    var memory: [ContextMemoryNote]
    var code: [ContextCodeNote]
}

struct ContextTier: Codable, Equatable, Identifiable {
    var tier: String
    var tokens: Int64
    var cap: Int64
    var fill_pct: Double
    var note: String
    var id: String { tier }
}

struct ContextConversation: Codable, Equatable {
    var turns_total: Int
    var at_l0: Int
    var at_l1: Int
    var at_l2: Int
    var demoted_by_budget: Int
    var dropped: Int
    var notes: [ContextTurnNote]
    var notes_omitted: Int
}

struct ContextTurnNote: Codable, Equatable, Identifiable {
    var age: Int
    var from_rung: Int
    var to_rung: Int
    var dropped: Bool
    var tokens_before: Int64
    var tokens_after: Int64
    var preview: String
    var id: Int { age }
}

struct ContextMemoryNote: Codable, Equatable, Identifiable {
    var text: String
    var score: Double?
    var pinned: Bool
    var included: Bool
    var tokens: Int64
    /// nil until Chorus counts references (memory counters); the column is
    /// hidden while every fact has nil.
    var referenced: Bool?
    var id: String { text }
}

struct ContextCodeNote: Codable, Equatable, Identifiable {
    var label: String
    var tokens: Int64
    var status: String
    var id: String { label + status + String(tokens) }
}

struct ContextTargetPreview: Codable, Equatable {
    var agent: String
    var budget: Int64
    var budget_known: Bool
    var context_tokens: Int64
    var demoted: Int
    var dropped: Int
    var brief_markdown: String
    var brief_tokens: Int64
    var instruction_file: String?
}

/// Names and colours for the tiers, in the order Rust sends them.
enum ContextTierStyle {
    static func title(_ tier: String) -> String {
        switch tier {
        case "System": "ระบบ"
        case "TaskState": "สถานะงาน"
        case "Memory": "ความจำ"
        case "Conversation": "บทสนทนา"
        case "Code": "โค้ดและ diff"
        case "Reserve": "กันไว้ให้ตอบ"
        default: tier
        }
    }

    static func color(_ tier: String) -> Color {
        switch tier {
        case "System": .gray
        case "TaskState": .blue
        case "Memory": .orange
        case "Conversation": .green
        case "Code": .purple
        default: Color.primary.opacity(0.18)
        }
    }

    static func rungLabel(_ rung: Int) -> String {
        ["เต็ม", "สรุปสั้น (L1)", "สรุปตอน (L2)"][min(max(rung, 0), 2)]
    }
}

/// "12,345" — the numbers here are token counts a person compares by eye.
func tokenText(_ n: Int64) -> String {
    let f = NumberFormatter()
    f.numberStyle = .decimal
    return f.string(from: NSNumber(value: n)) ?? String(n)
}

