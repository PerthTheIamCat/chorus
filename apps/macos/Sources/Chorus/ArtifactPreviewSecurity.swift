import Foundation
import WebKit

/// Generated artifacts run with an opaque origin and no ambient native/browser authority.
enum ArtifactPreviewSecurity {
    static let mermaidScriptURL = "https://cdn.jsdelivr.net/npm/mermaid@10.9.8/dist/mermaid.min.js"
    static let mermaidIntegrity = "sha384-N3QqR/7q+xm3BGX+CBbNI8AUmRRqcsDzToy+0z1NLDI0QmTKW8zvwLvqulJgk3dP"
    static let mermaidScriptTag = "<script src=\"\(mermaidScriptURL)\" integrity=\"\(mermaidIntegrity)\" crossorigin=\"anonymous\"></script>"

    static func sandboxedHTML(_ html: String, rendererScript: Bool = false) -> String {
        // Mermaid's trusted template needs its renderer; ordinary artifact scripts are inline only.
        let renderer = rendererScript ? " \(mermaidScriptURL)" : ""
        let policy = "default-src 'none'; script-src 'unsafe-inline'\(renderer); style-src 'unsafe-inline'; img-src data: blob:; font-src data:; media-src data: blob:; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'"
        // srcdoc inherits its parent's CSP; the child may tighten it but cannot relax it.
        let parentPolicy = policy.replacingOccurrences(of: "frame-src 'none'", with: "frame-src about:")
        let child = "<meta http-equiv=\"Content-Security-Policy\" content=\"\(policy)\">" + html
        let escaped = child
            .replacingOccurrences(of: "&", with: "&amp;")
            .replacingOccurrences(of: "\"", with: "&quot;")
            .replacingOccurrences(of: "<", with: "&lt;")
            .replacingOccurrences(of: ">", with: "&gt;")
        return """
        <!doctype html><html><head>
        <meta charset="utf-8">
        <meta http-equiv="Content-Security-Policy" content="\(parentPolicy)">
        <style>html,body{margin:0;width:100%;height:100%;overflow:hidden}iframe{border:0;width:100%;height:100%}</style>
        </head><body><iframe sandbox="allow-scripts" srcdoc="\(escaped)"></iframe></body></html>
        """
    }

    static func makeWebView() -> WKWebView {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .nonPersistent()
        return WKWebView(frame: .zero, configuration: configuration)
    }

    static func allowsNavigation(_ url: URL?, artifact: Bool) -> Bool {
        guard let url, let scheme = url.scheme?.lowercased() else { return false }
        if artifact {
            let document = url.absoluteString.split(separator: "#", maxSplits: 1).first
            return document == "about:blank" || document == "about:srcdoc"
        }
        return ["http", "https"].contains(scheme) && !(url.host?.isEmpty ?? true)
            && url.user == nil && url.password == nil
    }

    static func webURL(_ input: String) -> URL? {
        let trimmed = input.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        var candidate = trimmed
        if !trimmed.lowercased().hasPrefix("http://") && !trimmed.lowercased().hasPrefix("https://") {
            // Bare hosts/ports are convenient; explicit non-web schemes have no authority here.
            if let components = URLComponents(string: trimmed), let scheme = components.scheme {
                let isHostPort = !trimmed.contains("/") && trimmed.split(separator: ":", omittingEmptySubsequences: false).count == 2
                    && trimmed.split(separator: ":", omittingEmptySubsequences: false).last?.allSatisfy(\.isNumber) == true
                guard isHostPort && !["file", "javascript", "data", "mailto", "tel"].contains(scheme.lowercased()) else { return nil }
            }
            candidate = "http://" + trimmed
        }
        guard let url = URL(string: candidate), allowsNavigation(url, artifact: false),
              url.user == nil, url.password == nil else { return nil }
        return url
    }
}
