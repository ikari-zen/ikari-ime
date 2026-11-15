// 設定の LM の案内（#425）。出すかどうかと文は shell-core が決めるので、ここは「Hello の返事を渡して出る・出ない」だけを確かめる
#if IKARI_HAS_SHELL_FFI
import Foundation
import IkariProto
import Testing

@testable import IkariSettings

@Suite @MainActor struct LMNoticeTests {
    private func notice(device: Ikari_Engine_V1_LmDevice, lm: String) async throws -> String? {
        let harness = try await Harness.make()
        defer { harness.close() }
        harness.shell.state.withLock {
            $0.lmDevice = device
            $0.settings["engine.lm"] = lm
        }
        return await makeSettingsModel(harness).lmNotice
    }

    @Test func showsTheNoticeWhenTheLMRunsOnTheCPU() async throws {
        let text = try await notice(device: .cpu, lm: "on")
        #expect(text?.contains("CPU") == true)
    }

    @Test func hidesItOnTheGPU() async throws {
        #expect(try await notice(device: .gpu, lm: "on") == nil)
    }

    @Test func hidesItWhenTheLMIsOff() async throws {
        #expect(try await notice(device: .cpu, lm: "off") == nil)
    }

    @Test func hidesItWhenTheDeviceIsUnknown() async throws {
        #expect(try await notice(device: .unspecified, lm: "on") == nil)
    }
}

#endif
