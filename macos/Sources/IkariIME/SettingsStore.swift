// settings.tsv の読み書き（ADR 0005 §7.1、§7.2）。**ファイルを書くのは殻だけ**で、メニューの操作も設定アプリの
// `UpdateSettings` も、ここを通る。書き手を 1 つにすると、同時の書き込みの競合と、反映の通知の漏れがなくなる。
// 手で編集してもよい（`reloadIfFileChanged` が mtime と大きさで気づく）。
// ログには設定のキー名と件数だけを書く（値は書かない。NFR-19）。
import Foundation
import IkariShellCore

final class SettingsStore: @unchecked Sendable {
    /// 殻の今の設定。`version` は `settings` が変わるたびに増える（コントローラが適用済みの版と比べる）
    struct Snapshot: Sendable {
        var settings: Settings
        var keymap: Keymap
        var version: UInt64
    }

    enum UpdateError: Error, Equatable {
        /// 書いてはいけない（未知のキー、不正な値、行を混ぜるおそれのある文字）。`key` は空のこともある
        case invalid(key: String)
        case writeFailed
    }

    /// ファイルの変わった目印（mtime と大きさ。stat 1 回で取れる）
    private struct FileStamp: Equatable {
        var modified: Date
        var size: Int
    }

    private let fileURL: URL
    private let loadKeymap: @Sendable (Settings) -> Keymap
    private let lock = NSLock()
    private var current: Snapshot
    private var stamp: FileStamp?

    /// 設定が変わったあと（自分で書いたとき、手の編集に気づいたとき）に、ロックの外で呼ぶ。起動直後に 1 度だけ設定する
    var onChange: (@Sendable (_ old: Settings, _ new: Settings) -> Void)?

    init(fileURL: URL, loadKeymap: @escaping @Sendable (Settings) -> Keymap) {
        self.fileURL = fileURL
        self.loadKeymap = loadKeymap
        let text = Self.readText(fileURL)
        let parsed = Settings.parse(tsv: text ?? "")
        if !parsed.warnings.isEmpty {
            AppContext.log.warning("settings.tsv: \(parsed.warnings.count) lines ignored")
        }
        current = Snapshot(settings: parsed.settings, keymap: loadKeymap(parsed.settings), version: 0)
        stamp = Self.stamp(of: fileURL)
    }

    var snapshot: Snapshot {
        lock.lock()
        defer { lock.unlock() }
        return current
    }

    var settings: Settings {
        lock.lock()
        defer { lock.unlock() }
        return current.settings
    }

    // MARK: 手の編集に気づく

    /// ファイルの mtime か大きさが変わっていれば読み直す（`activateServer` で呼ぶ。stat 1 回なので打鍵には影響しない）。
    func reloadIfFileChanged() {
        let latest = Self.stamp(of: fileURL)
        lock.lock()
        guard latest != stamp else {
            lock.unlock()
            return
        }
        stamp = latest
        let parsed = Settings.parse(tsv: Self.readText(fileURL) ?? "")
        let old = current.settings
        let changed = adopt(parsed.settings)
        lock.unlock()
        if !parsed.warnings.isEmpty {
            AppContext.log.warning("settings.tsv: \(parsed.warnings.count) lines ignored")
        }
        if changed { onChange?(old, parsed.settings) }
    }

    // MARK: 書く

    /// `set` の値を書き、`reset` のキーの行を消す（既定に戻す）。行を保ったまま値だけを差し替える（`SettingsFile.update`）。
    /// 書く前に、キーと値を検査し、結果の警告が増えていないことを確かめる（増えたら書かない）。
    /// 一時ファイルに書いて rename する（`Data.write(.atomic)`）。成功したら新しい状態を返す。
    @discardableResult
    func update(set: [(key: String, value: String)], reset: [String]) throws -> Snapshot {
        for entry in set where !Settings.isAcceptable(key: entry.key, value: entry.value) {
            throw UpdateError.invalid(key: Settings.isKnownKey(entry.key) ? entry.key : "")
        }
        for key in reset where !Settings.isKnownKey(key) {
            throw UpdateError.invalid(key: "")
        }

        lock.lock()
        let old = current.settings
        let text = Self.readText(fileURL) ?? ""
        let updated = SettingsFile.update(text: text, set: set, reset: reset)
        let before = Settings.parse(tsv: text).warnings.count
        let result = Settings.parse(tsv: updated)
        guard result.warnings.count <= before else {
            lock.unlock()
            throw UpdateError.invalid(key: "")
        }
        if updated != text {
            do {
                try FileManager.default.createDirectory(
                    at: fileURL.deletingLastPathComponent(), withIntermediateDirectories: true)
                try Data(updated.utf8).write(to: fileURL, options: .atomic)
                try FileManager.default.setAttributes(
                    [.posixPermissions: 0o600], ofItemAtPath: fileURL.path)
            } catch {
                lock.unlock()
                AppContext.log.error("settings.tsv: write failed")
                throw UpdateError.writeFailed
            }
            stamp = Self.stamp(of: fileURL)
        }
        let changed = adopt(result.settings)
        let snapshot = current
        lock.unlock()

        if changed { onChange?(old, result.settings) }
        return snapshot
    }

    // MARK: 内部

    /// `lock` を持っているときに呼ぶ。設定が違えば、キー割り当てを読み直して版を進める
    private func adopt(_ settings: Settings) -> Bool {
        guard settings != current.settings else { return false }
        current = Snapshot(settings: settings, keymap: loadKeymap(settings), version: current.version + 1)
        return true
    }

    private static func readText(_ url: URL) -> String? {
        try? String(contentsOf: url, encoding: .utf8)
    }

    private static func stamp(of url: URL) -> FileStamp? {
        guard let attributes = try? FileManager.default.attributesOfItem(atPath: url.path),
            let modified = attributes[.modificationDate] as? Date,
            let size = attributes[.size] as? Int
        else { return nil }
        return FileStamp(modified: modified, size: size)
    }
}

/// キー割り当てのプリセットの読み込み（同梱の `keymap/<preset>.tsv`）
enum KeymapLoader {
    static let resourceDirectory = "keymap"
    static let resourceExtension = "tsv"

    /// プリセットの資源が見つからないときの最低限の割り当て（F6〜F10。入力は止めない）
    private static let functionKeysOnlyPreset = """
        f6\tconvert.hiragana
        f7\tconvert.fullwidth_katakana
        f8\tconvert.halfwidth_katakana
        f9\tconvert.fullwidth_alnum
        f10\tconvert.halfwidth_alnum
        """

    static func load(settings: Settings, bundle: Bundle = .main) -> Keymap {
        let url = bundle.url(
            forResource: settings.keymapPreset.rawValue, withExtension: resourceExtension, subdirectory: resourceDirectory)
        let presetText = url.flatMap { try? String(contentsOf: $0, encoding: .utf8) } ?? functionKeysOnlyPreset
        if let keymap = try? Keymap(presetTSV: presetText, settings: settings) { return keymap }
        AppContext.log.error("keymap preset is broken; using function keys only")
        return (try? Keymap(presetTSV: functionKeysOnlyPreset, settings: settings)) ?? Keymap()
    }
}
