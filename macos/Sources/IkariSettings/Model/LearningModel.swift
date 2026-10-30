// 「学習とデータ」タブのモデル（ADR 0005 §6.2、§8.4、§8.5）。
// 学習の件数、層ごとのリセット、エクスポート、インポート（確認してから上書き）、バックアップの場所。
//
// ファイルの受け渡し: エンジンは Sandbox の中にいて、利用者が選んだパスを開かない。選ぶのも読み書きするのも
// 設定アプリで、エンジンにはバイト列で渡す（NSSavePanel / NSOpenPanel は `DialogPresenting` の向こう）。
// 学習の中身は特徴量だけなので、画面には件数だけを出す（原文を作り出さない）。
import Foundation
import IkariProto
import Observation

/// 画面の操作（確認、ファイルの選択、Finder）。本物は AppKit（`AppKitDialogs`）、テストは台本どおりに答える偽物
@MainActor
protocol DialogPresenting {
    /// 確認のダイアログ。了承されたら true
    func confirm(_ request: ConfirmRequest) -> Bool
    /// 書き出し先を選ぶ（NSSavePanel）。やめたら nil
    func chooseSaveLocation(suggestedName: String) -> URL?
    /// 読み込むファイルを選ぶ（NSOpenPanel）。やめたら nil
    func chooseOpenFile() -> URL?
    func reveal(_ url: URL)
}

struct ConfirmRequest: Equatable {
    var title: String
    var message: String
    var confirmTitle: String
    /// 取り消せない（またはバックアップから戻す必要がある）操作。確認のボタンを破壊的な見た目にする
    var isDestructive: Bool
}

/// リセットの対象（エンジンの `LearningLayer` に対応）
enum ResetTarget: CaseIterable, Equatable {
    case usage, initial, all

    var layer: Ikari_Engine_V1_LearningLayer {
        switch self {
        case .usage: .usage
        case .initial: .initial
        case .all: .all
        }
    }

    var title: String {
        switch self {
        case .usage: "使用の学習を消す"
        case .initial: "初期学習を消す"
        case .all: "すべての学習を消す"
        }
    }

    var summary: String {
        switch self {
        case .usage: "変換の選択から学んだ内容（使用の層）"
        case .initial: "取り込んだ文章から作った初期学習（初期学習の層）"
        case .all: "使用の層と初期学習の層のすべて"
        }
    }
}

enum DataFileError: Error, Equatable {
    case tooLarge
    case empty
    case unreadable
}

enum DataFiles {
    /// インポートのファイルを読む。上限（64 MiB）を超えるもの、空のものは読まずに断る（大きさの判定は shell-core）
    static func readImport(at url: URL) throws -> Data {
        let size: Int
        do {
            size = try url.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0
        } catch {
            throw DataFileError.unreadable
        }
        try check(size: size)
        guard let data = try? Data(contentsOf: url, options: .mappedIfSafe) else { throw DataFileError.unreadable }
        try check(size: data.count)  // 読む間に変わった場合
        return data
    }

    private static func check(size: Int) throws {
        var request = Ikari_Settings_V1_ImportSizeRequest()
        request.size = UInt64(max(size, 0))
        guard case .importSize(let verdict)? = SettingsCore.call(.importSize(request)) else {
            throw DataFileError.unreadable
        }
        switch verdict {
        case .ok: return
        case .tooLarge: throw DataFileError.tooLarge
        case .empty: throw DataFileError.empty
        case .unspecified, .UNRECOGNIZED: throw DataFileError.unreadable
        }
    }
}

enum LearningTexts {
    /// 書き出しの既定の名前（`ikari-export-YYYYMMDD.tsv`。名前は shell-core が作る）。年月日は利用者の暦と時刻帯で数える
    static func exportFileName(date: Date, calendar: Calendar = .current) -> String {
        let parts = calendar.dateComponents([.year, .month, .day], from: date)
        var request = Ikari_Settings_V1_ExportFileNameRequest()
        request.year = UInt32(clamping: parts.year ?? 0)
        request.month = UInt32(clamping: parts.month ?? 0)
        request.day = UInt32(clamping: parts.day ?? 0)
        return SettingsCore.text(.exportFileName(request)) ?? fallbackExportFileName
    }

    /// shell-core を使えないときの名前（書き出し先の候補にすぎず、利用者が変えられる）
    private static let fallbackExportFileName = "ikari-export.tsv"

    static func resetConfirmation(_ target: ResetTarget, status: Ikari_Engine_V1_LearningStatusReply?) -> ConfirmRequest {
        let rows: String
        switch (target, status) {
        case (.usage, let status?): rows = "現在 \(status.usageRows) 行。"
        case (.initial, let status?): rows = "現在 \(status.initialRows) 行。"
        case (.all, let status?): rows = "現在 \(status.usageRows + status.initialRows) 行。"
        default: rows = ""
        }
        return ConfirmRequest(
            title: "\(target.summary)を消しますか？",
            message: "\(rows)消す前に自動でバックアップを取ります。戻すときは、バックアップのファイルをインポートします。確定済みの文字には影響しません。",
            confirmTitle: "消す", isDestructive: true)
    }

    static func importConfirmation(counts: Ikari_Engine_V1_DataCounts, fileName: String) -> ConfirmRequest {
        ConfirmRequest(
            title: "「\(fileName)」をインポートして上書きしますか？",
            message: "上書きされる内容: \(Messages.describe(counts))。いまのユーザー辞書、学習、分野はすべて置き換わります（マージはしません）。実行前に自動でバックアップを取ります。",
            confirmTitle: "上書きする", isDestructive: true)
    }

    static func describe(_ error: DataFileError) -> String {
        switch error {
        case .tooLarge: "ファイルが大きすぎます（上限 64 MiB）"
        case .empty: "ファイルが空です"
        case .unreadable: "ファイルを読めません"
        }
    }
}

@MainActor @Observable
final class LearningModel {
    /// エンジンのデータの場所（バックアップは `backups/`）。ADR 0005 §2
    static let backupsDirectoryName = "backups"

    private(set) var status: Ikari_Engine_V1_LearningStatusReply?
    private(set) var isBusy = false
    /// 失敗（画面に赤で出す文言）
    private(set) var message: String?
    /// 成功の知らせ
    private(set) var notice: String?

    private let bridge: any SettingsBridge
    private let dialogs: any DialogPresenting
    private let dataDirectory: URL
    private let now: @Sendable () -> Date

    init(
        bridge: any SettingsBridge, dialogs: any DialogPresenting,
        dataDirectory: URL = LearningModel.defaultDataDirectory(),
        now: @escaping @Sendable () -> Date = { Date() }
    ) {
        self.bridge = bridge
        self.dialogs = dialogs
        self.dataDirectory = dataDirectory
        self.now = now
    }

    static func defaultDataDirectory() -> URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Application Support/ikari-ime", isDirectory: true)
    }

    var backupsDirectory: URL { dataDirectory.appendingPathComponent(Self.backupsDirectoryName, isDirectory: true) }

    func dismissMessages() {
        message = nil
        notice = nil
    }

    // MARK: 件数

    func load() async {
        do {
            status = try await fetchStatus()
            message = nil
        } catch {
            message = Messages.describe(error)
        }
    }

    private func fetchStatus() async throws -> Ikari_Engine_V1_LearningStatusReply {
        guard case .learningStatusReply(let reply) = try await bridge.engine(.learningStatus(Ikari_Engine_V1_LearningStatus()))
        else { throw BridgeClientError.unexpectedReply }
        return reply
    }

    // MARK: リセット

    /// 確認してから消す。やめたときは何も送らない
    func reset(_ target: ResetTarget) async {
        guard !isBusy else { return }
        dismissMessages()
        guard dialogs.confirm(LearningTexts.resetConfirmation(target, status: status)) else { return }
        isBusy = true
        defer { isBusy = false }
        do {
            var request = Ikari_Engine_V1_LearningReset()
            request.layer = target.layer
            guard case .learningStatusReply(let reply) = try await bridge.engine(.resetLearning(request)) else {
                throw BridgeClientError.unexpectedReply
            }
            status = reply
            notice = "消去しました。" + backupNote(reply.backupFile)
        } catch {
            message = Messages.describe(error)
        }
    }

    // MARK: エクスポート

    func export() async {
        guard !isBusy else { return }
        dismissMessages()
        // 先に書き出し先を選ぶ（やめたらエンジンに何も頼まない。選んだ時点の最新の内容を書き出す）
        guard let destination = dialogs.chooseSaveLocation(suggestedName: LearningTexts.exportFileName(date: now())) else {
            return
        }
        isBusy = true
        defer { isBusy = false }
        do {
            guard case .exportReply(let reply) = try await bridge.engine(.exportData(Ikari_Engine_V1_ExportData())) else {
                throw BridgeClientError.unexpectedReply
            }
            do {
                try reply.content.write(to: destination, options: .atomic)
                // 学習データには確定文字列が入るので、本人だけが読めるようにする
                try FileManager.default.setAttributes(
                    [.posixPermissions: 0o600], ofItemAtPath: destination.path)
            } catch {
                message = "ファイルを書き込めませんでした"
                return
            }
            notice = "書き出しました（\(Messages.describe(reply.counts))）。原文は含まれません"
        } catch {
            message = Messages.describe(error)
        }
    }

    // MARK: インポート

    /// 選ぶ → 読む（上限 64 MiB）→ `dry_run` で検証して件数を出す → 確認 → 上書き
    func importFile() async {
        guard !isBusy else { return }
        dismissMessages()
        guard let source = dialogs.chooseOpenFile() else { return }
        let content: Data
        do {
            content = try DataFiles.readImport(at: source)
        } catch let error as DataFileError {
            message = LearningTexts.describe(error)
            return
        } catch {
            message = LearningTexts.describe(.unreadable)
            return
        }
        isBusy = true
        defer { isBusy = false }
        do {
            let check = try await sendImport(content, dryRun: true)
            if Messages.hasIssue(check.issue) {
                message = Messages.describe(check.issue, changedNothing: true)
                return
            }
            let request = LearningTexts.importConfirmation(counts: check.counts, fileName: source.lastPathComponent)
            guard dialogs.confirm(request) else { return }
            let result = try await sendImport(content, dryRun: false)
            if Messages.hasIssue(result.issue) {
                message = Messages.describe(result.issue, changedNothing: true)
                return
            }
            guard result.applied else {
                message = "インポートできませんでした。何も変更していません"
                return
            }
            notice = "インポートしました（\(Messages.describe(result.counts))）。" + backupNote(result.backupFile)
            status = try? await fetchStatus()
        } catch {
            message = Messages.describe(error)
        }
    }

    private func sendImport(_ content: Data, dryRun: Bool) async throws -> Ikari_Engine_V1_ImportReply {
        var request = Ikari_Engine_V1_ImportData()
        request.content = content
        request.dryRun = dryRun
        guard case .importReply(let reply) = try await bridge.engine(.importData(request)) else {
            throw BridgeClientError.unexpectedReply
        }
        return reply
    }

    // MARK: バックアップ

    private func backupNote(_ file: String) -> String {
        file.isEmpty ? "" : "実行前の状態を \(Self.backupsDirectoryName)/\(file) に保存しました。"
    }

    /// Finder で `backups/` を開く。まだなければ、データのディレクトリを開く
    func openBackupFolder() {
        let manager = FileManager.default
        if manager.fileExists(atPath: backupsDirectory.path) {
            dialogs.reveal(backupsDirectory)
        } else if manager.fileExists(atPath: dataDirectory.path) {
            dialogs.reveal(dataDirectory)
            notice = "バックアップはまだありません（インポートか学習のリセットの前に自動で作られます）"
        } else {
            message = "ikari のデータの場所が見つかりません"
        }
    }
}
