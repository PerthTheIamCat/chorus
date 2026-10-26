import SwiftUI
import WebKit
import AppKit

/// Interactive Embedded Webview for Dev Server & Artifact Live Preview (Frontier Super App).
/// Allows developers to inspect web UIs, responsive viewports, and local dev servers directly in Chorus.
public struct LivePreviewView: View {
    @Binding var currentURLString: String
    var htmlContent: String?
    var onClose: (() -> Void)?

    @State private var inputURL: String = "http://localhost:3000"
    @State private var webView = ArtifactPreviewSecurity.makeWebView()
    @State private var isLoading: Bool = false
    @State private var canGoBack: Bool = false
    @State private var canGoForward: Bool = false
    @State private var viewportMode: ViewportMode = .responsive

    public enum ViewportMode: String, CaseIterable, Identifiable {
        case responsive = "เต็มจอ"
        case tablet = "iPad (768px)"
        case mobile = "iPhone (375px)"

        public var id: String { rawValue }

        var targetWidth: CGFloat? {
            switch self {
            case .responsive: return nil
            case .tablet: return 768
            case .mobile: return 375
            }
        }

        var icon: String {
            switch self {
            case .responsive: return "macwindow"
            case .tablet: return "ipad"
            case .mobile: return "iphone"
            }
        }
    }

    public init(
        currentURLString: Binding<String>,
        htmlContent: String? = nil,
        onClose: (() -> Void)? = nil
    ) {
        self._currentURLString = currentURLString
        self.htmlContent = htmlContent
        self.onClose = onClose
    }

    public var body: some View {
        VStack(spacing: 0) {
            // Control Header
            HStack(spacing: 8) {
                // Navigation buttons
                HStack(spacing: 6) {
                    Button {
                        if webView.canGoBack { webView.goBack() }
                    } label: {
                        Image(systemName: "chevron.left")
                            .font(.system(size: 11, weight: .semibold))
                    }
                    .buttonStyle(.chorusPlain)
                    .disabled(!canGoBack)
                    .help("ย้อนกลับ")

                    Button {
                        if webView.canGoForward { webView.goForward() }
                    } label: {
                        Image(systemName: "chevron.right")
                            .font(.system(size: 11, weight: .semibold))
                    }
                    .buttonStyle(.chorusPlain)
                    .disabled(!canGoForward)
                    .help("ไปข้างหน้า")

                    Button {
                        loadURL()
                    } label: {
                        Image(systemName: isLoading ? "xmark" : "arrow.clockwise")
                            .font(.system(size: 11, weight: .semibold))
                    }
                    .buttonStyle(.chorusPlain)
                    .help(isLoading ? "หยุดโหลด" : "รีเฟรชหน้าเว็บ (⌘R)")
                }
                .foregroundStyle(.secondary)

                // Quick Port Presets Menu
                Menu {
                    Button("localhost:3000 (React / Next.js)") { setPort(3000) }
                    Button("localhost:3001 (Agent Worktree 1)") { setPort(3001) }
                    Button("localhost:3002 (Agent Worktree 2)") { setPort(3002) }
                    Button("localhost:5173 (Vite / Svelte / Vue)") { setPort(5173) }
                    Button("localhost:8080 (Webpack / Go / Java)") { setPort(8080) }
                    Button("localhost:8000 (Python / FastAPI)") { setPort(8000) }
                } label: {
                    HStack(spacing: 3) {
                        Image(systemName: "network")
                            .font(.system(size: 10))
                        Text("พอร์ต")
                            .font(.caption2.weight(.medium))
                    }
                    .padding(.horizontal, 6)
                    .padding(.vertical, 3)
                    .background(Color.primary.opacity(0.06), in: RoundedRectangle(cornerRadius: 6))
                }
                .menuStyle(.borderlessButton)
                .hoverHighlight()
                .fixedSize()

                // Address Bar
                HStack(spacing: 6) {
                    Image(systemName: "lock.fill")
                        .font(.system(size: 9))
                        .foregroundStyle(.secondary)

                    TextField("กรอก URL (เช่น http://localhost:3000)", text: $inputURL)
                        .textFieldStyle(.plain)
                        .font(.system(size: 11, design: .monospaced))
                        .onSubmit {
                            currentURLString = inputURL
                            loadURL()
                        }

                    if isLoading {
                        ProgressView()
                            .controlSize(.mini)
                    }
                }
                .padding(.horizontal, 8)
                .padding(.vertical, 4)
                .background(Color.primary.opacity(0.04), in: RoundedRectangle(cornerRadius: 8))

                // Viewport Mode Picker
                Picker("", selection: $viewportMode) {
                    ForEach(ViewportMode.allCases) { mode in
                        Label(mode.rawValue, systemImage: mode.icon).tag(mode)
                    }
                }
                .pickerStyle(.segmented)
                .frame(width: 170)

                // Open in External Browser Button
                Button {
                    if let url = ArtifactPreviewSecurity.webURL(inputURL) {
                        NSWorkspace.shared.open(url)
                    }
                } label: {
                    Image(systemName: "arrow.up.right.square")
                        .font(.system(size: 12))
                        .foregroundStyle(.secondary)
                }
                .buttonStyle(.chorusPlain)
                .help("เปิดในเบราว์เซอร์ของระบบ (Safari / Chrome)")

                if let onClose {
                    Button(action: onClose) {
                        Image(systemName: "xmark.circle.fill")
                            .font(.system(size: 13))
                            .foregroundStyle(.secondary)
                    }
                    .buttonStyle(.chorusPlain)
                    .help("ปิดแผงพรีวิว")
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .background(Color(NSColor.controlBackgroundColor).opacity(0.8))

            Divider()

            // Webview Content Canvas
            ZStack {
                Color(NSColor.underPageBackgroundColor)

                if let targetWidth = viewportMode.targetWidth {
                    // Device Frame Simulator (Tablet / Mobile)
                    VStack(spacing: 0) {
                        WebViewContainer(
                            webView: webView,
                            url: inputURL,
                            htmlContent: htmlContent,
                            isLoading: $isLoading,
                            canGoBack: $canGoBack,
                            canGoForward: $canGoForward
                        )
                        .frame(width: targetWidth)
                        .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
                        .overlay(
                            RoundedRectangle(cornerRadius: 16, style: .continuous)
                                .stroke(Color.primary.opacity(0.12), lineWidth: 2)
                        )
                        .padding(.vertical, 20)
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    // Full Responsive Canvas
                    WebViewContainer(
                        webView: webView,
                        url: inputURL,
                        htmlContent: htmlContent,
                        isLoading: $isLoading,
                        canGoBack: $canGoBack,
                        canGoForward: $canGoForward
                    )
                }
            }
        }
        .onAppear {
            inputURL = currentURLString.isEmpty ? "http://localhost:3000" : currentURLString
            loadURL()
        }
        .onChange(of: currentURLString) { _, newURL in
            if !newURL.isEmpty && newURL != inputURL {
                inputURL = newURL
                loadURL()
            }
        }
    }

    private func setPort(_ port: Int) {
        inputURL = "http://localhost:\(port)"
        currentURLString = inputURL
        loadURL()
    }

    private func loadURL() {
        if let html = htmlContent, !html.isEmpty {
            webView.loadHTMLString(ArtifactPreviewSecurity.sandboxedHTML(html), baseURL: nil)
            return
        }

        if let url = ArtifactPreviewSecurity.webURL(inputURL) {
            let req = URLRequest(url: url)
            webView.load(req)
        }
    }
}

// MARK: - AppKit WKWebView Bridge
private struct WebViewContainer: NSViewRepresentable {
    let webView: WKWebView
    let url: String
    let htmlContent: String?
    @Binding var isLoading: Bool
    @Binding var canGoBack: Bool
    @Binding var canGoForward: Bool

    func makeCoordinator() -> Coordinator {
        Coordinator(self)
    }

    func makeNSView(context: Context) -> WKWebView {
        webView.navigationDelegate = context.coordinator
        webView.setValue(false, forKey: "drawsBackground")
        return webView
    }

    func updateNSView(_ nsView: WKWebView, context: Context) {
        context.coordinator.parent = self
        // Managed via parent trigger methods
    }

    class Coordinator: NSObject, WKNavigationDelegate {
        var parent: WebViewContainer

        init(_ parent: WebViewContainer) {
            self.parent = parent
        }

        func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction,
                     decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
            let artifact = !(parent.htmlContent?.isEmpty ?? true)
            decisionHandler(ArtifactPreviewSecurity.allowsNavigation(action.request.url, artifact: artifact) ? .allow : .cancel)
        }

        func webView(_ webView: WKWebView, didStartProvisionalNavigation navigation: WKNavigation!) {
            DispatchQueue.main.async {
                self.parent.isLoading = true
                self.parent.canGoBack = webView.canGoBack
                self.parent.canGoForward = webView.canGoForward
            }
        }

        func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
            DispatchQueue.main.async {
                self.parent.isLoading = false
                self.parent.canGoBack = webView.canGoBack
                self.parent.canGoForward = webView.canGoForward
            }
        }

        func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
            DispatchQueue.main.async {
                self.parent.isLoading = false
            }
        }
    }
}
