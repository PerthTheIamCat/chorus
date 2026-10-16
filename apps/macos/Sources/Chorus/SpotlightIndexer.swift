import Foundation
import CoreSpotlight
import UniformTypeIdentifiers

/// Native CoreSpotlight indexer for Chorus AI sessions and code decisions.
///
/// Enables users to search via macOS Spotlight (⌘Space) or Alfred / Raycast
/// for previous agent sessions, architectural decisions, and debugging runs.
final class SpotlightIndexer {
    static let shared = SpotlightIndexer()
    private let index = CSSearchableIndex.default()
    private let domainIdentifier = "dev.chorus.app.sessions"

    private init() {}

    /// Indexes an active or updated Chorus session.
    func indexSession(id: Int64, title: String, goal: String, cwd: String) {
        guard CSSearchableIndex.isIndexingAvailable() else { return }

        let attributeSet = CSSearchableItemAttributeSet(contentType: .text)
        attributeSet.title = title.isEmpty ? "Chorus Session #\(id)" : title
        attributeSet.contentDescription = goal.isEmpty ? "Workspace: \(cwd)" : "Goal: \(goal)\nWorkspace: \(cwd)"
        attributeSet.keywords = ["Chorus", "AI", "Agent", "Coding", (cwd as NSString).lastPathComponent]

        let item = CSSearchableItem(
            uniqueIdentifier: "\(domainIdentifier).\(id)",
            domainIdentifier: domainIdentifier,
            attributeSet: attributeSet
        )

        // Expire after 90 days if not re-indexed
        item.expirationDate = Date().addingTimeInterval(90 * 86400)

        index.indexSearchableItems([item]) { error in
            if let error = error {
                NSLog("[SpotlightIndexer] Failed to index session %lld: %@", id, error.localizedDescription)
            }
        }
    }

    /// Deletes a session from Spotlight when closed or deleted.
    func deleteSession(id: Int64) {
        guard CSSearchableIndex.isIndexingAvailable() else { return }
        index.deleteSearchableItems(withIdentifiers: ["\(domainIdentifier).\(id)"]) { error in
            if let error = error {
                NSLog("[SpotlightIndexer] Failed to remove session %lld: %@", id, error.localizedDescription)
            }
        }
    }

    /// Clears all indexed items for Chorus.
    func clearAll() {
        guard CSSearchableIndex.isIndexingAvailable() else { return }
        index.deleteSearchableItems(withDomainIdentifiers: [domainIdentifier], completionHandler: nil)
    }
}
