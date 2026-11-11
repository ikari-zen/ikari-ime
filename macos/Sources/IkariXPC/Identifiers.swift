// Bundle ID、XPC のサービス名、署名要件（ADR 0004 §5.1、§15.1-1/2。docs/phase2-contract.md §1〜§2）。契約。
import Foundation

public enum IkariIdentifiers {
    /// 殻（ikari.app）の Bundle ID。`.inputmethod.` を含める
    public static let shell = Identity.productID
    /// 同梱の XPC サービス。サービス名もこれ
    public static let engine = "\(Identity.productID).engine"
    /// 補助の E2E 実行ファイル（署名の identifier）
    public static let smoke = "\(Identity.productID).smoke"
    /// IMK の接続名（Info.plist の `InputMethodConnectionName`）
    public static let inputMethodConnection = "\(Identity.productID)_Connection"
    /// 設定アプリ（ikari.app/Contents/Helpers/IkariSettings.app）の Bundle ID。署名の identifier もこれ（ADR 0005 §6.2）
    public static let settings = "\(Identity.productID).settings"
    /// 入力ソースの表示名
    public static let displayName = "ikari"
    /// Apple Development 証明書のチームID（`certificate leaf[subject.OU]`）
    public static let teamID = Identity.teamID
}

/// `build.sh --signing team|adhoc` で選ぶ署名の方式。
public enum SigningMode: String, Sendable, CaseIterable {
    /// Apple Development 証明書でチームIDに固定する（既定。ADR 0004 §15.1-1）
    case team
    /// ad-hoc（identifier の一致しか見られない）
    case adhoc
}

/// `NSXPCConnection.setCodeSigningRequirement` に渡す要件の文字列。
public enum SigningRequirements {

    /// 殻 → エンジン: 相手（エンジン）が正しいか
    public static func engine(mode: SigningMode, teamID: String = IkariIdentifiers.teamID) -> String {
        let identifier = "identifier \"\(IkariIdentifiers.engine)\""
        switch mode {
        case .team: return "\(identifier) and \(teamClause(teamID))"
        case .adhoc: return identifier
        }
    }

    /// エンジン → 殻: 相手（殻、または smoke）が正しいか
    public static func shell(mode: SigningMode, teamID: String = IkariIdentifiers.teamID) -> String {
        let identifiers =
            "(identifier \"\(IkariIdentifiers.shell)\" or identifier \"\(IkariIdentifiers.smoke)\")"
        switch mode {
        case .team: return "\(identifiers) and \(teamClause(teamID))"
        case .adhoc: return "identifier \"\(IkariIdentifiers.shell)\" or identifier \"\(IkariIdentifiers.smoke)\""
        }
    }

    /// 殻がブリッジ（Unix ソケット）で受けた接続の相手（設定アプリ）が正しいか（ADR 0005 §6.3）。
    /// ad-hoc では identifier の一致だけでは相手を確かめられないので、`cdhash`（同じバンドルから読んだ値）も要る。
    /// ad-hoc で `cdhash` がなければ要件を作れない（nil。呼び出し側は接続を断る）。
    public static func settingsApp(
        mode: SigningMode, teamID: String = IkariIdentifiers.teamID, cdhash: Data? = nil
    ) -> String? {
        pinned(identifier: IkariIdentifiers.settings, mode: mode, teamID: teamID, cdhash: cdhash)
    }

    /// 設定アプリがブリッジ（Unix ソケット）でつないだ相手（殻）が正しいか（殻側の `settingsApp` の対）。`cdhash` の扱いも同じ
    public static func bridgeShell(
        mode: SigningMode, teamID: String = IkariIdentifiers.teamID, cdhash: Data? = nil
    ) -> String? {
        pinned(identifier: IkariIdentifiers.shell, mode: mode, teamID: teamID, cdhash: cdhash)
    }

    /// `cdhash H"<16 進>"`（`kSecCodeInfoUnique` の値から）
    static func cdhashClause(_ cdhash: Data) -> String {
        "cdhash H\"" + cdhash.map { String(format: "%02x", $0) }.joined() + "\""
    }

    private static func pinned(identifier: String, mode: SigningMode, teamID: String, cdhash: Data?) -> String? {
        let identifierClause = "identifier \"\(identifier)\""
        switch mode {
        case .team: return "\(identifierClause) and \(teamClause(teamID))"
        case .adhoc:
            guard let cdhash, !cdhash.isEmpty else { return nil }
            return "\(identifierClause) and \(cdhashClause(cdhash))"
        }
    }

    private static func teamClause(_ teamID: String) -> String {
        "anchor apple generic and certificate leaf[subject.OU] = \"\(teamID)\""
    }
}
