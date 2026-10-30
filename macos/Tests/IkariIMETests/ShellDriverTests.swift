// `ShellDriver`（IMK の呼び出し → shell-core の 1 本の口 → 動作の実行）を、本物の shell-core（FFI）と替え玉のクライアントで確かめる
// （ADR 0041 L-E）。変換の振る舞いそのものは共通のシナリオ（shell-core/scenarios）が見るので、ここは配線だけを見る。
// shell-ffi の XCFramework がないビルドでは飛ばす。
import Foundation
import IkariProto
import IkariShellABI
import InputMethodKit
import Testing

@testable import IkariIME

#if IKARI_HAS_SHELL_FFI

/// エンジンにつながらない（変換は殻の代わりの候補になる）
private struct NoEngine: EngineHost {
    func connect() -> UInt64 { 0 }
    func request(_ connection: UInt64, _ bytes: Data, timeoutMs: UInt32) -> ShellTransportResult { .unavailable }
    func post(_ connection: UInt64, _ bytes: Data) {}
    func close(_ connection: UInt64) {}
    func pid(_ connection: UInt64) -> Int32 { 0 }
}

private final class FakeSession: ShellSession {
    let client: FakeTextClient?
    private(set) var selectedModes: [String] = []
    init(_ client: FakeTextClient?) { self.client = client }
    var textClient: (any IMKTextInput)? { client }
    func selectMode(_ modeID: String) { selectedModes.append(modeID) }
}

private func makeDriver() throws -> ShellDriver {
    let dir = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("ikari-driver-\(UUID().uuidString.prefix(6))")
    try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    return ShellDriver(
        config: .init(shellVersion: "t", settingsPath: dir.appendingPathComponent("settings.tsv").path, resourcesDir: ""),
        engine: NoEngine())
}

private func key(_ kind: Ikari_Shell_V1_KeyKind, text: String = "", jis: UInt32? = nil) -> Ikari_Shell_V1_KeyInput {
    var input = Ikari_Shell_V1_KeyInput()
    input.kind = kind
    input.text = text
    if let jis {
        input.jis = jis
        input.layoutText = text
        input.text = ""
    }
    return input
}

@Suite(.serialized) struct ShellDriverTests {
    @Test func romajiKeysMarkTextInTheClientAndEnterCommitsIt() throws {
        let driver = try makeDriver()
        let client = FakeTextClient(document: "")
        let session = FakeSession(client)
        driver.activate(session)
        #expect(driver.key(key(.character, text: "a"), session: session))
        #expect(client.document == "あ")
        #expect(client.markedRange().length == 1)
        #expect(driver.key(key(.enter), session: session))
        #expect(client.document == "あ")
        #expect(client.markedRange().location == NSNotFound)
        // 何も入力していないときの Enter はアプリへ
        #expect(!driver.key(key(.enter), session: session))
    }

    @Test func jisPositionKeysAreReadAsLayoutCharactersInRomajiInput() throws {
        let driver = try makeDriver()
        let client = FakeTextClient(document: "")
        let session = FakeSession(client)
        driver.activate(session)
        // かな入力でないので、JIS の位置の番号ではなく配列の文字（`k`、`a`）で読む
        #expect(driver.key(key(.jis, text: "k", jis: 32), session: session))
        #expect(driver.key(key(.jis, text: "a", jis: 22), session: session))
        #expect(client.document == "か")
    }

    @Test func eisuAndKanaKeysAreAbsorbed() throws {
        let driver = try makeDriver()
        let session = FakeSession(FakeTextClient(document: ""))
        driver.activate(session)
        #expect(driver.key(key(.eisu), session: session))
        #expect(driver.key(key(.kana), session: session))
    }

    @Test func aKeyWithoutAClientGoesToTheApp() throws {
        let driver = try makeDriver()
        let session = FakeSession(nil)
        #expect(!driver.key(key(.character, text: "a"), session: session))
    }

    @Test func englishModeCommitsTheCompositionAndPassesLetters() throws {
        let driver = try makeDriver()
        let client = FakeTextClient(document: "")
        let session = FakeSession(client)
        driver.activate(session)
        _ = driver.key(key(.character, text: "a"), session: session)
        driver.inputModeChanged("io.github.ikari-zen.inputmethod.ikari.English", session: session)
        // 合成中の「あ」は、今の表示のまま確定してから切り替わる
        #expect(client.document == "あ")
        #expect(client.markedRange().location == NSNotFound)
        #expect(!driver.key(key(.character, text: "a"), session: session))
    }

    @Test func theMenuIsPublishedBeforeAnyKey() throws {
        let driver = try makeDriver()
        let menu = try #require(driver.currentMenu())
        #expect(menu.items.contains { $0.kind == .radio })
    }

    @Test func deactivatingAnotherSessionDoesNotTouchTheActiveOne() throws {
        let driver = try makeDriver()
        let first = FakeSession(FakeTextClient(document: ""))
        let secondClient = FakeTextClient(document: "")
        let second = FakeSession(secondClient)
        driver.activate(first)
        driver.activate(second)
        _ = driver.key(key(.character, text: "a"), session: second)
        // 遅れて届いた前の入力先の `deactivateServer` は、今の入力先の合成を確定しない
        driver.deactivate(first)
        #expect(secondClient.markedRange().length == 1)
    }
}

#endif
