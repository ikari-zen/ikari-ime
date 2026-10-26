// swift-tools-version: 6.2
// ikari の macOS 版（ADR 0004 §6.2）。ターゲット名とファイルの持ち主は ADR 0004 §11.2、ADR 0005 §11.3 に従う。
import Foundation
import PackageDescription

// XCFramework は macos/scripts/build.sh が cargo で作る（gitignore）。ないときは FFI に依存するターゲットを
// FFI なしでビルドできる形（スタブ）にして、`swift build` / `swift test` を XCFramework なしで通す。
let packageRoot = URL(fileURLWithPath: #filePath).deletingLastPathComponent().path
func hasFramework(_ name: String) -> Bool {
    FileManager.default.fileExists(atPath: "\(packageRoot)/Frameworks/\(name).xcframework")
}
let hasShellFFI = hasFramework("IkariShellFFI")
let hasEngineFFI = hasFramework("IkariEngineFFI")

// Rust の静的ライブラリ（llama.cpp を含む）が必要とするシステムのライブラリ（ADR 0004 §9.2-3）
let rustNativeLibs: [LinkerSetting] = [
    .linkedFramework("Foundation"),
    .linkedFramework("Metal"),
    .linkedFramework("MetalKit"),
    .linkedFramework("Accelerate"),
    .linkedLibrary("c++"),
]

var targets: [Target] = [
    // 入力の状態機械、キー割り当て、配置の計算、EngineClient プロトコル、設定。AppKit と IMK に依存しない
    .target(name: "IkariShellCore"),
    // 生成した protobuf（macos/scripts/gen-proto.sh。コミットする）
    .target(
        name: "IkariProto",
        dependencies: [.product(name: "SwiftProtobuf", package: "swift-protobuf")]
    ),
    // EngineClient の XPC 実装、署名要件
    .target(name: "IkariXPC", dependencies: ["IkariShellCore", "IkariProto"]),
    // IMKServer、IkariInputController、CandidatePanel
    .executableTarget(
        name: "IkariIME",
        dependencies: ["IkariShellCore", "IkariXPC", "IkariBridge", "IkariProto"] + (hasShellFFI ? ["IkariShellFFI"] : []),
        swiftSettings: [.define("IKARI_HAS_BRIDGE")] + (hasShellFFI ? [.define("IKARI_HAS_SHELL_FFI")] : [])
    ),
    // 設定アプリと殻のブリッジ（ADR 0005 §6.3）。枠の読み書き、BridgeTransport、中継の許可リスト。
    // 契約コミットで完成させてあり、以後はレーン S3 も U も変えない
    .target(name: "IkariBridge", dependencies: ["IkariShellCore", "IkariProto"]),
    // 設定アプリ（ADR 0005 §6.2）。レーン U が実装する。エンジンには直接つながない（殻の中継だけ）
    .executableTarget(name: "IkariSettings", dependencies: ["IkariShellCore", "IkariBridge", "IkariProto", "IkariXPC"]),
    // XPC サービス（FFI を呼ぶだけ）
    .executableTarget(
        name: "IkariEngineService",
        dependencies: ["IkariProto"] + (hasEngineFFI ? ["IkariEngineFFI"] : []),
        swiftSettings: hasEngineFFI ? [.define("IKARI_HAS_ENGINE_FFI")] : [],
        linkerSettings: hasEngineFFI ? rustNativeLibs : []
    ),
    // XPC 経由の E2E
    .executableTarget(name: "IkariSmoke", dependencies: ["IkariShellCore", "IkariXPC", "IkariProto"]),
    .testTarget(name: "IkariIMETests", dependencies: ["IkariIME", "IkariBridge", "IkariProto", "IkariShellCore", "IkariXPC"]),
    .testTarget(name: "IkariShellCoreTests", dependencies: ["IkariShellCore"]),
    .testTarget(name: "IkariBridgeTests", dependencies: ["IkariBridge", "IkariProto", "IkariShellCore"]),
    .testTarget(name: "IkariSettingsTests", dependencies: ["IkariSettings", "IkariBridge", "IkariProto", "IkariShellCore", "IkariXPC"]),
    .testTarget(name: "IkariXPCTests", dependencies: ["IkariXPC", "IkariShellCore", "IkariProto"]),
]

if hasShellFFI {
    targets.append(.binaryTarget(name: "IkariShellFFI", path: "Frameworks/IkariShellFFI.xcframework"))
}
if hasEngineFFI {
    targets.append(.binaryTarget(name: "IkariEngineFFI", path: "Frameworks/IkariEngineFFI.xcframework"))
}

let package = Package(
    name: "ikari",
    platforms: [.macOS("26.0")],
    products: [
        .library(name: "IkariShellCore", targets: ["IkariShellCore"]),
        .executable(name: "ikari", targets: ["IkariIME"]),
        .executable(name: "IkariEngine", targets: ["IkariEngineService"]),
        .executable(name: "ikari-smoke", targets: ["IkariSmoke"]),
        .executable(name: "IkariSettings", targets: ["IkariSettings"]),
    ],
    dependencies: [
        // 1.38.x に固定する（ADR 0004 §1）
        .package(url: "https://github.com/apple/swift-protobuf.git", .upToNextMinor(from: "1.38.0")),
    ],
    targets: targets
)
