// swift-tools-version:6.0
import Foundation
import PackageDescription

// The Rust core is built by Cargo and staged into `macos/lib` by
// `scripts/build.sh`, which is what selects debug vs release. Resolving the
// path from #filePath keeps linking independent of the working directory.
let packageDir = URL(fileURLWithPath: #filePath).deletingLastPathComponent().path

let coreLinkSettings: [LinkerSetting] = [
    .unsafeFlags(["-L\(packageDir)/lib"]),
    .linkedLibrary("newt_ffi"),
]

let package = Package(
    name: "newt",
    platforms: [.macOS(.v14)],
    products: [
        .executable(name: "NewtApp", targets: ["NewtApp"]),
        .library(name: "NewtKit", targets: ["NewtKit"]),
    ],
    targets: [
        // Raw C ABI. Nothing outside NewtKit should import this.
        .systemLibrary(name: "CNewt", path: "Sources/CNewt"),

        // Safe Swift types over the ABI. Terminal semantics live in the core,
        // never here.
        .target(
            name: "NewtKit",
            dependencies: ["CNewt"],
            linkerSettings: coreLinkSettings
        ),

        // AppKit shell: windows, tabs, panes, CoreText renderer.
        .executableTarget(
            name: "NewtApp",
            dependencies: ["NewtKit"],
            linkerSettings: coreLinkSettings
        ),

        .testTarget(name: "NewtKitTests", dependencies: ["NewtKit"]),
    ]
)
