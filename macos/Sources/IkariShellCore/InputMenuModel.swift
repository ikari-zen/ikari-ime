// 入力メニューの中身の決定（ADR 0005 §5、§15.2-3/4）。契約。
// AppKit にも IMK にも依存しない。`IkariIME` はこの結果を `NSMenuItem` に写すだけにする。
//
// 契約:
// - `build(state:)` は純関数。`InputMenuState` が同じなら、毎回同じ `[MenuItemSpec]` を返す。
//   `menu()` はほぼ打鍵ごとに呼ばれる（S0-1: 30 分で 2048 回）ので、キャッシュは IkariIME が
//   `InputMenuState` を鍵にして持つ（`InputMenuState` が変わったときだけ `build` を呼び直す）。
// - 項目は平らに並べる（サブメニューを使わない。§5.2）。
// - **排他の選択（ラジオ）は `MenuItemSpec.Kind.radio(group)` で表す。同じ `MenuRadioGroup` の項目は、ちょうど 1 つだけ
//   `isChecked == true` になる**（一方を選ぶと、もう一方のチェックが外れる。§15.2-3）。
//   `NSMenuItem` にラジオの仕組みはないので、チェックは毎回 `state` から作り直す。
import Foundation

/// 入力メニューの項目を選んだときの動作。`IkariIME` は `representedObject` に入れて、`ikariMenuCommand:` で受ける。
public enum MenuCommand: Sendable, Hashable {
    case selectKeymapPreset(KeymapPreset)
    case selectInputMethod(InputMethodKind)
    /// LM（zenz）のオン・オフ。`engine.lm` を書き、変わればエンジンを起こし直す（ADR 0005 §7.3）
    case setLM(enabled: Bool)
    /// 分野を含める（`often`）／外す。`sometimes` にするのは設定アプリから（§5.2）
    case setProfileDomain(id: String, included: Bool)
    /// 設定アプリを開く。`tab == nil` なら前回のタブ（既定は先頭）
    case openSettings(tab: SettingsTab?)
}

/// 設定アプリのタブ。メニューが開くタブを指し、ブリッジの `OpenTab.tab` には `rawValue` を入れる。
public enum SettingsTab: String, Sendable, CaseIterable {
    case input
    case conversion
    case userDictionary = "dictionary"
    case learning
    case about
}

/// 排他の選択の組。同じ組の項目は、ちょうど 1 つだけチェックが付く。
public enum MenuRadioGroup: Sendable, Hashable, CaseIterable {
    /// キー割り当て（macOS 風／MS-IME 風／ATOK 風）
    case keymapPreset
    /// 入力方式（ローマ字／かな）
    case inputMethod
}

public struct MenuItemSpec: Sendable, Equatable {
    public enum Kind: Sendable, Equatable {
        case separator
        /// チェックのない項目（「環境設定…」など）
        case action
        /// 独立して入り切りする項目（LM、分野）。`isChecked` が今の状態
        case toggle
        /// 排他の選択の 1 つ。同じ `group` の項目のうち、ちょうど 1 つが `isChecked`
        case radio(MenuRadioGroup)
    }

    public var kind: Kind
    public var title: String
    /// 選んだときの動作。`separator` では nil、それ以外では必ずある
    public var command: MenuCommand?
    public var isChecked: Bool
    public var isEnabled: Bool

    public init(kind: Kind, title: String, command: MenuCommand?, isChecked: Bool = false, isEnabled: Bool = true) {
        self.kind = kind
        self.title = title
        self.command = command
        self.isChecked = isChecked
        self.isEnabled = isEnabled
    }

    public static let separator = MenuItemSpec(kind: .separator, title: "", command: nil, isEnabled: false)
}

/// 分野（属性プロファイル）の重み。エンジンの `ProfileWeightKind` に対応する（IkariShellCore は protobuf を知らない）。
public enum ProfileWeight: Sendable, Equatable {
    case off, sometimes, often
}

public struct ProfileDomain: Sendable, Equatable {
    public var id: String
    public var name: String
    public var weight: ProfileWeight

    public init(id: String, name: String, weight: ProfileWeight) {
        self.id = id
        self.name = name
        self.weight = weight
    }
}

/// メニューを決める入力のすべて。`Equatable` なので、IkariIME が前回の値と比べて `build` の結果を使い回せる。
public struct InputMenuState: Sendable, Equatable {
    /// 殻の設定（`input.method`、`keymap.preset`）
    public var settings: Settings
    /// `engine.lm`（殻の `Settings` は `engine.*` を持たない。読み方はレーン S3 が決める）
    public var lmEnabled: Bool
    /// エンジンの LM の状態。まだ分からないとき nil。読み込み中や使えないときは項目名に括弧で添える（§5.2）
    public var lmState: EngineLmState?
    /// キャッシュしている分野。まだ取れていないとき nil（分野の項目を出さない）。`menu()` の中でエンジンは呼ばない
    public var profile: [ProfileDomain]?

    public init(settings: Settings, lmEnabled: Bool, lmState: EngineLmState? = nil, profile: [ProfileDomain]? = nil) {
        self.settings = settings
        self.lmEnabled = lmEnabled
        self.lmState = lmState
        self.profile = profile
    }
}

public enum InputMenuModel {
    /// 項目名（ADR 0005 §5.2）
    enum Title {
        static let keymapMacOS = "キー割り当て: macOS 風"
        static let keymapMSIME = "キー割り当て: MS-IME 風"
        static let keymapATOK = "キー割り当て: ATOK 風"
        static let inputRomaji = "入力方式: ローマ字"
        static let inputKana = "入力方式: かな"
        static let lm = "LM（zenz）で並べ替える"
        static let lmLoading = "（読み込み中）"
        static let lmUnavailable = "（使えません）"
        static let domainPrefix = "分野: "
        static let domainSometimes = "（たまに）"
        static let userDictionary = "ユーザー辞書と学習…"
        static let preferences = "環境設定…"
    }

    /// 入力メニューの項目（ADR 0005 §5.2 の順）。純関数: `state` が同じなら結果も同じ。
    public static func build(state: InputMenuState) -> [MenuItemSpec] {
        var items: [MenuItemSpec] = []

        let preset = state.settings.keymapPreset
        items.append(radio(.keymapPreset, Title.keymapMacOS, .selectKeymapPreset(.macos), checked: preset == .macos))
        items.append(radio(.keymapPreset, Title.keymapMSIME, .selectKeymapPreset(.msIme), checked: preset == .msIme))
        items.append(radio(.keymapPreset, Title.keymapATOK, .selectKeymapPreset(.atok), checked: preset == .atok))
        items.append(.separator)

        let method = state.settings.inputMethod
        items.append(radio(.inputMethod, Title.inputRomaji, .selectInputMethod(.romaji), checked: method == .romaji))
        items.append(radio(.inputMethod, Title.inputKana, .selectInputMethod(.kana), checked: method == .kana))
        items.append(.separator)

        items.append(
            MenuItemSpec(
                kind: .toggle, title: Title.lm + lmSuffix(state.lmState, enabled: state.lmEnabled),
                command: .setLM(enabled: !state.lmEnabled), isChecked: state.lmEnabled))
        items.append(.separator)

        if let domains = state.profile, !domains.isEmpty {
            for domain in domains {
                let included = domain.weight != .off
                let suffix = domain.weight == .sometimes ? Title.domainSometimes : ""
                items.append(
                    MenuItemSpec(
                        kind: .toggle, title: Title.domainPrefix + domain.name + suffix,
                        command: .setProfileDomain(id: domain.id, included: !included), isChecked: included))
            }
            items.append(.separator)
        }

        items.append(MenuItemSpec(kind: .action, title: Title.userDictionary, command: .openSettings(tab: .userDictionary)))
        items.append(MenuItemSpec(kind: .action, title: Title.preferences, command: .openSettings(tab: nil)))
        return items
    }

    private static func radio(_ group: MenuRadioGroup, _ title: String, _ command: MenuCommand, checked: Bool) -> MenuItemSpec {
        MenuItemSpec(kind: .radio(group), title: title, command: command, isChecked: checked)
    }

    /// 読み込み中と使えないときだけ括弧で添える。切っているとき（古い状態が残りうる）と、使えるときは何も添えない
    private static func lmSuffix(_ state: EngineLmState?, enabled: Bool) -> String {
        guard enabled else { return "" }
        return switch state {
        case .loading: Title.lmLoading
        case .failed: Title.lmUnavailable
        case .unspecified, .disabled, .ready, nil: ""
        }
    }
}
