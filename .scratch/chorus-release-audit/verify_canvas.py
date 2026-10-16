"""Real WebKit artifact isolation regressions; only a loopback fixture receives traffic."""
from pathlib import Path
import http.server
import subprocess
import sys
import tempfile
import threading

repo = Path(__file__).resolve().parents[2]
requests = []
class Fixture(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        requests.append(self.path)
        body = b"<html>synthetic dev server</html>"
        self.send_response(200)
        self.send_header("Content-Type", "text/html")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def log_message(self, *args):
        pass
    do_POST = do_GET
server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Fixture)
thread = threading.Thread(target=server.serve_forever, daemon=True)
thread.start()
fixture = r'''
extension CanvasPreviewView { func auditHTML() -> String { generateHTML() } }
let app = NSApplication.shared
let endpoint = "__ENDPOINT__"
private class AuditNavigation: CanvasWebViewRepresentable.Coordinator {
    var finished = false
    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) { finished = true }
}
var delegates: [NSObject] = []
var views: [WKWebView] = []
func makeView(_ html: String) -> WKWebView {
    let web = ArtifactPreviewSecurity.makeWebView()
    views.append(web)
    web.configuration.userContentController.addUserScript(WKUserScript(
        source: "window.auditMessages = []; window.addEventListener('message', e => window.auditMessages.push(e.data));",
        injectionTime: .atDocumentStart, forMainFrameOnly: true))
    web.configuration.userContentController.addUserScript(WKUserScript(
        source: "if(window !== window.top) {parent.postMessage({kind:'loaded',rendererPresent:typeof window.mermaid==='object'},'*'); const timer=setInterval(()=>{if(document.querySelector('.mermaid svg')) {parent.postMessage({kind:'renderer',rendered:true},'*');clearInterval(timer);}},50); }",
        injectionTime: .atDocumentEnd, forMainFrameOnly: false))
    let delegate = AuditNavigation()
    delegates.append(delegate)
    web.navigationDelegate = delegate
    web.loadHTMLString(html, baseURL: nil)
    return web
}
func report(_ web: WKWebView, _ kind: String) -> [String: Any] {
    let deadline = Date().addingTimeInterval(10)
    while Date() < deadline {
        var finished = false
        var result: [String: Any]?
        web.evaluateJavaScript("window.auditMessages.find(x => x.kind === '\(kind)') || null") { value, error in
            result = value as? [String: Any]
            finished = true
        }
        while !finished && Date() < deadline { RunLoop.main.run(until: Date().addingTimeInterval(0.02)) }
        if let result { return result }
        RunLoop.main.run(until: Date().addingTimeInterval(0.05))
    }
    fatalError("Missing report: \(kind)")
}
let payload = "graph TD; A-->B\n</div><script>window.auditInjected = true</script><div>"
let html = CanvasPreviewView(language: "mermaid", code: payload).auditHTML()
let encodedRendererTag = ArtifactPreviewSecurity.mermaidScriptTag
    .replacingOccurrences(of: "&", with: "&amp;")
    .replacingOccurrences(of: "\"", with: "&quot;")
    .replacingOccurrences(of: "<", with: "&lt;")
    .replacingOccurrences(of: ">", with: "&gt;")
let offline = html.replacingOccurrences(
    of: encodedRendererTag,
    with: "&lt;script&gt;window.mermaid = {initialize: function(o) {parent.postMessage({kind:'mermaid',injected:window.auditInjected===true,strict:o.securityLevel==='strict'},'*');}};&lt;/script&gt;")
assert(offline != html, "renderer stub must replace the only external renderer")
let mermaid = report(makeView(offline), "mermaid")
assert(mermaid["injected"] as? Bool == false && mermaid["strict"] as? Bool == true)
if CommandLine.arguments.contains("--real-renderer") {
    let valid = CanvasPreviewView(language: "mermaid", code: "flowchart TD; A[Start] --> B[Finish]").auditHTML()
    let rendered = makeView(valid)
    assert(report(rendered, "renderer")["rendered"] as? Bool == true)
    print("PASS: existing Mermaid CDN renderer produces SVG inside the sandbox")
    let invalid = valid.replacingOccurrences(of: ArtifactPreviewSecurity.mermaidIntegrity,
        with: "sha384-" + String(repeating:"A",count:64))
    assert(report(makeView(invalid), "loaded")["rendererPresent"] as? Bool == false)
    print("PASS: WebKit rejects a renderer with incorrect SRI digest")
}
let active = """
<meta http-equiv="Content-Security-Policy" content="default-src * 'unsafe-inline'">
<style>body{background-image:url('\(endpoint)/css')}</style>
<img src="\(endpoint)/image"><iframe src="\(endpoint)/frame"></iframe>
<script src="\(endpoint)/script"></script>
<form id="f" action="\(endpoint)/form"><input name="fixture" value="synthetic"></form>
<script>
const result = {kind:'html', interactive:true};
try {parent.document.body.dataset.escaped='yes';result.parentDenied=false;}catch(e){result.parentDenied=true;}
try {localStorage.setItem('fixture','synthetic');result.storageDenied=false;}catch(e){result.storageDenied=true;}
try {top.location='\(endpoint)/top';}catch(e){}
try {window.open('\(endpoint)/popup');}catch(e){}
try {document.getElementById('f').submit();}catch(e){}
try {navigator.sendBeacon('\(endpoint)/beacon','synthetic');}catch(e){}
const originalPolicy=document.querySelector('meta[http-equiv="Content-Security-Policy"]');
if(originalPolicy) originalPolicy.remove();
fetch('\(endpoint)/fetch').then(()=>{result.fetchDenied=false;},()=>{result.fetchDenied=true;})
.finally(()=>{parent.postMessage(result,'*');setTimeout(()=>{location.href='\(endpoint)/self';},0);});
</script>
"""
let htmlReport = report(makeView(CanvasPreviewView(language: "html", code: active).auditHTML()), "html")
assert(htmlReport["interactive"] as? Bool == true)
assert(htmlReport["parentDenied"] as? Bool == true)
assert(htmlReport["storageDenied"] as? Bool == true)
assert(htmlReport["fetchDenied"] as? Bool == true)
let liveReport = report(makeView(ArtifactPreviewSecurity.sandboxedHTML(active)), "html")
assert(liveReport["parentDenied"] as? Bool == true && liveReport["fetchDenied"] as? Bool == true)
let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" onload=\"parent.postMessage({kind:'svg',interactive:true},'*')\"><image href=\"\(endpoint)/svg-image\" /></svg>"
assert(report(makeView(CanvasPreviewView(language: "svg", code: svg).auditHTML()), "svg")["interactive"] as? Bool == true)
for bad in ["", "http://", "file:///tmp/synthetic-preview", "javascript:alert(1)", "data:text/html,fixture", "chorus://fixture", "mailto:fixture@example.test", "https://user:password@example.test"] {
    assert(ArtifactPreviewSecurity.webURL(bad) == nil, bad)
    assert(!ArtifactPreviewSecurity.allowsNavigation(URL(string: bad), artifact: true))
}
for good in ["localhost:3000", "http://127.0.0.1:3000", "https://example.test"] {
    assert(ArtifactPreviewSecurity.webURL(good) != nil, good)
}
assert(ArtifactPreviewSecurity.allowsNavigation(URL(string: "about:srcdoc"), artifact: true))
// Positive control: ordinary dev-server HTTP mode is still usable.
let dev = ArtifactPreviewSecurity.makeWebView()
class DevNavigation: NSObject, WKNavigationDelegate {
    var finished = false
    func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction, decisionHandler: @escaping (WKNavigationActionPolicy)->Void) {
        decisionHandler(ArtifactPreviewSecurity.allowsNavigation(action.request.url, artifact: false) ? .allow : .cancel)
    }
    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) { finished = true }
}
let devDelegate = DevNavigation()
dev.navigationDelegate = devDelegate
dev.load(URLRequest(url: URL(string:endpoint+"/allowed-dev")!))
let deadline = Date().addingTimeInterval(10)
while !devDelegate.finished && Date() < deadline { RunLoop.main.run(until:Date().addingTimeInterval(0.02)) }
assert(devDelegate.finished)
print("PASS: Mermaid injection, opaque origin, storage and network isolation; inline HTML/SVG and dev-server mode work")
'''
fixture = fixture.replace("__ENDPOINT__", f"http://127.0.0.1:{server.server_port}")
sources = ["ArtifactPreviewSecurity.swift", "CanvasPreviewView.swift"]
source = "\n".join((repo / "apps/macos/Sources/Chorus" / name).read_text() for name in sources)
try:
    with tempfile.TemporaryDirectory(prefix="chorus-canvas-audit-") as scratch:
        directory = Path(scratch)
        (directory / "main.swift").write_text(source + fixture)
        compiled = subprocess.run(["swiftc", str(directory / "main.swift"), "-o", str(directory / "audit")],capture_output=True,text=True)
        assert compiled.returncode == 0, compiled.stderr[-3000:]
        arguments = [str(directory / "audit")]
        if "--real-renderer" in sys.argv:
            arguments.append("--real-renderer")
        result = subprocess.run(arguments,capture_output=True,text=True,timeout=50)
        assert result.returncode == 0, result.stderr[-3000:]
        assert requests == ["/allowed-dev"], requests
        print(result.stdout.strip())
        print("PASS: loopback recorder saw only the explicit dev-server positive control")
finally:
    server.shutdown()
    server.server_close()
