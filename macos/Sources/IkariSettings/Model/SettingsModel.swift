// 「入力」と「変換」のタブのモデル（ADR 0005 §6.2）。設定は殻に書いてもらい（書き手は殻だけ。§7.1）、
// 返ってきた「反映後の値」を画面に戻す。属性プロファイルはエンジンの管理の要求（ProfileGet / ProfileSet）。
// 値の読み方、範囲への丸め、変わったかどうか、Bundle ID の検証、キーの衝突、送る変更の組み立ては shell-core（ADR 0041 §2.3。
// `SettingsCore`）。ここは画面の状態（@Observable）と文言を持つ。
import Foundation
import IkariProto
import Observation

struct KeymapRow: Identifiable, Equatable {
    var action: KeymapAction
    /// 画面に出す名前
    var title: String
    /// 今の割り当て。プリセットと同じ値は `.preset` にそろえる
    var binding: KeymapBinding
    /// 割り当てのキー（`⌃J`、`なし`）
    var keyText: String
    /// プリセットの割り当て（上書き中に「戻す」と何になるか）。ないときは nil
    var presetText: String?

    var id: KeymapAction { action }
    var isOverridden: Bool { binding != .preset }
}

struct ProfileDomainRow: Identifiable, Equatable {
    var id: String
    var name: String
    var weight: ProfileWeight
}

@MainActor @Observable
final class SettingsModel {
    static let noneText = "なし"
    static let actionTitles: [KeymapAction: String] = [
        .toHiragana: "ひらがなに変換",
        .toFullwidthKatakana: "全角カタカナに変換",
        .toHalfwidthKatakana: "半角カタカナに変換",
        .toFullwidthAlnum: "全角英数に変換",
        .toHalfwidthAlnum: "半角英数に変換",
        .segmentFocusLeft: "文節: 左へ移動",
        .segmentFocusRight: "文節: 右へ移動",
        .segmentShrink: "文節: 縮める",
        .segmentExpand: "文節: 伸ばす",
        .candidateNext: "次の候補",
        .candidatePrevious: "前の候補",
        .reconvert: "再変換",
        .modeToggle: "モード: 日本語と英語を切り替え",
        .modeEnglish: "モード: 英語にする",
        .modeJapanese: "モード: 日本語にする",
        .clauseDelete: "直前の文節を削除",
        .clauseDeletePreview: "削除する文節を見てから削除",
    ]
    static let invalidBundleIDMessage = "Bundle ID には英数字、「.」「-」「_」だけを使えます（例: com.apple.Terminal）"
    static let coreUnavailableMessage = "設定を処理できませんでした。設定アプリを入れ直してください"

    private(set) var values = SettingsValues()
    /// キー割り当ての行（値が変わるたびに shell-core で作り直す）
    private(set) var keymapRows: [KeymapRow] = []
    private(set) var isLoaded = false
    private(set) var errorMessage: String?
    private(set) var profile: [ProfileDomainRow] = []
    private(set) var profileMessage: String?
    let presets: KeymapPresetTable
    private let bridge: any SettingsBridge
    /// 殻が返した設定の行（shell-core に渡す今の値）
    private var entries: [String: String] = [:]

    init(bridge: any SettingsBridge, presets: KeymapPresetTable = .bundled()) {
        self.bridge = bridge
        self.presets = presets
        show([:])
    }

    func dismissError() { errorMessage = nil }

    // MARK: 読み込み

    func load() async {
        await reloadSettings()
        await reloadProfile()
    }

    /// 殻の通知（`settings_changed`）でも呼ぶ
    func reloadSettings() async {
        do {
            show(try await bridge.getSettings())
            isLoaded = true
        } catch {
            errorMessage = Messages.describe(error)
        }
    }

    func reloadProfile() async {
        do {
            let body = try await bridge.engine(.getProfile(Ikari_Engine_V1_ProfileGet()))
            guard case .profileReply(let reply) = body else { throw BridgeClientError.unexpectedReply }
            profile = reply.domains.map(Self.row(from:))
            profileMessage = nil
        } catch {
            profileMessage = Messages.describe(error)
        }
    }

    /// 殻の返した行を画面の値とキー割り当ての行にする
    private func show(_ snapshot: [String: String]) {
        entries = snapshot
        guard let described = SettingsCore.describe(snapshot, presets: presets) else {
            values = SettingsValues.defaults
            keymapRows = []
            return
        }
        values = SettingsValues(message: described.values)
        keymapRows = described.rows.compactMap { row in
            guard let action = KeymapAction(rawValue: row.action) else { return nil }
            return KeymapRow(
                action: action, title: Self.actionTitles[action] ?? action.rawValue,
                binding: SettingsCore.binding(row.binding),
                keyText: row.hasKeyText ? row.keyText : Self.noneText,
                presetText: row.hasPresetText ? row.presetText : nil)
        }
    }

    // MARK: 設定の変更

    func setInputMethod(_ method: InputMethodKind) async { await apply(.inputMethod(SettingsCore.kind(method))) }
    func setKeymapPreset(_ preset: KeymapPreset) async { await apply(.keymapPreset(SettingsCore.kind(preset))) }
    func setPagingWrap(_ wrap: Bool) async { await apply(.pagingWrap(wrap)) }
    func setPageSize(_ size: Int) async { await apply(.pageSize(Int64(size))) }
    /// 殻が `engine.*` の変更を見て、エンジンを起こし直す（ADR 0005 §7.3）
    func setLMEnabled(_ enabled: Bool) async { await apply(.lmEnabled(enabled)) }
    func setIdleUnloadSecs(_ secs: Int) async { await apply(.idleUnloadSecs(Int64(secs))) }
    func setLMDeadlineMs(_ ms: Int) async { await apply(.lmDeadlineMs(Int64(ms))) }
    func setConvertWaitMs(_ ms: Int) async { await apply(.convertWaitMs(Int64(ms))) }
    func setClauseMinChars(_ count: Int) async { await apply(.clauseMinChars(Int64(count))) }

    // MARK: 郵便番号（ADR 0032）など、オン・オフの設定

    func setPostalJapanese(_ enabled: Bool) async { await apply(.postalJapanese(enabled)) }
    func setPostalEnglish(_ enabled: Bool) async { await apply(.postalEnglish(enabled)) }
    func setCandidateHideWhenConfident(_ enabled: Bool) async { await apply(.candidateHideWhenConfident(enabled)) }
    func setUnicodeNotation(_ enabled: Bool) async { await apply(.unicodeNotation(enabled)) }
    func setDateWeekdayDates(_ enabled: Bool) async { await apply(.dateWeekdayDates(enabled)) }
    func setPostalTePrefix(_ enabled: Bool) async { await apply(.postalTePrefix(enabled)) }

    // MARK: 絵文字のショートコード（ADR 0027 §8.1）

    func setEmojiSmallLetters(_ enabled: Bool) async { await apply(.emojiSmallLetters(enabled)) }
    func setEmojiShortcode(_ enabled: Bool) async { await apply(.emojiShortcode(enabled)) }

    /// ショートコードを使わないアプリに Bundle ID を足す
    func addEmojiShortcodeExclude(_ rawID: String) async { await apply(.addEmojiShortcodeExclude(rawID)) }

    func addEmojiShortcodeExclude(appAt url: URL) async {
        guard let id = bundleID(ofAppAt: url) else { return }
        await addEmojiShortcodeExclude(id)
    }

    private func bundleID(ofAppAt url: URL) -> String? {
        guard let id = ExcludedApps.bundleID(ofAppAt: url) else {
            errorMessage = "「\(url.deletingPathExtension().lastPathComponent)」の Bundle ID を読めませんでした。Bundle ID を直接入力してください"
            return nil
        }
        return id
    }

    func removeEmojiShortcodeExclude(_ id: String) async { await apply(.removeEmojiShortcodeExclude(id)) }
    /// 既定の一覧に戻す（設定の行を消す）
    func resetEmojiShortcodeExclude() async { await apply(.resetEmojiShortcodeExclude(Ikari_Shell_V1_Empty())) }

    /// 操作ごとの割り当てを変える。`.preset` は上書きの行を消す（`reset`）。ほかの操作が使うキーは断る
    func setKeymap(_ action: KeymapAction, to binding: KeymapBinding) async {
        var intent = Ikari_Settings_V1_KeymapIntent()
        intent.action = action.rawValue
        intent.binding = SettingsCore.message(binding)
        await apply(.keymap(intent))
    }

    /// 操作を shell-core で変更にして殻に送る。今の値と同じなら何も送らない
    private func apply(_ intent: Ikari_Settings_V1_Intent.OneOf_Kind) async {
        switch SettingsCore.plan(intent, entries: entries, presets: presets) {
        case .changes(let changes)?:
            await send(changes)
        case .unchanged?:
            return
        case .error(let error)?:
            errorMessage = describe(error)
        case nil:
            errorMessage = Self.coreUnavailableMessage
        }
    }

    private func describe(_ error: Ikari_Settings_V1_PlanError) -> String {
        switch error.kind {
        case .invalidBundleID?:
            return Self.invalidBundleIDMessage
        case .keyConflict(let conflict)?:
            let title = KeymapAction(rawValue: conflict.otherAction).flatMap { Self.actionTitles[$0] } ?? conflict.otherAction
            return "\(conflict.chordDisplay) は「\(title)」に割り当てられています"
        case .unknownAction?, nil:
            return Self.coreUnavailableMessage
        }
    }

    private func send(_ changes: Ikari_Shell_V1_SettingsChanges) async {
        do {
            let snapshot = try await bridge.updateSettings(
                set: changes.set.map { entry in
                    var out = SettingEntry()
                    out.key = entry.key
                    out.value = entry.value
                    return out
                },
                reset: changes.reset)
            show(snapshot)
            errorMessage = nil
        } catch {
            errorMessage = Messages.describe(error)
            // 画面を実際の値に戻す（エラーの文言は残す）
            let message = errorMessage
            await reloadSettings()
            if errorMessage == nil { errorMessage = message }
        }
    }

    // MARK: キー割り当て

    /// 今の割り当て。上書きがプリセットの割り当てと同じなら `.preset`（shell-core が行に入れた値）
    func resolvedBinding(for action: KeymapAction) -> KeymapBinding {
        keymapRows.first { $0.action == action }?.binding ?? .preset
    }

    // MARK: 属性プロファイル

    /// 分野ごとの重み。`ProfileSet` は全体を置き換えるので、全分野を送る（OFF は書かない。shell-core が組み立てる）
    func setWeight(_ weight: ProfileWeight, forDomain id: String) async {
        var request = Ikari_Settings_V1_ProfileSetRequest()
        request.rows = profile.map(Self.state(from:))
        request.id = id
        request.weight = Self.kind(from: weight)
        guard case .profileSet(let planned)? = SettingsCore.call(.profileSet(request)),
            case .request(let set)? = planned.result
        else { return }
        do {
            let body = try await bridge.engine(.setProfile(set))
            guard case .profileReply(let reply) = body else { throw BridgeClientError.unexpectedReply }
            profile = reply.domains.map(Self.row(from:))
            profileMessage = nil
        } catch {
            profileMessage = Messages.describe(error)
            await reloadProfile()
            if profileMessage == nil { profileMessage = Messages.describe(error) }
        }
    }

    static func row(from state: Ikari_Engine_V1_ProfileDomainState) -> ProfileDomainRow {
        ProfileDomainRow(id: state.id, name: state.name, weight: weight(from: state.weight))
    }

    static func state(from row: ProfileDomainRow) -> Ikari_Engine_V1_ProfileDomainState {
        var state = Ikari_Engine_V1_ProfileDomainState()
        state.id = row.id
        state.name = row.name
        state.weight = kind(from: row.weight)
        return state
    }

    /// proto の列挙と画面の列挙の写し（読めない値は OFF。shell-core の `profile_weight_from_proto` と同じ）
    static func weight(from kind: Ikari_Engine_V1_ProfileWeightKind) -> ProfileWeight {
        switch kind {
        case .profileWeightSometimes: .sometimes
        case .profileWeightOften: .often
        case .profileWeightOff, .profileWeightUnspecified, .UNRECOGNIZED: .off
        }
    }

    static func kind(from weight: ProfileWeight) -> Ikari_Engine_V1_ProfileWeightKind {
        switch weight {
        case .off: .profileWeightOff
        case .sometimes: .profileWeightSometimes
        case .often: .profileWeightOften
        }
    }
}
