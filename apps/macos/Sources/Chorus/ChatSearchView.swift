import SwiftUI

/// Cross-session Full Text Search Sheet (Ticket 04 & 21, triggered via ⌘F).
/// Fast sub-millisecond search across historical sessions in SQLite.
struct ChatSearchView: View {
    @ObservedObject var engine: Engine
    let dbPath: String
    var onSelectSession: (Int64) -> Void
    var onClose: () -> Void

    @State private var query = ""
    @State private var hits: [Engine.SearchHit] = []
    @State private var isSearching = false

    var body: some View {
        VStack(spacing: 0) {
            // Header Search Input
            HStack(spacing: 10) {
                Image(systemName: "magnifyingglass")
                    .foregroundColor(.secondary)
                    .font(.system(size: 15))

                TextField("Search all sessions, messages, and code snippets...", text: $query)
                    .textFieldStyle(.plain)
                    .font(.system(size: 14))
                    .onSubmit {
                        performSearch()
                    }

                if !query.isEmpty {
                    Button(action: { query = ""; hits = [] }) {
                        Image(systemName: "xmark.circle.fill")
                            .foregroundColor(.secondary)
                    }
                    .buttonStyle(.plain)
                }

                Button("Done", action: onClose)
                    .keyboardShortcut(.cancelAction)
            }
            .padding(14)
            .background(Color(NSColor.controlBackgroundColor))

            Divider()

            // Search Results
            if isSearching {
                VStack {
                    Spacer()
                    ProgressView("Searching sessions...")
                    Spacer()
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if hits.isEmpty {
                VStack(spacing: 10) {
                    Spacer()
                    Image(systemName: "text.magnifyingglass")
                        .font(.system(size: 32))
                        .foregroundColor(.secondary)
                    Text(query.isEmpty ? "Type a search query to search across all sessions" : "No results found for \"\(query)\"")
                        .font(.system(size: 13))
                        .foregroundColor(.secondary)
                    Spacer()
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                List(hits, id: \.id) { hit in
                    Button(action: {
                        onSelectSession(hit.session_id)
                        onClose()
                    }) {
                        VStack(alignment: .leading, spacing: 4) {
                            HStack {
                                Image(systemName: "bubble.left.and.bubble.right")
                                    .foregroundColor(.blue)
                                    .font(.system(size: 11))
                                Text(hit.session_title)
                                    .font(.system(size: 12, weight: .semibold))
                                Spacer()
                                Text("Turn \(hit.seq)")
                                    .font(.system(size: 10, design: .monospaced))
                                    .foregroundColor(.secondary)
                            }
                            Text(hit.snippet)
                                .font(.system(size: 11, design: .monospaced))
                                .foregroundColor(.secondary)
                                .lineLimit(2)
                        }
                        .padding(.vertical, 4)
                    }
                    .buttonStyle(.plain)
                }
            }
        }
        .frame(width: 580, height: 420)
        .background(Color(NSColor.windowBackgroundColor))
        .onChange(of: query) { _, newQ in
            if newQ.count >= 2 {
                performSearch()
            } else if newQ.isEmpty {
                hits = []
            }
        }
    }

    private func performSearch() {
        guard !query.trimmingCharacters(in: .whitespaces).isEmpty else { return }
        isSearching = true
        engine.searchSessions(dbPath: dbPath, query: query) { results in
            self.hits = results
            self.isSearching = false
        }
    }
}
