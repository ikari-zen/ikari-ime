// 設定の検証・整形・既定値・変更の組み立てを shell-core（Rust）に頼む薄い層（ADR 0041 §2.3）。
// `ikari_settings_app`（`IkariShellABI.ShellSettings`）を呼び、proto の型と画面の型（IkariShellCore の列挙）を写すだけ。
// 判断はここに書かない（入口に分岐が増えたら Rust に移す。CLAUDE.md「判断の置き場所」）。
import Foundation
import IkariProto
import IkariShellABI
import IkariShellCore

typealias SettingsAppBody = Ikari_Settings_V1_AppReply.OneOf_Body

enum SettingsCore {
    /// 1 つの要求。FFI のないビルドと、版のずれ・panic のときは nil
    static func call(_ body: Ikari_Settings_V1_AppRequest.OneOf_Body) -> SettingsAppBody? {
        ShellSettings.app(body)
    }

    /// 既定値と範囲（変わらないので 1 回だけ取る）
    static let constants: Ikari_Settings_V1_Constants = {
        guard case .constants(let constants)? = call(.constants(Ikari_Shell_V1_Empty())) else {
            return Ikari_Settings_V1_Constants()
        }
        return constants
    }()

    static func range(_ range: Ikari_Settings_V1_IntRange) -> ClosedRange<Int> {
        let lower = Int(range.min)
        return lower...max(lower, Int(range.max))
    }

    // MARK: 列挙の写し（proto ↔ IkariShellCore）

    static func method(_ kind: Ikari_Shell_V1_InputMethodKind) -> InputMethodKind? {
        switch kind {
        case .romaji: .romaji
        case .kana: .kana
        case .unspecified, .UNRECOGNIZED: nil
        }
    }

    static func kind(_ method: InputMethodKind) -> Ikari_Shell_V1_InputMethodKind {
        switch method {
        case .romaji: .romaji
        case .kana: .kana
        }
    }

    static func preset(_ kind: Ikari_Shell_V1_KeymapPreset) -> KeymapPreset? {
        switch kind {
        case .macos: .macos
        case .msIme: .msIme
        case .atok: .atok
        case .unspecified, .UNRECOGNIZED: nil
        }
    }

    static func kind(_ preset: KeymapPreset) -> Ikari_Shell_V1_KeymapPreset {
        switch preset {
        case .macos: .macos
        case .msIme: .msIme
        case .atok: .atok
        }
    }

    static func binding(_ binding: Ikari_Settings_V1_Binding) -> KeymapBinding {
        guard let kind = binding.kind else { return .preset }
        switch kind {
        case .none: return KeymapBinding.none
        case .chord(let text): return .chord(text)
        case .preset: return .preset
        }
    }

    static func message(_ binding: KeymapBinding) -> Ikari_Settings_V1_Binding {
        var out = Ikari_Settings_V1_Binding()
        switch binding {
        case .preset: out.kind = .preset(Ikari_Shell_V1_Empty())
        case .none: out.kind = .none(Ikari_Shell_V1_Empty())
        case .chord(let text): out.kind = .chord(text)
        }
        return out
    }

    static func modifiers(_ modifiers: KeyModifiers) -> Ikari_Shell_V1_Modifiers {
        var out = Ikari_Shell_V1_Modifiers()
        out.shift = modifiers.contains(.shift)
        out.control = modifiers.contains(.control)
        out.option = modifiers.contains(.option)
        out.command = modifiers.contains(.command)
        return out
    }

    // MARK: 要求

    static func describe(_ entries: [String: String], presets: KeymapPresetTable) -> Ikari_Settings_V1_Described? {
        var request = Ikari_Settings_V1_Describe()
        request.entries = entries
        request.presets = presets.texts
        guard case .described(let described)? = call(.describe(request)) else { return nil }
        return described
    }

    static func plan(
        _ intent: Ikari_Settings_V1_Intent.OneOf_Kind, entries: [String: String], presets: KeymapPresetTable
    ) -> Ikari_Settings_V1_Planned.OneOf_Result? {
        var request = Ikari_Settings_V1_Plan()
        request.entries = entries
        request.presets = presets.texts
        request.intent.kind = intent
        guard case .planned(let planned)? = call(.plan(request)) else { return nil }
        return planned.result
    }

    static func chord(_ form: Ikari_Settings_V1_ChordRequest.OneOf_Form) -> Ikari_Settings_V1_ChordInfo? {
        var request = Ikari_Settings_V1_ChordRequest()
        request.form = form
        guard case .chord(let info)? = call(.chord(request)) else { return nil }
        return info
    }

    static func text(_ body: Ikari_Settings_V1_AppRequest.OneOf_Body) -> String? {
        guard case .text(let text)? = call(body) else { return nil }
        return text
    }
}
