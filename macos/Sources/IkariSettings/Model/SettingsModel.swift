// 「入力」と「変換」のタブのモデル（ADR 0005 §6.2）。設定は殻に書いてもらい（書き手は殻だけ。§7.1）、
// 返ってきた「反映後の値」を画面に戻す。属性プロファイルはエンジンの管理の要求（ProfileGet / ProfileSet）。
import Foundation
import IkariProto
import IkariShellCore
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
    ]

    private(set) var values = SettingsValues()
    private(set) var isLoaded = false
    private(set) var errorMessage: String?
    private(set) var profile: [ProfileDomainRow] = []
    private(set) var profileMessage: String?
    let presets: KeymapPresetTable
    private let bridge: any SettingsBridge

    init(bridge: any SettingsBridge, presets: KeymapPresetTable = .bundled()) {
        self.bridge = bridge
        self.presets = presets
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
            values = SettingsValues(snapshot: try await bridge.getSettings())
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

    // MARK: 設定の変更

    func setInputMethod(_ method: InputMethodKind) async {
        guard method != values.inputMethod else { return }
        await apply(set: [(SettingKey.inputMethod, method.rawValue)])
    }

    func setKeymapPreset(_ preset: KeymapPreset) async {
        guard preset != values.keymapPreset else { return }
        await apply(set: [(SettingKey.keymapPreset, preset.rawValue)])
    }

    func setPagingWrap(_ wrap: Bool) async {
        guard wrap != values.pagingWrap else { return }
        await apply(set: [(SettingKey.pagingWrap, wrap ? "true" : "false")])
    }

    func setPageSize(_ size: Int) async {
        let clamped = min(max(size, Settings.pageSizeRange.lowerBound), Settings.pageSizeRange.upperBound)
        guard clamped != values.pageSize else { return }
        await apply(set: [(SettingKey.pagingSize, String(clamped))])
    }

    /// 殻が `engine.*` の変更を見て、エンジンを起こし直す（ADR 0005 §7.3）
    func setLMEnabled(_ enabled: Bool) async {
        guard enabled != values.lmEnabled else { return }
        await apply(set: [(SettingKey.engineLM, enabled ? EngineToggle.on : EngineToggle.off)])
    }

    func setIdleUnloadSecs(_ secs: Int) async {
        let range = SettingsValues.idleUnloadRange
        let clamped = min(max(secs, range.lowerBound), range.upperBound)
        guard clamped != values.idleUnloadSecs else { return }
        await apply(set: [(SettingKey.engineIdleUnloadSecs, String(clamped))])
    }

    func setLMDeadlineMs(_ ms: Int) async {
        let range = SettingsValues.lmDeadlineRange
        let clamped = min(max(ms, range.lowerBound), range.upperBound)
        guard clamped != values.lmDeadlineMs else { return }
        await apply(set: [(SettingKey.engineLMDeadlineMs, String(clamped))])
    }

    func setConvertWaitMs(_ ms: Int) async {
        let range = SettingsValues.convertWaitRange
        let clamped = min(max(ms, range.lowerBound), range.upperBound)
        guard clamped != values.convertWaitMs else { return }
        await apply(set: [(SettingKey.engineConvertWaitMs, String(clamped))])
    }

    /// 文脈を貸し借りしないアプリに Bundle ID を足す。空白は削る。使えない文字があれば足さずに、画面に断りを出す
    func addSharedExclude(_ rawID: String) async {
        guard let id = validBundleID(rawID), !values.contextSharedExclude.contains(id) else { return }
        await setSharedExclude(values.contextSharedExclude + [id])
    }

    /// 選んだアプリ（`.app`）の Bundle ID を足す。Bundle ID を読めなければ足さずに、画面に断りを出す
    func addSharedExclude(appAt url: URL) async {
        guard let id = bundleID(ofAppAt: url) else { return }
        await addSharedExclude(id)
    }

    /// 入力欄の Bundle ID を確かめる。空は無視（nil）。使えない文字があれば画面に断りを出して nil
    private func validBundleID(_ rawID: String) -> String? {
        let text = rawID.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return nil }
        guard let parsed = Settings.parseBundleIDList(text), parsed.count == 1, let id = parsed.first else {
            errorMessage = "Bundle ID には英数字、「.」「-」「_」だけを使えます（例: com.apple.Terminal）"
            return nil
        }
        return id
    }

    private func bundleID(ofAppAt url: URL) -> String? {
        guard let id = SharedExcludeApps.bundleID(ofAppAt: url) else {
            errorMessage = "「\(url.deletingPathExtension().lastPathComponent)」の Bundle ID を読めませんでした。Bundle ID を直接入力してください"
            return nil
        }
        return id
    }

    // MARK: 郵便番号（ADR 0032）

    func setPostalJapanese(_ enabled: Bool) async {
        guard enabled != values.postalJapanese else { return }
        await apply(set: [(SettingKey.postalJapanese, enabled ? EngineToggle.on : EngineToggle.off)])
    }

    func setPostalEnglish(_ enabled: Bool) async {
        guard enabled != values.postalEnglish else { return }
        await apply(set: [(SettingKey.postalEnglish, enabled ? EngineToggle.on : EngineToggle.off)])
    }

    func setCandidateHideWhenConfident(_ enabled: Bool) async {
        guard enabled != values.candidateHideWhenConfident else { return }
        await apply(set: [(SettingKey.candidateHideWhenConfident, enabled ? EngineToggle.on : EngineToggle.off)])
    }

    func setUnicodeNotation(_ enabled: Bool) async {
        guard enabled != values.unicodeNotation else { return }
        await apply(set: [(SettingKey.unicodeNotation, enabled ? EngineToggle.on : EngineToggle.off)])
    }

    func setDateWeekdayDates(_ enabled: Bool) async {
        guard enabled != values.dateWeekdayDates else { return }
        await apply(set: [(SettingKey.dateWeekdayDates, enabled ? EngineToggle.on : EngineToggle.off)])
    }

    func setPostalTePrefix(_ enabled: Bool) async {
        guard enabled != values.postalTePrefix else { return }
        await apply(set: [(SettingKey.postalTePrefix, enabled ? EngineToggle.on : EngineToggle.off)])
    }

    // MARK: 絵文字のショートコード（ADR 0027 §8.1）

    func setEmojiSmallLetters(_ enabled: Bool) async {
        guard enabled != values.emojiSmallLetters else { return }
        await apply(set: [(SettingKey.emojiSmallLetters, enabled ? EngineToggle.on : EngineToggle.off)])
    }

    func setEmojiShortcode(_ enabled: Bool) async {
        guard enabled != values.emojiShortcode else { return }
        await apply(set: [(SettingKey.emojiShortcode, enabled ? EngineToggle.on : EngineToggle.off)])
    }

    /// ショートコードを使わないアプリに Bundle ID を足す
    func addEmojiShortcodeExclude(_ rawID: String) async {
        guard let id = validBundleID(rawID), !values.emojiShortcodeExclude.contains(id) else { return }
        await setEmojiShortcodeExclude(values.emojiShortcodeExclude + [id])
    }

    func addEmojiShortcodeExclude(appAt url: URL) async {
        guard let id = bundleID(ofAppAt: url) else { return }
        await addEmojiShortcodeExclude(id)
    }

    func removeEmojiShortcodeExclude(_ id: String) async {
        guard values.emojiShortcodeExclude.contains(id) else { return }
        await setEmojiShortcodeExclude(values.emojiShortcodeExclude.filter { $0 != id })
    }

    /// 既定の一覧に戻す（設定の行を消す）
    func resetEmojiShortcodeExclude() async {
        guard values.emojiShortcodeExclude != Settings.defaultEmojiShortcodeExclude else { return }
        await apply(reset: [SettingKey.emojiShortcodeExclude])
    }

    /// 一覧を書く。空の一覧は空の値（除外なし）として書く
    private func setEmojiShortcodeExclude(_ list: [String]) async {
        let value = list.joined(separator: String(Settings.listSeparator))
        await apply(set: [(SettingKey.emojiShortcodeExclude, value)])
    }

    func removeSharedExclude(_ id: String) async {
        guard values.contextSharedExclude.contains(id) else { return }
        await setSharedExclude(values.contextSharedExclude.filter { $0 != id })
    }

    /// 既定の一覧に戻す（設定の行を消す）
    func resetSharedExclude() async {
        guard values.contextSharedExclude != Settings.defaultContextSharedExclude else { return }
        await apply(reset: [SettingKey.contextSharedExclude])
    }

    /// 一覧を書く。空の一覧は空の値（除外なし）として書く
    private func setSharedExclude(_ list: [String]) async {
        let value = list.joined(separator: String(Settings.listSeparator))
        await apply(set: [(SettingKey.contextSharedExclude, value)])
    }

    /// 操作ごとの割り当てを変える。`.preset` は上書きの行を消す（`reset`）
    func setKeymap(_ action: KeymapAction, to binding: KeymapBinding) async {
        let key = SettingKey.keymap(action)
        switch binding {
        case .preset:
            guard values.keymapBindings[action] != nil else { return }
            await apply(reset: [key])
        case .none:
            await apply(set: [(key, Settings.noneValue)])
        case .chord(let text):
            guard let chord = KeyChord(parsing: text) else { return }
            if let other = conflictingAction(for: chord, excluding: action) {
                errorMessage = "\(KeyChordText.display(for: chord)) は「\(Self.actionTitles[other] ?? other.rawValue)」に割り当てられています"
                return
            }
            await apply(set: [(key, KeyChordText.string(for: chord))])
        }
    }

    private func apply(set: [(key: String, value: String)] = [], reset: [String] = []) async {
        do {
            let snapshot = try await bridge.updateSettings(
                set: set.map { entry in
                    var out = SettingEntry()
                    out.key = entry.key
                    out.value = entry.value
                    return out
                },
                reset: reset)
            values = SettingsValues(snapshot: snapshot)
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

    var keymapRows: [KeymapRow] {
        KeymapAction.allCases.map { action in
            let binding = resolvedBinding(for: action)
            let presetChord = presets.primaryChord(for: action, preset: values.keymapPreset)
            let presetText = presetChord.map(KeyChordText.display(for:))
            let keyText: String
            switch binding {
            case .preset: keyText = presetText ?? Self.noneText
            case .none: keyText = Self.noneText
            case .chord(let text): keyText = KeyChordText.display(forText: text)
            }
            return KeymapRow(
                action: action, title: Self.actionTitles[action] ?? action.rawValue, binding: binding,
                keyText: keyText, presetText: presetText)
        }
    }

    /// 上書きがプリセットの割り当てと同じなら `.preset` とみなす
    /// （殻が、上書きの行だけでなく「今効いているキー」を返す場合にも合わせられる）
    func resolvedBinding(for action: KeymapAction) -> KeymapBinding {
        guard let binding = values.keymapBindings[action] else { return .preset }
        if case .chord(let text) = binding, let chord = KeyChord(parsing: text),
            presets.chords(for: action, preset: values.keymapPreset).contains(chord)
        {
            return .preset
        }
        return binding
    }

    /// 今効いている、その操作のキー（F キーを含む）
    func effectiveChords(for action: KeymapAction) -> [KeyChord] {
        let preset = presets.chords(for: action, preset: values.keymapPreset)
        switch resolvedBinding(for: action) {
        case .preset: return preset
        case .none: return []
        case .chord(let text):
            // 上書きは文字キーを置き換え、F キーは残す（`Keymap.init(presetTSV:settings:)`）
            return [KeyChord(parsing: text)].compactMap { $0 } + preset.filter(KeyChordText.isFunctionKey)
        }
    }

    /// そのキーを、ほかの操作がすでに使っていれば、その操作
    func conflictingAction(for chord: KeyChord, excluding action: KeymapAction) -> KeymapAction? {
        KeymapAction.allCases.first { other in
            other != action && effectiveChords(for: other).contains(chord)
        }
    }

    // MARK: 属性プロファイル

    /// 分野ごとの重み。`ProfileSet` は全体を置き換えるので、全分野を送る（OFF は書かない）
    func setWeight(_ weight: ProfileWeight, forDomain id: String) async {
        guard let index = profile.firstIndex(where: { $0.id == id }), profile[index].weight != weight else { return }
        var next = profile
        next[index].weight = weight
        var request = Ikari_Engine_V1_ProfileSet()
        request.domains = next.filter { $0.weight != .off }.map(Self.state(from:))
        do {
            let body = try await bridge.engine(.setProfile(request))
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
