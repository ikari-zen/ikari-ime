// ad-hoc 署名のとき、ブリッジの相手を「同じバンドルの中の本物」に固定する（#269）。
// ad-hoc では識別子の一致だけでは相手を確かめられないので、同じビルドのバンドルから cdhash を実行時に読み、
// 相手のコードの cdhash と、相手のパスが自分のバンドルの想定の場所かも確かめる。値はコードに埋めない。
// 読めない・構造が想定と違うときは nil（呼び出し側は接続を断る）。
import Foundation
import Security

public enum BundlePinning {
    /// 外側のバンドル（ikari.app）の中の設定アプリの相対パス
    public static let settingsRelativePath = "Contents/Helpers/IkariSettings.app"

    /// 殻（ikari.app）から見た、同梱の設定アプリのバンドルの場所
    public static func settingsURL(shellBundle: URL) -> URL {
        shellBundle.appendingPathComponent(settingsRelativePath, isDirectory: true)
    }

    /// 設定アプリ（`…/ikari.app/Contents/Helpers/IkariSettings.app`）から見た、外側の殻のバンドルの場所。
    /// 構造が想定と違えば nil
    public static func shellURL(settingsBundle: URL) -> URL? {
        let settings = settingsBundle.standardizedFileURL
        let helpers = settings.deletingLastPathComponent()
        let contents = helpers.deletingLastPathComponent()
        let shell = contents.deletingLastPathComponent()
        guard helpers.lastPathComponent == "Helpers", contents.lastPathComponent == "Contents",
            shell.pathExtension == "app", settings.pathExtension == "app"
        else { return nil }
        return shell
    }

    /// バンドルの静的コードの cdhash（`kSecCodeInfoUnique`）。読めなければ nil
    public static func cdhash(ofBundleAt url: URL) -> Data? {
        var staticCode: SecStaticCode?
        guard SecStaticCodeCreateWithPath(url as CFURL, SecCSFlags(), &staticCode) == errSecSuccess,
            let staticCode
        else { return nil }
        var information: CFDictionary?
        guard
            SecCodeCopySigningInformation(
                staticCode, SecCSFlags(rawValue: kSecCSSigningInformation), &information) == errSecSuccess,
            let dictionary = information as? [String: Any],
            let unique = dictionary[kSecCodeInfoUnique as String] as? Data, !unique.isEmpty
        else { return nil }
        return unique
    }

    /// 相手のパスが想定の場所か（シンボリックリンクを解いて比べる）
    public static func pathMatches(_ actual: URL, expected: URL) -> Bool {
        actual.resolvingSymlinksInPath().standardizedFileURL.path
            == expected.resolvingSymlinksInPath().standardizedFileURL.path
    }
}

/// ブリッジの相手を確かめるときに満たすべきこと（要件の文字列と、ad-hoc のときの相手のパス）
public struct PeerExpectation: Sendable {
    public let requirement: String
    /// 相手のコードのパスが満たすべき場所。nil なら見ない（team は要件がチームIDで縛る）
    public let expectedPath: URL?

    public init(requirement: String, expectedPath: URL?) {
        self.requirement = requirement
        self.expectedPath = expectedPath
    }

    /// 殻が、受けた接続の相手（設定アプリ）に求めるもの。`shellBundle` は自分（ikari.app）。作れなければ nil
    public static func settingsApp(
        mode: SigningMode, shellBundle: URL, teamID: String = IkariIdentifiers.teamID
    ) -> PeerExpectation? {
        switch mode {
        case .team:
            guard let text = SigningRequirements.settingsApp(mode: mode, teamID: teamID) else { return nil }
            return PeerExpectation(requirement: text, expectedPath: nil)
        case .adhoc:
            let url = BundlePinning.settingsURL(shellBundle: shellBundle)
            guard let hash = BundlePinning.cdhash(ofBundleAt: url),
                let text = SigningRequirements.settingsApp(mode: mode, cdhash: hash)
            else { return nil }
            return PeerExpectation(requirement: text, expectedPath: url)
        }
    }

    /// 設定アプリが、つないだ相手（殻）に求めるもの。`settingsBundle` は自分（IkariSettings.app）。作れなければ nil
    public static func bridgeShell(
        mode: SigningMode, settingsBundle: URL, teamID: String = IkariIdentifiers.teamID
    ) -> PeerExpectation? {
        switch mode {
        case .team:
            guard let text = SigningRequirements.bridgeShell(mode: mode, teamID: teamID) else { return nil }
            return PeerExpectation(requirement: text, expectedPath: nil)
        case .adhoc:
            guard let url = BundlePinning.shellURL(settingsBundle: settingsBundle),
                let hash = BundlePinning.cdhash(ofBundleAt: url),
                let text = SigningRequirements.bridgeShell(mode: mode, cdhash: hash)
            else { return nil }
            return PeerExpectation(requirement: text, expectedPath: url)
        }
    }
}
