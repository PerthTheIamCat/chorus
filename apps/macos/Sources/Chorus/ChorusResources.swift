import Foundation

/// Where the app's bundled resources (icons) live.
///
/// SwiftPM's generated `Bundle.module` looks for `Chorus_Chorus.bundle` next
/// to the app (`Chorus.app/…`) or at the absolute path of the build machine's
/// `.build` folder — never in `Contents/Resources`, where build.sh puts it
/// (a bundle at the app root would break the code signature). Inside an
/// installed app it therefore hits a `fatalError` as soon as the first icon is
/// drawn, unless the build folder it was compiled in still exists.
/// `Bundle.module` is only touched when running outside an .app (`swift run`).
enum ChorusResources {
    private static let bundleName = "Chorus_Chorus.bundle"

    static func path(forResource name: String, ofType ext: String) -> String? {
        let inApp = [Bundle.main.resourceURL, Bundle.main.bundleURL]
            .compactMap { $0?.appendingPathComponent(bundleName) }
        for url in inApp {
            if let path = Bundle(url: url)?.path(forResource: name, ofType: ext) { return path }
        }
        if Bundle.main.bundlePath.hasSuffix(".app") { return nil }
        return Bundle.module.path(forResource: name, ofType: ext)
    }
}
