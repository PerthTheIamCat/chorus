import SwiftUI
import WebKit

// A web page next to the work: a dev server on localhost, or any http(s) URL.
// Navigation is limited by `ArtifactPreviewSecurity` (no file:, javascript:,
// credentials in the URL) and the page gets a non-persistent data store.

@MainActor
final class PreviewModel: NSObject, ObservableObject, WKNavigationDelegate {
    let webView = ArtifactPreviewSecurity.makeWebView()
    @Published var input = "http://localhost:3000"
    @Published var loading = false
    @Published var canGoBack = false
    @Published var canGoForward = false
    @Published var error: String?

    override init() {
        super.init()
        webView.navigationDelegate = self
    }

    func load() {
        guard let url = ArtifactPreviewSecurity.webURL(input) else {
            error = "ใส่ที่อยู่เว็บแบบ http:// หรือ https:// หรือ localhost:พอร์ต"
            return
        }
        error = nil
        webView.load(URLRequest(url: url))
    }

    private func refresh() {
        loading = webView.isLoading
        canGoBack = webView.canGoBack
        canGoForward = webView.canGoForward
        if let u = webView.url?.absoluteString { input = u }
    }

    nonisolated func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction,
                             decisionHandler: @escaping @MainActor (WKNavigationActionPolicy) -> Void) {
        let ok = ArtifactPreviewSecurity.allowsNavigation(action.request.url, artifact: false)
        Task { @MainActor in decisionHandler(ok ? .allow : .cancel) }
    }

    nonisolated func webView(_ webView: WKWebView, didStartProvisionalNavigation navigation: WKNavigation!) {
        Task { @MainActor in self.refresh() }
    }

    nonisolated func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        Task { @MainActor in self.refresh() }
    }

    nonisolated func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
        let message = error.localizedDescription
        Task { @MainActor in self.refresh(); self.error = message }
    }

    nonisolated func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
        let message = error.localizedDescription
        Task { @MainActor in self.refresh(); self.error = message }
    }
}

private struct WebHost: NSViewRepresentable {
    let webView: WKWebView
    func makeNSView(context: Context) -> WKWebView { webView }
    func updateNSView(_ nsView: WKWebView, context: Context) {}
}

struct ShellPreview: View {
    @StateObject private var model = PreviewModel()
    @State private var width: Width = .full

    enum Width: String, CaseIterable, Identifiable {
        case full = "เต็ม", tablet = "แท็บเล็ต", phone = "มือถือ"
        var id: String { rawValue }
        var points: CGFloat? {
            switch self { case .full: nil; case .tablet: 768; case .phone: 375 }
        }
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                Button { model.webView.goBack() } label: { Image(systemName: "chevron.left") }
                    .disabled(!model.canGoBack).help("ย้อนกลับ")
                Button { model.webView.goForward() } label: { Image(systemName: "chevron.right") }
                    .disabled(!model.canGoForward).help("ไปข้างหน้า")
                Button { model.webView.reload() } label: { Image(systemName: "arrow.clockwise") }
                    .help("โหลดใหม่")
                TextField("ที่อยู่เว็บ", text: $model.input)
                    .textFieldStyle(.roundedBorder)
                    .onSubmit { model.load() }
                if model.loading { ProgressView().controlSize(.small) }
                Picker("ความกว้าง", selection: $width) {
                    ForEach(Width.allCases) { Text($0.rawValue).tag($0) }
                }
                .pickerStyle(.segmented).labelsHidden().frame(width: 190)
            }
            .buttonStyle(.borderless)
            .padding(10)
            Divider()
            if let e = model.error {
                Text(e).font(.caption).foregroundStyle(.red).padding(8).frame(maxWidth: .infinity, alignment: .leading)
            }
            WebHost(webView: model.webView)
                .frame(maxWidth: width.points ?? .infinity)
                .frame(maxWidth: .infinity)
                .background(Color(nsColor: .underPageBackgroundColor))
        }
        .onAppear { if model.webView.url == nil { model.load() } }
    }
}
