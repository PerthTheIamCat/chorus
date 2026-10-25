import AppKit
import UserNotifications

/// The system notification for "a turn finished while Chorus was in the
/// background" — the point of running several agents is not watching them.
enum TurnNotice {
    /// Title and body for a finished turn: the failure if there was one,
    /// otherwise the first line of the agent's last message.
    static func content(lastError: String?, lastReply: String?, agent: String?) -> (title: String, body: String) {
        let who = agent.map { " · \($0)" } ?? ""
        if let err = lastError?.trimmingCharacters(in: .whitespacesAndNewlines), !err.isEmpty {
            return ("Chorus: เทิร์นล้มเหลว\(who)", firstLine(err))
        }
        let reply = firstLine(lastReply ?? "")
        return ("Chorus: เทิร์นเสร็จแล้ว\(who)", reply.isEmpty ? "agent ตอบเสร็จแล้ว" : reply)
    }

    static func firstLine(_ text: String, limit: Int = 140) -> String {
        let line = text.split(whereSeparator: \.isNewline)
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .first { !$0.isEmpty } ?? ""
        return line.count > limit ? String(line.prefix(limit - 1)) + "…" : line
    }

    /// Posts only from the .app (a bare executable has no bundle id and
    /// UNUserNotificationCenter raises) and only while Chorus is not frontmost.
    static func post(id: String, title: String, body: String) {
        guard Bundle.main.bundleIdentifier != nil else { return }
        let center = UNUserNotificationCenter.current()
        center.requestAuthorization(options: [.alert, .sound]) { granted, error in
            if let error { NSLog("chorus: notification permission failed: \(error)") }
            guard granted else { return }
            let content = UNMutableNotificationContent()
            content.title = title
            content.body = body
            content.sound = .default
            center.add(UNNotificationRequest(identifier: id, content: content, trigger: nil)) { error in
                if let error { NSLog("chorus: notification not delivered: \(error)") }
            }
        }
    }
}

/// Clicking a Chorus notification brings the app forward.
final class NotificationClickHandler: NSObject, UNUserNotificationCenterDelegate {
    static let shared = NotificationClickHandler()

    func userNotificationCenter(_ center: UNUserNotificationCenter,
                                didReceive response: UNNotificationResponse,
                                withCompletionHandler completionHandler: @escaping () -> Void) {
        DispatchQueue.main.async { NSApp.activate(ignoringOtherApps: true) }
        completionHandler()
    }
}
