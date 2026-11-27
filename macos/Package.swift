// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "LedgeGrabMac",
    platforms: [.macOS(.v14)],
    products: [.executable(name: "LedgeGrab", targets: ["LedgeGrab"])],
    targets: [
        .target(name: "LedgeCaptureCore"),
        .executableTarget(name: "LedgeGrab", dependencies: ["LedgeCaptureCore"]),
        .testTarget(name: "LedgeCaptureCoreTests", dependencies: ["LedgeCaptureCore"])
    ]
)
