import Foundation
import IkariBridge
import IkariProto
import IkariShellCore

@testable import IkariSettings

/// 偽の殻につないだ `BridgeClient`
struct Harness {
    let client: BridgeClient
    let shell: FakeShell

    static func make(
        state: FakeState = FakeState(), acceptedBridgeVersion: UInt32 = BridgeProtocol.version
    ) async throws -> Harness {
        let (appSide, shellSide) = try FileDescriptorTransport.socketPair()
        let shell = FakeShell(transport: shellSide, state: state, acceptedBridgeVersion: acceptedBridgeVersion)
        shell.start()
        let client = try await BridgeClient.connect(transport: appSide, appVersion: "test")
        return Harness(client: client, shell: shell)
    }

    func close() {
        client.close()
        shell.stop()
    }
}

func entry(_ key: String, _ value: String) -> SettingEntry {
    var entry = SettingEntry()
    entry.key = key
    entry.value = value
    return entry
}

func userEntry(
    reading: String, surface: String, pos: String = "名詞", comment: String = ""
) -> Ikari_Engine_V1_UserDictEntry {
    var entry = Ikari_Engine_V1_UserDictEntry()
    entry.reading = reading
    entry.surface = surface
    entry.pos = pos
    entry.comment = comment
    return entry
}
