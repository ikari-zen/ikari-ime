// settings.tsv（ADR 0004 §6.3、docs/phase2-contract.md §3）。契約。
// 殻と設定アプリの設定の処理は shell-core（`settings`。ADR 0041 §2.3）に移った。ここに残るのは、M5 まで残る
// Swift の状態機械（`InputStateMachine`、`Keymap`）とその測り手・テストが使う型と読み込みだけ（M5 で消す）。
import Foundation

public enum InputMethodKind: String, Sendable {
    case romaji
    case kana
}

public struct Settings: Sendable, Equatable {
    /// `input.method`
    public var inputMethod: InputMethodKind
    /// `keymap.preset`
    public var keymapPreset: KeymapPreset
    /// `keymap.<操作>`。値はキーの表記、または `none`（`Settings.noneValue`）
    public var keymapOverrides: [KeymapAction: String]
    /// `paging.wrap`。最後のページで Space を押したら先頭に戻る（ADR 0004 §15.1-3）
    public var pagingWrap: Bool
    /// `paging.size`（1..9）。最初に見せる候補の数。3件目からさらに進むと、ページ全体（最大9件）に広がる
    public var pageSize: Int
    /// `engine.lm`。エンジンが読む（殻は読まないが、メニューの表示と、変わったときのエンジンの再起動の判定に使う。ADR 0005 §7.3）
    public var engineLM: Bool
    /// `engine.idle_unload_secs`。LM を解放するまでの秒数（0 = 解放しない）
    public var engineIdleUnloadSecs: Int
    /// `engine.lm_deadline_ms`。先行計算で LM を待てる締め切り（ミリ秒。エンジンが 50〜2000 に丸める。暫定）
    public var engineLMDeadlineMs: Int
    /// `engine.convert_wait_ms`。Space で先行計算の完了を待つ上限（ミリ秒。0 = 待たない。エンジンが 0〜200 に丸める。暫定）
    public var engineConvertWaitMs: Int
    /// `context.shared_exclude`。ほかのアプリと文脈を貸し借りしないアプリの Bundle ID（ADR 0015 §3.5）。
    /// 順は保つ。空にすると除外なし
    public var contextSharedExclude: [String]
    /// `emoji.shortcode`。絵文字のショートコード（`:eyes:`）のオン・オフ（全モード。ADR 0027 §8.1）
    public var emojiShortcode: Bool
    /// `emoji.shortcode_exclude`。ショートコードを使わないアプリの Bundle ID（`context.shared_exclude` と同じ書式。ADR 0027 §8.1）
    public var emojiShortcodeExclude: [String]
    /// `emoji.small_letters`。英語のモードで `:m2:` → m²、`:m_2:` → m₂ などの小さい文字の候補を出す（`emoji.shortcode` がオンのときだけ。既定はオフ。#264）
    public var emojiSmallLetters: Bool
    /// `postal.japanese`。住所 ↔ 郵便番号の変換（日本語。エンジンが読む。ADR 0032）
    public var postalJapanese: Bool
    /// `postal.english`。英語のモードで郵便番号の候補（語頭の `te` + 数字 7 桁。殻が使う。ADR 0032 §9.1。`EnglishPostal.swift`）
    public var postalEnglish: Bool
    /// `postal.te_prefix`。「て」+ 郵便番号で〒つきの候補（日本語・英語の両方。エンジンが読む。ADR 0032 §8）
    public var postalTePrefix: Bool
    /// `unicode.notation`。1 文字のかなに `U+3042` の形の候補を出す（日本語。エンジンが読む。既定はオフ）
    public var unicodeNotation: Bool
    /// `date.weekday_dates`。曜日の読みに、直近の日付の候補を出す（日本語。エンジンが読む。既定はオフ）
    public var dateWeekdayDates: Bool
    /// `candidate.hide_when_confident`。エンジンが確信が高いと返した変換では、最初の Space で 1 位だけを下線つきで出し、一覧は 2 回目の Space か ↓ で出す（殻が使う。既定はオン（2026-10-28 ユーザー決定）。#69）
    public var candidateHideWhenConfident: Bool

    public init(
        inputMethod: InputMethodKind = .romaji,
        keymapPreset: KeymapPreset = .macos,
        keymapOverrides: [KeymapAction: String] = [:],
        pagingWrap: Bool = true,
        pageSize: Int = defaultPageSize,
        engineLM: Bool = true,
        engineIdleUnloadSecs: Int = 300,
        engineLMDeadlineMs: Int = Settings.defaultEngineLMDeadlineMs,
        engineConvertWaitMs: Int = Settings.defaultEngineConvertWaitMs,
        contextSharedExclude: [String] = Settings.defaultContextSharedExclude,
        emojiShortcode: Bool = Settings.defaultEmojiShortcode,
        emojiShortcodeExclude: [String] = Settings.defaultEmojiShortcodeExclude,
        emojiSmallLetters: Bool = Settings.defaultEmojiSmallLetters,
        postalJapanese: Bool = Settings.defaultPostalJapanese,
        postalEnglish: Bool = Settings.defaultPostalEnglish,
        postalTePrefix: Bool = Settings.defaultPostalTePrefix,
        unicodeNotation: Bool = Settings.defaultUnicodeNotation,
        dateWeekdayDates: Bool = Settings.defaultDateWeekdayDates,
        candidateHideWhenConfident: Bool = Settings.defaultCandidateHideWhenConfident
    ) {
        self.inputMethod = inputMethod
        self.keymapPreset = keymapPreset
        self.keymapOverrides = keymapOverrides
        self.pagingWrap = pagingWrap
        self.pageSize = pageSize
        self.engineLM = engineLM
        self.engineIdleUnloadSecs = engineIdleUnloadSecs
        self.engineLMDeadlineMs = engineLMDeadlineMs
        self.engineConvertWaitMs = engineConvertWaitMs
        self.contextSharedExclude = contextSharedExclude
        self.emojiShortcode = emojiShortcode
        self.emojiShortcodeExclude = emojiShortcodeExclude
        self.emojiSmallLetters = emojiSmallLetters
        self.postalJapanese = postalJapanese
        self.postalEnglish = postalEnglish
        self.postalTePrefix = postalTePrefix
        self.unicodeNotation = unicodeNotation
        self.dateWeekdayDates = dateWeekdayDates
        self.candidateHideWhenConfident = candidateHideWhenConfident
    }

    /// 既定値（ファイルがない、または読めないときに使う）
    public static let `default` = Settings()

    /// `keymap.<操作>` で割り当てを外すときの値
    public static let noneValue = "none"
    public static let pageSizeRange = 1...9
    /// `paging.size` の既定。最初に見せる候補の数（ADR 0014 §9。これを超えて進むと `enginePageSize` 件に広がる）
    public static let defaultPageSize = 3
    /// エンジンに頼む 1 ページの件数（いつも上限。`paging.size` は殻が最初に見せる件数。ADR 0014 §2.3 (c)）
    public static let enginePageSize = pageSizeRange.upperBound
    /// `context.shared_exclude` の既定（ADR 0015 §3.5 Q6）。パスワード管理: 1Password 8、Bitwarden デスクトップ、キーチェーンアクセス。
    /// ターミナル: Terminal、iTerm2、WezTerm、Ghostty
    public static let defaultContextSharedExclude = [
        "com.1password.1password",
        "com.bitwarden.desktop",
        "com.apple.keychainaccess",
        "com.apple.Terminal",
        "com.googlecode.iterm2",
        "com.github.wez.wezterm",
        "com.mitchellh.ghostty",
    ]
    public static let contextSharedExcludeKey = "context.shared_exclude"
    /// 絵文字のショートコード（ADR 0027 §8.1）。値は `on` / `off`（`engine.lm` と同じ書き方）
    public static let emojiShortcodeKey = "emoji.shortcode"
    public static let emojiShortcodeExcludeKey = "emoji.shortcode_exclude"
    public static let emojiSmallLettersKey = "emoji.small_letters"
    public static let defaultEmojiSmallLetters = false
    /// `emoji.shortcode` の既定（ADR 0027 §8.1 Q12。2026-10-20 に既定をオフへ変更）
    public static let defaultEmojiShortcode = false
    /// `emoji.shortcode_exclude` の既定（ADR 0027 §8.1 Q17）。ターミナル: Terminal、iTerm2、WezTerm、Ghostty
    public static let defaultEmojiShortcodeExclude = [
        "com.apple.Terminal",
        "com.googlecode.iterm2",
        "com.github.wez.wezterm",
        "com.mitchellh.ghostty",
    ]
    /// 郵便番号（ADR 0032）。値は `on` / `off`
    public static let postalJapaneseKey = "postal.japanese"
    public static let postalEnglishKey = "postal.english"
    public static let postalTePrefixKey = "postal.te_prefix"
    public static let defaultPostalJapanese = true
    public static let defaultPostalEnglish = false
    public static let defaultPostalTePrefix = false
    /// Unicode 表記（1 文字のかな → `U+3042`）。値は `on` / `off`
    public static let unicodeNotationKey = "unicode.notation"
    public static let defaultUnicodeNotation = false
    /// 曜日の読みに直近の日付の候補（`げつようび` → 直近の月曜の日付）。値は `on` / `off`
    public static let dateWeekdayDatesKey = "date.weekday_dates"
    public static let defaultDateWeekdayDates = false
    /// 確信が高い変換で候補一覧を隠す（#69）。値は `on` / `off`
    public static let candidateHideWhenConfidentKey = "candidate.hide_when_confident"
    public static let defaultCandidateHideWhenConfident = true
    /// `context.shared_exclude` の値の区切り
    public static let listSeparator: Character = ","
    private static let keymapKeyPrefix = "keymap."
    private static let engineKeyPrefix = "engine."
    private static let engineLMKey = "engine.lm"
    private static let engineIdleUnloadKey = "engine.idle_unload_secs"
    private static let engineLMDeadlineKey = "engine.lm_deadline_ms"
    private static let engineConvertWaitKey = "engine.convert_wait_ms"
    /// `engine.lm_deadline_ms` の既定（エンジンの `DEFAULT_LM_DEADLINE_MS` と同じ）
    public static let defaultEngineLMDeadlineMs = 300
    /// `engine.convert_wait_ms` の既定（エンジンの `DEFAULT_CONVERT_WAIT_MS` と同じ）
    public static let defaultEngineConvertWaitMs = 30
    private static let engineLMOn = "on"
    private static let engineLMOff = "off"

    /// TSV を読む。未知のキーと不正な値は無視して既定値を使い、`warnings` に積む（読み込みで入力を止めない）。
    /// 警告は行番号と設定のキー名だけで、値の文字列は含めない（ログにユーザーの文字列を残さない）。
    /// `engine.lm`、`engine.idle_unload_secs`、`engine.lm_deadline_ms`、`engine.convert_wait_ms` は殻も読む（メニューとエンジンの再起動の判定のため）。値が不正なら警告。
    /// それ以外の `engine.*` は無視する（警告も出さない）。
    public static func parse(tsv: String) -> (settings: Settings, warnings: [String]) {
        var settings = Settings.default
        var warnings: [String] = []

        for (offset, rawLine) in tsv.split(omittingEmptySubsequences: false, whereSeparator: \.isNewline).enumerated() {
            let number = offset + 1
            let line = rawLine.trimmingCharacters(in: CharacterSet(charactersIn: "\r"))
            if line.trimmingCharacters(in: .whitespaces).isEmpty || line.hasPrefix("#") { continue }

            let columns = line.split(separator: "\t", maxSplits: 1, omittingEmptySubsequences: false)
            guard columns.count == 2 else {
                warnings.append("line \(number): expected key<TAB>value")
                continue
            }
            let key = columns[0].trimmingCharacters(in: .whitespaces)
            let value = columns[1].trimmingCharacters(in: .whitespaces)
            let invalid = "line \(number): invalid value for \(key)"

            if key.hasPrefix(engineKeyPrefix) {
                switch key {
                case engineLMKey:
                    switch value {
                    case engineLMOn: settings.engineLM = true
                    case engineLMOff: settings.engineLM = false
                    default: warnings.append(invalid)
                    }
                case engineIdleUnloadKey:
                    if let seconds = Int(value), seconds >= 0 {
                        settings.engineIdleUnloadSecs = seconds
                    } else {
                        warnings.append(invalid)
                    }
                case engineLMDeadlineKey:
                    if let ms = Int(value), ms >= 0 {
                        settings.engineLMDeadlineMs = ms
                    } else {
                        warnings.append(invalid)
                    }
                case engineConvertWaitKey:
                    if let ms = Int(value), ms >= 0 {
                        settings.engineConvertWaitMs = ms
                    } else {
                        warnings.append(invalid)
                    }
                default:
                    break
                }
                continue
            }
            if key.hasPrefix(keymapKeyPrefix) && key != "keymap.preset" {
                guard let action = KeymapAction(rawValue: String(key.dropFirst(keymapKeyPrefix.count))) else {
                    warnings.append("line \(number): unknown key")
                    continue
                }
                if value == noneValue {
                    settings.keymapOverrides[action] = noneValue
                } else if KeyChord(parsing: value) != nil {
                    settings.keymapOverrides[action] = value
                } else {
                    warnings.append(invalid)
                }
                continue
            }

            switch key {
            case contextSharedExcludeKey:
                if let list = parseBundleIDList(value) {
                    settings.contextSharedExclude = list
                } else {
                    warnings.append(invalid)
                }
            case emojiShortcodeKey:
                switch value {
                case engineLMOn: settings.emojiShortcode = true
                case engineLMOff: settings.emojiShortcode = false
                default: warnings.append(invalid)
                }
            case emojiShortcodeExcludeKey:
                if let list = parseBundleIDList(value) {
                    settings.emojiShortcodeExclude = list
                } else {
                    warnings.append(invalid)
                }
            case postalJapaneseKey, postalEnglishKey, postalTePrefixKey:
                guard let on = parseSwitch(value) else {
                    warnings.append(invalid)
                    break
                }
                switch key {
                case postalJapaneseKey: settings.postalJapanese = on
                case postalEnglishKey: settings.postalEnglish = on
                default: settings.postalTePrefix = on
                }
            case emojiSmallLettersKey:
                if let on = parseSwitch(value) {
                    settings.emojiSmallLetters = on
                } else {
                    warnings.append(invalid)
                }
            case unicodeNotationKey:
                if let on = parseSwitch(value) {
                    settings.unicodeNotation = on
                } else {
                    warnings.append(invalid)
                }
            case dateWeekdayDatesKey:
                if let on = parseSwitch(value) {
                    settings.dateWeekdayDates = on
                } else {
                    warnings.append(invalid)
                }
            case candidateHideWhenConfidentKey:
                if let on = parseSwitch(value) {
                    settings.candidateHideWhenConfident = on
                } else {
                    warnings.append(invalid)
                }
            case "input.method":
                if let method = InputMethodKind(rawValue: value) {
                    settings.inputMethod = method
                } else {
                    warnings.append(invalid)
                }
            case "keymap.preset":
                if let preset = KeymapPreset(rawValue: value) {
                    settings.keymapPreset = preset
                } else {
                    warnings.append(invalid)
                }
            case "paging.wrap":
                switch value {
                case "true": settings.pagingWrap = true
                case "false": settings.pagingWrap = false
                default: warnings.append(invalid)
                }
            case "paging.size":
                if let size = Int(value), pageSizeRange.contains(size) {
                    settings.pageSize = size
                } else {
                    warnings.append(invalid)
                }
            default:
                warnings.append("line \(number): unknown key")
            }
        }
        return (settings, warnings)
    }

    /// Bundle ID の一覧（`,` 区切り）。空の値は空の一覧。空白は削り、空の要素は捨てて、重複は最初だけ残す。
    /// 使えない文字（英数字、`.`、`-`、`_` 以外）があれば nil
    private static func parseBundleIDList(_ value: String) -> [String]? {
        var result: [String] = []
        for part in value.split(separator: listSeparator, omittingEmptySubsequences: true) {
            let id = part.trimmingCharacters(in: .whitespaces)
            if id.isEmpty { continue }
            guard id.unicodeScalars.allSatisfy(bundleIDScalars.contains) else { return nil }
            if !result.contains(id) { result.append(id) }
        }
        return result
    }

    private static let bundleIDScalars: CharacterSet = {
        var set = CharacterSet.alphanumerics
        set.formUnion(CharacterSet(charactersIn: ".-_"))
        return set
    }()

    private static func parseSwitch(_ value: String) -> Bool? {
        switch value {
        case engineLMOn: true
        case engineLMOff: false
        default: nil
        }
    }
}
