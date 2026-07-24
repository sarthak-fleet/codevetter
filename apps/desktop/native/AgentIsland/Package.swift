// swift-tools-version: 5.9

import PackageDescription

let package = Package(
    name: "CodeVetterAgentIsland",
    platforms: [
        .macOS(.v12),
    ],
    products: [
        .executable(
            name: "codevetter-agent-island",
            targets: ["CodeVetterAgentIsland"]
        ),
    ],
    targets: [
        .executableTarget(
            name: "CodeVetterAgentIsland",
            path: "Sources"
        ),
    ],
    swiftLanguageVersions: [.v5]
)
