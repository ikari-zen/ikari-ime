// Bundle ID、XPC のサービス名、署名要件（ADR 0004 §5.1、§15.1-1/2。docs/phase2-contract.md §1〜§2）。契約。
import Foundation
import IkariShellCore

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

    /// 殻がブリッジ（Unix ソケット）で受けた接続の相手（設定アプリ）が正しいか（ADR 0005 §6.3）
    public static func settingsApp(mode: SigningMode, teamID: String = IkariIdentifiers.teamID) -> String {
        let identifier = "identifier \"\(IkariIdentifiers.settings)\""
        switch mode {
        case .team: return "\(identifier) and \(teamClause(teamID))"
        case .adhoc: return identifier
        }
    }

    private static func teamClause(_ teamID: String) -> String {
        "anchor apple generic and certificate leaf[subject.OU] = \"\(teamID)\""
    }
}
