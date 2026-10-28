// settings.tsv の値の型づけ（ADR 0005 §6.3「設定は文字列のキーと値で運ぶ」）。
// 殻の `IkariShellCore.Settings` と同じキーを使う。`engine.*` は殻の `Settings` が持たないので、ここで読む。
// 画面は「今効いている値」のスナップショット（`SettingsSnapshot`）から作る。
import Foundation
import IkariShellCore

/// docs/phase2-contract.md §3 のキー
enum SettingKey {
    static let inputMethod = "input.method"
    static let keymapPreset = "keymap.preset"
    static let pagingWrap = "paging.wrap"
    static let pagingSize = "paging.size"
    static let engineLM = "engine.lm"
    static let engineIdleUnloadSecs = "engine.idle_unload_secs"
    static let engineLMDeadlineMs = "engine.lm_deadline_ms"
    static let engineConvertWaitMs = "engine.convert_wait_ms"
    static let contextSharedExclude = Settings.contextSharedExcludeKey
    static let emojiShortcode = Settings.emojiShortcodeKey
    static let emojiShortcodeExclude = Settings.emojiShortcodeExcludeKey
    static let emojiSmallLetters = Settings.emojiSmallLettersKey
    static let postalJapanese = Settings.postalJapaneseKey
    static let postalEnglish = Settings.postalEnglishKey
    static let postalTePrefix = Settings.postalTePrefixKey
    static let unicodeNotation = Settings.unicodeNotationKey
    static let dateWeekdayDates = Settings.dateWeekdayDatesKey
    static let candidateHideWhenConfident = Settings.candidateHideWhenConfidentKey
    private static let keymapPrefix = "keymap."

    static func keymap(_ action: KeymapAction) -> String { keymapPrefix + action.rawValue }
}

/// `engine.lm` の値
enum EngineToggle {
    static let on = "on"
    static let off = "off"
}

/// 操作ごとのキー割り当て。`preset` は上書きなし
enum KeymapBinding: Equatable, Sendable {
    case preset
    /// 割り当てを外す（`none`）
    case none
    /// `ctrl+j` の表記
    case chord(String)
}

struct SettingsValues: Equatable, Sendable {
    /// `engine.idle_unload_secs` の既定（docs/phase2-contract.md §3）
    static let defaultIdleUnloadSecs = 300
    /// 画面で選べる秒数の上限（1 日）。`0` は解放しない
    static let idleUnloadRange = 0...86_400
    /// `engine.lm_deadline_ms` の既定と範囲（暫定。エンジンも同じ範囲に丸める）
    static let defaultLMDeadlineMs = Settings.defaultEngineLMDeadlineMs
    static let lmDeadlineRange = 50...2000
    /// `engine.convert_wait_ms` の既定と範囲（暫定。0 = 待たない）
    static let defaultConvertWaitMs = Settings.defaultEngineConvertWaitMs
    static let convertWaitRange = 0...200

    var inputMethod: InputMethodKind = Settings.default.inputMethod
    var keymapPreset: KeymapPreset = Settings.default.keymapPreset
    var keymapBindings: [KeymapAction: KeymapBinding] = [:]
    var pagingWrap: Bool = Settings.default.pagingWrap
    var pageSize: Int = Settings.default.pageSize
    var lmEnabled = true
    var idleUnloadSecs = SettingsValues.defaultIdleUnloadSecs
    var lmDeadlineMs = SettingsValues.defaultLMDeadlineMs
    var convertWaitMs = SettingsValues.defaultConvertWaitMs
    /// `context.shared_exclude`。ほかのアプリと文脈を貸し借りしないアプリの Bundle ID
    var contextSharedExclude = Settings.defaultContextSharedExclude
    /// `emoji.shortcode`。絵文字のショートコード（`:eyes:`）のオン・オフ
    var emojiShortcode = Settings.defaultEmojiShortcode
    /// `emoji.shortcode_exclude`。ショートコードを使わないアプリの Bundle ID
    var emojiShortcodeExclude = Settings.defaultEmojiShortcodeExclude
    /// `emoji.small_letters`。英語のモードで小さい文字（m²、m₂）の候補
    var emojiSmallLetters = Settings.defaultEmojiSmallLetters
    /// `postal.japanese`。住所 ↔ 郵便番号（日本語）
    var postalJapanese = Settings.defaultPostalJapanese
    /// `postal.english`。英語のモードでの郵便番号の候補
    var postalEnglish = Settings.defaultPostalEnglish
    /// `postal.te_prefix`。「て」+ 郵便番号で〒つき
    var postalTePrefix = Settings.defaultPostalTePrefix
    /// `unicode.notation`。1 文字のかなに `U+3042` の形の候補
    var unicodeNotation = Settings.defaultUnicodeNotation
    /// `date.weekday_dates`。曜日の読みに直近の日付の候補
    var dateWeekdayDates = Settings.defaultDateWeekdayDates
    /// `candidate.hide_when_confident`。確信が高い変換では候補一覧を出さない
    var candidateHideWhenConfident = Settings.defaultCandidateHideWhenConfident

    init() {}

    /// 読めない値と未知のキーは既定のままにする（画面を壊さない）
    init(snapshot: [String: String]) {
        if let value = snapshot[SettingKey.inputMethod], let method = InputMethodKind(rawValue: value) {
            inputMethod = method
        }
        if let value = snapshot[SettingKey.keymapPreset], let preset = KeymapPreset(rawValue: value) {
            keymapPreset = preset
        }
        for action in KeymapAction.allCases {
            guard let value = snapshot[SettingKey.keymap(action)], !value.isEmpty else { continue }
            if value == Settings.noneValue {
                keymapBindings[action] = KeymapBinding.none
            } else if KeyChord(parsing: value) != nil {
                keymapBindings[action] = .chord(value)
            }
        }
        switch snapshot[SettingKey.pagingWrap] {
        case "true": pagingWrap = true
        case "false": pagingWrap = false
        default: break
        }
        if let text = snapshot[SettingKey.pagingSize], let size = Int(text), Settings.pageSizeRange.contains(size) {
            pageSize = size
        }
        switch snapshot[SettingKey.engineLM] {
        case EngineToggle.on: lmEnabled = true
        case EngineToggle.off: lmEnabled = false
        default: break
        }
        if let text = snapshot[SettingKey.engineIdleUnloadSecs], let secs = Int(text),
            Self.idleUnloadRange.contains(secs)
        {
            idleUnloadSecs = secs
        }
        if let text = snapshot[SettingKey.engineLMDeadlineMs], let ms = Int(text), Self.lmDeadlineRange.contains(ms) {
            lmDeadlineMs = ms
        }
        if let text = snapshot[SettingKey.engineConvertWaitMs], let ms = Int(text), Self.convertWaitRange.contains(ms) {
            convertWaitMs = ms
        }
        if let text = snapshot[SettingKey.contextSharedExclude], let list = Settings.parseBundleIDList(text) {
            contextSharedExclude = list
        }
        switch snapshot[SettingKey.emojiShortcode] {
        case EngineToggle.on: emojiShortcode = true
        case EngineToggle.off: emojiShortcode = false
        default: break
        }
        if let text = snapshot[SettingKey.emojiShortcodeExclude], let list = Settings.parseBundleIDList(text) {
            emojiShortcodeExclude = list
        }
        func toggle(_ key: String, _ current: Bool) -> Bool {
            switch snapshot[key] {
            case EngineToggle.on: true
            case EngineToggle.off: false
            default: current
            }
        }
        postalJapanese = toggle(SettingKey.postalJapanese, postalJapanese)
        postalEnglish = toggle(SettingKey.postalEnglish, postalEnglish)
        postalTePrefix = toggle(SettingKey.postalTePrefix, postalTePrefix)
        emojiSmallLetters = toggle(SettingKey.emojiSmallLetters, emojiSmallLetters)
        unicodeNotation = toggle(SettingKey.unicodeNotation, unicodeNotation)
        dateWeekdayDates = toggle(SettingKey.dateWeekdayDates, dateWeekdayDates)
        candidateHideWhenConfident = toggle(SettingKey.candidateHideWhenConfident, candidateHideWhenConfident)
    }
}

/// プリセットの TSV から、操作ごとの割り当てを引く。上書きが「プリセットと同じ」かの判定に使う。
struct KeymapPresetTable: Sendable {
    private var chords: [KeymapPreset: [KeymapAction: [KeyChord]]] = [:]

    init() {}

    /// `read` は `macos` / `ms-ime` / `atok` の TSV を返す。読めないプリセットは空のまま（画面に既定のキーを出さないだけ）
    init(read: (KeymapPreset) -> String?) {
        for preset in KeymapPreset.allCases {
            guard let text = read(preset), let keymap = try? Keymap(presetTSV: text, overrides: [:]) else { continue }
            var byAction: [KeymapAction: [KeyChord]] = [:]
            for (chord, action) in keymap.bindings { byAction[action, default: []].append(chord) }
            chords[preset] = byAction.mapValues { $0.sorted(by: Self.order) }
        }
    }

    /// バンドルの `keymap/<プリセット>.tsv`（build.sh が設定アプリの Resources に置く）
    static func bundled(_ bundle: Bundle = .main) -> KeymapPresetTable {
        KeymapPresetTable { preset in
            bundle.url(forResource: preset.rawValue, withExtension: "tsv", subdirectory: "keymap")
                .flatMap { try? String(contentsOf: $0, encoding: .utf8) }
        }
    }

    func chords(for action: KeymapAction, preset: KeymapPreset) -> [KeyChord] {
        chords[preset]?[action] ?? []
    }

    /// 文字キーの既定の割り当て（`ctrl+j`）。F キーは常に使えるので除く
    func primaryChord(for action: KeymapAction, preset: KeymapPreset) -> KeyChord? {
        chords(for: action, preset: preset).first { !KeyChordText.isFunctionKey($0) }
    }

    /// F キーが先、次に修飾キーの少ない順、最後にキー名の順（表示を安定させる）
    private static func order(_ lhs: KeyChord, _ rhs: KeyChord) -> Bool {
        (lhs.modifiers.rawValue, lhs.key) < (rhs.modifiers.rawValue, rhs.key)
    }
}
