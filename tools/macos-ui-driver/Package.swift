// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "macos-ui-driver",
    platforms: [.macOS(.v13)],
    targets: [
        .executableTarget(
            name: "macos-ui-driver",
            path: "Sources/macos-ui-driver"
        )
    ]
)
