// settings.tsv の値の型づけ（ADR 0005 §6.3「設定は文字列のキーと値で運ぶ」）。
// 読み方、既定値、範囲は shell-core（`settings::SettingsValues`。ADR 0041 §2.3）が決め、ここは画面のために写した値を持つだけ。
// 画面は「今効いている値」のスナップショット（`SettingsSnapshot`）から作る。
import Foundation
import IkariProto

/// 操作ごとのキー割り当て。`preset` は上書きなし
enum KeymapBinding: Equatable, Sendable {
    case preset
    /// 割り当てを外す（`none`）
    case none
    /// `ctrl+j` の表記
    case chord(String)
}

struct SettingsValues: Equatable, Sendable {
    /// `engine.idle_unload_secs` の画面で選べる範囲（`0` は解放しない）
    static let idleUnloadRange = SettingsCore.range(SettingsCore.constants.idleUnloadSecs)
    /// `engine.lm_deadline_ms` の範囲（エンジンも同じ範囲に丸める）
    static let lmDeadlineRange = SettingsCore.range(SettingsCore.constants.lmDeadlineMs)
    /// `engine.convert_wait_ms` の範囲（0 = 待たない）
    static let convertWaitRange = SettingsCore.range(SettingsCore.constants.convertWaitMs)
    /// `clause.min_chars` の範囲（1 = 下限なし）
    static let clauseMinCharsRange = SettingsCore.range(SettingsCore.constants.clauseMinChars)
    /// `paging.size` の範囲
    static let pageSizeRange = SettingsCore.range(SettingsCore.constants.pageSize)
    /// 何も書いていないときの値
    static let defaults = SettingsValues(message: SettingsCore.constants.defaults)

    // 初期値は FFI のないビルドで画面を出すためだけのもの。実際の既定値は shell-core が返す（`defaults`）
    var inputMethod: InputMethodKind = .romaji
    var keymapPreset: KeymapPreset = .macos
    var keymapBindings: [KeymapAction: KeymapBinding] = [:]
    var pagingWrap = false
    var pageSize = 0
    var lmEnabled = false
    var idleUnloadSecs = 0
    var lmDeadlineMs = 0
    var convertWaitMs = 0
    /// `clause.min_chars`。文節ごとの削除で、消す文字がこの数に満たなければ前の文節まで広げる
    var clauseMinChars = 0
    /// `emoji.shortcode`。絵文字のショートコード（`:eyes:`）のオン・オフ
    var emojiShortcode = false
    /// `emoji.shortcode_exclude`。ショートコードを使わないアプリの Bundle ID
    var emojiShortcodeExclude: [String] = []
    /// `emoji.small_letters`。英語のモードで小さい文字（m²、m₂）の候補
    var emojiSmallLetters = false
    /// `postal.japanese`。住所 ↔ 郵便番号（日本語）
    var postalJapanese = false
    /// `postal.english`。英語のモードでの郵便番号の候補
    var postalEnglish = false
    /// `postal.te_prefix`。「て」+ 郵便番号で〒つき
    var postalTePrefix = false
    /// `unicode.notation`。1 文字のかなに `U+3042` の形の候補
    var unicodeNotation = false
    /// `date.weekday_dates`。曜日の読みに直近の日付の候補
    var dateWeekdayDates = false
    /// `candidate.hide_when_confident`。確信が高い変換では候補一覧を出さない
    var candidateHideWhenConfident = false

    /// 何も書いていないときの値（`defaults` と同じ）
    init() {
        self = Self.defaults
    }

    /// 殻の返す設定の行から作る。読めない値と未知のキーは既定のまま（画面を壊さない）
    init(snapshot: [String: String]) {
        guard let described = SettingsCore.describe(snapshot, presets: KeymapPresetTable()) else {
            self = Self.defaults
            return
        }
        self.init(message: described.values)
    }

    /// shell-core の返した値の一式から作る
    init(message: Ikari_Shell_V1_SettingsSnapshot) {
        if let method = SettingsCore.method(message.inputMethod) { inputMethod = method }
        if let preset = SettingsCore.preset(message.keymapPreset) { keymapPreset = preset }
        for (name, value) in message.keymapOverrides {
            guard let action = KeymapAction(rawValue: name) else { continue }
            keymapBindings[action] = value == Self.noneValue ? KeymapBinding.none : .chord(value)
        }
        pagingWrap = message.pagingWrap
        pageSize = Int(message.pageSize)
        lmEnabled = message.engineLm
        idleUnloadSecs = Int(message.engineIdleUnloadSecs)
        lmDeadlineMs = Int(message.engineLmDeadlineMs)
        convertWaitMs = Int(message.engineConvertWaitMs)
        clauseMinChars = Int(message.clauseMinChars)
        emojiShortcode = message.emojiShortcode
        emojiShortcodeExclude = message.emojiShortcodeExclude
        emojiSmallLetters = message.emojiSmallLetters
        postalJapanese = message.postalJapanese
        postalEnglish = message.postalEnglish
        postalTePrefix = message.postalTePrefix
        unicodeNotation = message.unicodeNotation
        dateWeekdayDates = message.dateWeekdayDates
        candidateHideWhenConfident = message.candidateHideWhenConfident
    }

    /// `keymap_overrides` の「割り当てを外す」の値（shell-core の `VALUE_NONE`。proto の約束）
    private static let noneValue = "none"
}

/// キー割り当てのプリセットの TSV。上書きが「プリセットと同じ」かの判定と、プリセットのキーの表示は shell-core が行う。
struct KeymapPresetTable: Sendable {
    private(set) var texts: [Ikari_Settings_V1_PresetText] = []

    init() {}

    /// `read` は `macos` / `ms-ime` / `atok` の TSV を返す。読めないプリセットは空のまま（画面に既定のキーを出さないだけ）
    init(read: (KeymapPreset) -> String?) {
        for preset in KeymapPreset.allCases {
            guard let text = read(preset) else { continue }
            var entry = Ikari_Settings_V1_PresetText()
            entry.preset = SettingsCore.kind(preset)
            entry.tsv = text
            texts.append(entry)
        }
    }

    /// バンドルの `keymap/<プリセット>.tsv`（build.sh が設定アプリの Resources に置く）
    static func bundled(_ bundle: Bundle = .main) -> KeymapPresetTable {
        KeymapPresetTable { preset in
            bundle.url(forResource: preset.rawValue, withExtension: "tsv", subdirectory: "keymap")
                .flatMap { try? String(contentsOf: $0, encoding: .utf8) }
        }
    }
}
