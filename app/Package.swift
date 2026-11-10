// swift-tools-version:6.0
// Berth app shell. Build with `make bootstrap && make build` (see docs/05-build.md).
//
// GhosttyKit.xcframework is produced by the pinned Ghostty submodule and is not
// committed. `swift build` fails until `make bootstrap` has run at least once.

import PackageDescription

let package = Package(
    name: "Berth",
    platforms: [.macOS(.v14)],
    products: [
        .executable(name: "Berth", targets: ["Berth"])
    ],
    targets: [
        .binaryTarget(
            name: "GhosttyKit",
            path: "../vendor/ghostty/macos/GhosttyKit.xcframework"
        ),
        .executableTarget(
            name: "Berth",
            dependencies: ["GhosttyKit"],
            path: "Sources/Berth"
        ),
        .testTarget(
            name: "BerthTests",
            dependencies: ["Berth"],
            path: "Tests/BerthTests"
        ),
    ]
)
