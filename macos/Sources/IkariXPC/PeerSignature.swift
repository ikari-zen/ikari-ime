// Unix ソケットの相手のコード署名の確認（`LOCAL_PEERTOKEN`）。殻の BridgeServer.peerSatisfies と同じ手順。
import Foundation
import Security

public enum PeerSignature {
    private static let solLocal: Int32 = 0
    private static let localPeerToken: Int32 = 0x006

    /// `getsockopt(LOCAL_PEERTOKEN)` の audit token から、相手のコードが署名要件を満たすか。
    /// 取得や評価のどこで失敗しても `false`。
    /// `expectedPath` があれば、相手のコードのパスもそこであること（`SecCodeCopyPath`）。
    public static func satisfies(requirement text: String, expectedPath: URL? = nil, descriptor: Int32) -> Bool {
        var token = audit_token_t()
        var length = socklen_t(MemoryLayout<audit_token_t>.size)
        guard getsockopt(descriptor, solLocal, localPeerToken, &token, &length) == 0 else { return false }
        let tokenData = withUnsafeBytes(of: &token) { Data($0) }

        var code: SecCode?
        let attributes = [kSecGuestAttributeAudit: tokenData] as CFDictionary
        guard SecCodeCopyGuestWithAttributes(nil, attributes, SecCSFlags(), &code) == errSecSuccess, let code else {
            return false
        }
        var requirement: SecRequirement?
        guard SecRequirementCreateWithString(text as CFString, SecCSFlags(), &requirement) == errSecSuccess,
            let requirement
        else { return false }
        guard SecCodeCheckValidity(code, SecCSFlags(), requirement) == errSecSuccess else { return false }
        guard let expectedPath else { return true }
        return codePath(code).map { BundlePinning.pathMatches($0, expected: expectedPath) } ?? false
    }

    /// 動いているコードのバンドルのパス
    static func codePath(_ code: SecCode) -> URL? {
        var staticCode: SecStaticCode?
        guard SecCodeCopyStaticCode(code, SecCSFlags(), &staticCode) == errSecSuccess, let staticCode else {
            return nil
        }
        return codePath(of: staticCode)
    }

    /// 静的コードのパス（`SecCodeCopyPath`。末尾に `/` が付くことがある）
    public static func codePath(of staticCode: SecStaticCode) -> URL? {
        var path: CFURL?
        guard SecCodeCopyPath(staticCode, SecCSFlags(), &path) == errSecSuccess else { return nil }
        return path as URL?
    }
}
