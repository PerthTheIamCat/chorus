// swift-tools-version:5.9
import PackageDescription

// The Rust core is built by ../build.sh (or `cargo build -p chorus-ffi`)
// into ../../target/{debug,release}. We link the debug static lib by
// default; pass -c release to swift build to match a release cargo build
// and update the -L path below if you do.
let target = "debug"

let package = Package(
    name: "Chorus",
    platforms: [.macOS(.v14)],
    dependencies: [
        .package(url: "https://github.com/sparkle-project/Sparkle", from: "2.6.4"),
    ],
    targets: [
        .systemLibrary(name: "CChorus", path: "Sources/CChorus"),
        .executableTarget(
            name: "Chorus",
            dependencies: [
                "CChorus",
                .product(name: "Sparkle", package: "Sparkle"),
            ],
            resources: [.process("Resources")],
            linkerSettings: [
                .unsafeFlags([
                    "-L../../target/\(target)",
                    "-lchorus_ffi",
                    // Rust staticlib transitive needs:
                    "-lc++",
                    // Sparkle.framework is embedded by build.sh into
                    // Contents/Frameworks of the .app bundle.
                    "-Xlinker", "-rpath", "-Xlinker", "@executable_path/../Frameworks",
                    "-lresolv",
                    "-framework", "CoreFoundation",
                    "-framework", "Security",
                    "-framework", "SystemConfiguration",
                    "-framework", "Vision",
                    "-framework", "CoreSpotlight",
                    // chorus-core's computer use: CGEvent input, display
                    // geometry and the TCC checks (AXIsProcessTrusted).
                    "-framework", "CoreGraphics",
                    "-framework", "ApplicationServices",
                ])
            ]
        ),
        // `swift test` links the same staticlib: run `cargo build -p chorus-ffi` first.
        .testTarget(
            name: "ChorusTests",
            dependencies: ["Chorus"]
        ),
    ]
)
