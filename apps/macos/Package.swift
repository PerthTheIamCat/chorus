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
    targets: [
        .systemLibrary(name: "CChorus", path: "Sources/CChorus"),
        .executableTarget(
            name: "Chorus",
            dependencies: ["CChorus"],
            resources: [.process("Resources")],
            linkerSettings: [
                .unsafeFlags([
                    "-L../../target/\(target)",
                    "-lchorus_ffi",
                    // Rust staticlib transitive needs:
                    "-lc++",
                    "-lresolv",
                    "-framework", "CoreFoundation",
                    "-framework", "Security",
                    "-framework", "SystemConfiguration",
                    "-framework", "Vision",
                    "-framework", "CoreSpotlight",
                ])
            ]
        ),
    ]
)
