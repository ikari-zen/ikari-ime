// 設定の処理は shell-core（ikari_settings_*）を通すので、XCFramework のあるビルドだけで流す（ADR 0041 §2.3）
#if IKARI_HAS_SHELL_FFI
import Foundation
import IkariProto
import Testing

@testable import IkariSettings

/// 台本どおりに答えるダイアログ
@MainActor
final class ScriptedDialogs: DialogPresenting {
    var confirmAnswer = true
    var saveLocation: URL?
    var openFile: URL?
    private(set) var confirmations: [ConfirmRequest] = []
    private(set) var suggestedNames: [String] = []
    private(set) var revealed: [URL] = []

    func confirm(_ request: ConfirmRequest) -> Bool {
        confirmations.append(request)
        return confirmAnswer
    }

    func chooseSaveLocation(suggestedName: String) -> URL? {
        suggestedNames.append(suggestedName)
        return saveLocation
    }

    func chooseOpenFile() -> URL? { openFile }
    func reveal(_ url: URL) { revealed.append(url) }
}

private func temporaryDirectory() throws -> URL {
    let url = FileManager.default.temporaryDirectory.appendingPathComponent("ikari-settings-tests-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
    return url
}

@MainActor
private func makeModel(
    _ harness: Harness, dialogs: ScriptedDialogs, dataDirectory: URL = URL(fileURLWithPath: "/nonexistent")
) -> LearningModel {
    LearningModel(
        bridge: harness.client, dialogs: dialogs, dataDirectory: dataDirectory,
        now: { Date(timeIntervalSince1970: 1_800_000_000) })  // 2027-01-15 UTC
}

private func sampleState() -> FakeState {
    var state = FakeState()
    state.userDict = [userEntry(reading: "いかり", surface: "錨"), userEntry(reading: "さかな", surface: "魚")]
    state.usageRows = 40
    state.initialRows = 60
    state.profile = ["it": .profileWeightOften, "novel": .profileWeightSometimes]
    return state
}

@Suite @MainActor struct LearningModelTests {
    @Test func showsTheRowCounts() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = makeModel(harness, dialogs: ScriptedDialogs())
        await model.load()
        #expect(model.status?.usageRows == 1234)
        #expect(model.status?.initialRows == 5678)
        #expect(model.status?.maxRows == 200_000)
    }

    @Test func resetAsksFirstAndDoesNothingWhenDeclined() async throws {
        let harness = try await Harness.make(state: sampleState())
        defer { harness.close() }
        let dialogs = ScriptedDialogs()
        dialogs.confirmAnswer = false
        let model = makeModel(harness, dialogs: dialogs)
        await model.load()
        await model.reset(.usage)
        #expect(dialogs.confirmations.count == 1)
        #expect(dialogs.confirmations[0].isDestructive)
        #expect(dialogs.confirmations[0].message.contains("現在 40 行"))
        #expect(!harness.shell.state.snapshot.requestLog.contains("reset_learning"))
        #expect(model.status?.usageRows == 40)
    }

    @Test func resettingTheUsageLayerKeepsTheInitialLayer() async throws {
        let harness = try await Harness.make(state: sampleState())
        defer { harness.close() }
        let model = makeModel(harness, dialogs: ScriptedDialogs())
        await model.load()
        await model.reset(.usage)
        #expect(model.status?.usageRows == 0)
        #expect(model.status?.initialRows == 60)
        #expect(model.notice?.contains("backups/before-reset-") == true)
        #expect(model.message == nil)
    }

    @Test func resettingTheInitialAndAllLayers() async throws {
        let harness = try await Harness.make(state: sampleState())
        defer { harness.close() }
        let model = makeModel(harness, dialogs: ScriptedDialogs())
        await model.load()
        await model.reset(.initial)
        #expect(model.status?.usageRows == 40)
        #expect(model.status?.initialRows == 0)
        await model.reset(.all)
        #expect(model.status?.usageRows == 0)
    }

    @Test func exportWritesTheEngineBytesToTheChosenFile() async throws {
        let harness = try await Harness.make(state: sampleState())
        defer { harness.close() }
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let dialogs = ScriptedDialogs()
        dialogs.saveLocation = directory.appendingPathComponent("out.tsv")
        let model = makeModel(harness, dialogs: dialogs)
        await model.export()
        #expect(dialogs.suggestedNames.count == 1)
        #expect(dialogs.suggestedNames[0].hasPrefix("ikari-export-2027"))
        #expect(dialogs.suggestedNames[0].hasSuffix(".tsv"))
        let written = try String(contentsOf: directory.appendingPathComponent("out.tsv"), encoding: .utf8)
        #expect(written.hasPrefix("#ikari-export\tschema=1\n"))
        #expect(written.contains("dict\tいかり\t錨"))
        #expect(model.notice?.contains("辞書 2 語") == true)
        #expect(model.message == nil)
    }

    @Test func cancellingTheSavePanelDoesNotAskTheEngine() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let model = makeModel(harness, dialogs: ScriptedDialogs())
        await model.export()
        #expect(!harness.shell.state.snapshot.requestLog.contains("export_data"))
        #expect(model.message == nil && model.notice == nil)
    }

    @Test func anUnwritableDestinationIsReported() async throws {
        let harness = try await Harness.make()
        defer { harness.close() }
        let dialogs = ScriptedDialogs()
        dialogs.saveLocation = URL(fileURLWithPath: "/nonexistent-ikari-dir/out.tsv")
        let model = makeModel(harness, dialogs: dialogs)
        await model.export()
        #expect(model.message == "ファイルを書き込めませんでした")
    }

    @Test func exportThenImportRoundTripsThroughConfirmation() async throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let file = directory.appendingPathComponent("backup.tsv")

        let source = try await Harness.make(state: sampleState())
        defer { source.close() }
        let sourceDialogs = ScriptedDialogs()
        sourceDialogs.saveLocation = file
        await makeModel(source, dialogs: sourceDialogs).export()

        let target = try await Harness.make()
        defer { target.close() }
        let dialogs = ScriptedDialogs()
        dialogs.openFile = file
        let model = makeModel(target, dialogs: dialogs)
        await model.load()
        await model.importFile()

        // 確認のダイアログにはファイル名と上書きされる件数が出る（ADR 0005 §8.4）
        #expect(dialogs.confirmations.count == 1)
        #expect(dialogs.confirmations[0].title.contains("backup.tsv"))
        #expect(dialogs.confirmations[0].message.contains("辞書 2 語、学習 100 行、分野 2、取り込み元 0"))
        #expect(dialogs.confirmations[0].isDestructive)
        // 先に dry_run、了承のあとで実行
        let log = target.shell.state.snapshot.requestLog.filter { $0.hasPrefix("import") }
        #expect(log == ["import_data_dry_run", "import_data"])
        let state = target.shell.state.snapshot
        #expect(Set(state.userDict.map(\.surface)) == ["魚", "錨"])
        #expect(state.usageRows == 40 && state.initialRows == 60)
        #expect(model.status?.usageRows == 40)
        #expect(model.notice?.contains("before-import-") == true)
        #expect(model.message == nil)
    }

    @Test func declinedImportChangesNothing() async throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let file = directory.appendingPathComponent("in.tsv")
        try Data("#ikari-export\tschema=1\nusage\t5\n".utf8).write(to: file)

        let harness = try await Harness.make(state: sampleState())
        defer { harness.close() }
        let dialogs = ScriptedDialogs()
        dialogs.openFile = file
        dialogs.confirmAnswer = false
        let model = makeModel(harness, dialogs: dialogs)
        await model.importFile()
        #expect(harness.shell.state.snapshot.requestLog == ["import_data_dry_run"])
        #expect(harness.shell.state.snapshot.usageRows == 40)
        #expect(harness.shell.state.snapshot.userDict.count == 2)
    }

    @Test func aFileThatIsNotAnExportIsExplainedWithoutAskingForConfirmation() async throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let file = directory.appendingPathComponent("bad.tsv")
        try Data("hello\n".utf8).write(to: file)

        let harness = try await Harness.make(state: sampleState())
        defer { harness.close() }
        let dialogs = ScriptedDialogs()
        dialogs.openFile = file
        let model = makeModel(harness, dialogs: dialogs)
        await model.importFile()
        #expect(dialogs.confirmations.isEmpty)
        #expect(model.message == "ikari の書き出しファイルではありません。何も変更していません")
        #expect(harness.shell.state.snapshot.userDict.count == 2)
    }

    @Test func anUnsupportedSchemaIsExplainedAndNothingChanges() async throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let file = directory.appendingPathComponent("v2.tsv")
        try Data("#ikari-export\tschema=2\n".utf8).write(to: file)

        let harness = try await Harness.make(state: sampleState())
        defer { harness.close() }
        let dialogs = ScriptedDialogs()
        dialogs.openFile = file
        let model = makeModel(harness, dialogs: dialogs)
        await model.importFile()
        #expect(dialogs.confirmations.isEmpty)
        #expect(model.message?.contains("版 2") == true)
        #expect(model.message?.hasSuffix("何も変更していません") == true)
        #expect(harness.shell.state.snapshot.requestLog == ["import_data_dry_run"])
    }

    @Test func aBrokenLineIsLocated() async throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let file = directory.appendingPathComponent("broken.tsv")
        try Data("#ikari-export\tschema=1\nusage\t5\ngarbage\n".utf8).write(to: file)

        let harness = try await Harness.make()
        defer { harness.close() }
        let dialogs = ScriptedDialogs()
        dialogs.openFile = file
        let model = makeModel(harness, dialogs: dialogs)
        await model.importFile()
        #expect(model.message?.contains("3 行目") == true)
    }

    @Test func aFileOverTheLimitIsNeverSent() async throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let file = directory.appendingPathComponent("huge.tsv")
        FileManager.default.createFile(atPath: file.path, contents: nil)
        let handle = try FileHandle(forWritingTo: file)
        try handle.truncate(atOffset: UInt64(AdminLimits.maxImportBytes) + 1)  // 疎なファイル
        try handle.close()

        let harness = try await Harness.make()
        defer { harness.close() }
        let dialogs = ScriptedDialogs()
        dialogs.openFile = file
        let model = makeModel(harness, dialogs: dialogs)
        await model.importFile()
        #expect(model.message == "ファイルが大きすぎます（上限 64 MiB）")
        #expect(harness.shell.state.snapshot.requestLog.isEmpty)
    }

    @Test func anEmptyFileIsRefused() async throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let file = directory.appendingPathComponent("empty.tsv")
        try Data().write(to: file)
        let harness = try await Harness.make()
        defer { harness.close() }
        let dialogs = ScriptedDialogs()
        dialogs.openFile = file
        let model = makeModel(harness, dialogs: dialogs)
        await model.importFile()
        #expect(model.message == "ファイルが空です")
    }

    @Test func backupFolderOpensBackupsThenTheDataDirectoryThenReportsMissing() async throws {
        let directory = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let harness = try await Harness.make()
        defer { harness.close() }

        let dialogs = ScriptedDialogs()
        let missing = makeModel(harness, dialogs: dialogs, dataDirectory: directory.appendingPathComponent("none"))
        missing.openBackupFolder()
        #expect(missing.message != nil)
        #expect(dialogs.revealed.isEmpty)

        let model = makeModel(harness, dialogs: dialogs, dataDirectory: directory)
        model.openBackupFolder()
        #expect(dialogs.revealed == [directory])
        #expect(model.notice?.contains("まだありません") == true)

        try FileManager.default.createDirectory(at: model.backupsDirectory, withIntermediateDirectories: true)
        model.openBackupFolder()
        #expect(dialogs.revealed.last == model.backupsDirectory)
    }
}

@Suite struct LearningTextsTests {
    @Test func exportFileNameIsDated() {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "Asia/Tokyo")!
        let date = Date(timeIntervalSince1970: 1_800_000_000)  // 2027-01-15 08:00 JST
        #expect(LearningTexts.exportFileName(date: date, calendar: calendar) == "ikari-export-20270115.tsv")
    }
}

#endif
