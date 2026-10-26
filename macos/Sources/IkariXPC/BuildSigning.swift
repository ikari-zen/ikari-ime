// ビルドで選んだ署名の方式の読み取り（ADR 0004 §5.1。build.sh の `--signing` が Info.plist に書く）。
// Info.plist は署名の対象なので、書き換えれば署名が壊れる。
import Foundation

public enum BuildSigning {
    /// Info.plist のキー（build.sh が `team` または `adhoc` を書く）
    public static let infoPlistKey = "IkariSigningMode"

    /// 主バンドル（殻、smoke、XPC サービスのどれでも自分のバンドル）の Info.plist から読む。
    /// 読めない、または不正なときは `team`（厳しい方）にする。
    public static func current(bundle: Bundle = .main) -> SigningMode {
        mode(from: bundle.object(forInfoDictionaryKey: infoPlistKey) as? String)
    }

    public static func mode(from value: String?) -> SigningMode {
        value.flatMap(SigningMode.init(rawValue:)) ?? .team
    }
}
