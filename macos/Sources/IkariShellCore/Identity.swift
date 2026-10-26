// 自動生成: macos/scripts/gen-identity.sh（値の出どころは macos/scripts/lib.sh の IKARI_ID_PREFIX と TEAM_ID。ADR 0036 §3）。
// 手で書き換えない。他の人が自分の値で使うときは、環境変数 IKARI_ID_PREFIX と TEAM_ID を与えて build.sh を流す。
import Foundation

public enum Identity {
    /// Bundle ID の頭（逆ドメインの形）
    public static let idPrefix = "io.github.ikari-zen"
    /// Apple Development 証明書のチームID（`certificate leaf[subject.OU]`）
    public static let teamID = "L9Z8SK775X"

    /// 製品（殻）の ID。他の識別子はこの後ろに足す
    public static let productID = "\(idPrefix).inputmethod.ikari"
}
