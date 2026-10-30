// shell-ffi の XCFramework にリンクして、殻の口（ADR 0041 §4.4）を 1 回呼べることを確かめる（契約 M1-b）。
// XCFramework がないビルド（macos/scripts/build.sh の前）では飛ばす。
import Foundation
import IkariProto
import Testing

#if IKARI_HAS_SHELL_FFI
@testable import IkariShellABI

/// ctx が Rust に渡り、release で返ることを数える
private final class Released: @unchecked Sendable {
    private let lock = NSLock()
    private var _count = 0
    var count: Int { lock.withLock { _count } }
    func bump() { lock.withLock { _count += 1 } }
}

private final class Probe {
    let released: Released
    init(_ released: Released) { self.released = released }
    deinit { released.bump() }
}

private func host(_ probe: Probe? = nil) -> ShellHost {
    ShellHost(
        surrounding: { _ = probe; return nil },
        selection: { nil },
        engineConnect: { 0 },
        engineRequest: { _, _, _ in .unavailable },
        enginePost: { _, _ in },
        engineClose: { _ in },
        enginePid: { _ in -1 }
    )
}

@Suite struct ShellABITests {
    @Test func dispatchRoundTripsOnce() throws {
        let shell = try ShellABI(shellVersion: "test", settingsPath: "", resourcesDir: "", host: host())
        var event = Ikari_Shell_V1_ShellEvent()
        var key = Ikari_Shell_V1_KeyInput()
        key.kind = .space
        event.key = key
        let result = shell.dispatch(event)
        #expect(result.status == .ok)
        // Shell::dispatch は L-A2 まで stub（空の列）
        #expect(result.actions.isEmpty)
        shell.downNotifier.engineDown(generation: 1, kind: .invalidated)
    }

    @Test func invalidBytesPassTheKeyThrough() throws {
        let shell = try ShellABI(shellVersion: "test", settingsPath: "", resourcesDir: "", host: host())
        let result = shell.dispatch(bytes: Data([0xff, 0xff, 0xff]))
        #expect(result.status == .decode)
        #expect(result.actions.count == 1)
        #expect(result.actions.first?.passThrough != nil)
    }

    @Test func hostIsReleasedWhenTheShellIsFreed() throws {
        let released = Released()
        var notifier: ShellDownNotifier?
        do {
            let shell = try ShellABI(
                shellVersion: "test", settingsPath: "", resourcesDir: "", host: host(Probe(released)))
            notifier = shell.downNotifier
            #expect(released.count == 0)
        }
        #expect(released.count == 1)
        // 解放のあとの通知は何もしない
        notifier?.engineDown(generation: 1, kind: .interrupted)
    }
}
#else
@Suite struct ShellABITests {
    @Test(.disabled("shell-ffi の XCFramework がない（macos/scripts/build.sh で作る）"))
    func requiresTheFramework() {}
}
#endif
