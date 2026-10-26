// Unix ソケットの相手のコード署名の確認（`LOCAL_PEERTOKEN`）。殻の BridgeServer.peerSatisfies と同じ手順。
import Foundation
import Security

public enum PeerSignature {
    private static let solLocal: Int32 = 0
    private static let localPeerToken: Int32 = 0x006

    /// `getsockopt(LOCAL_PEERTOKEN)` の audit token から、相手のコードが署名要件を満たすか。
    /// 取得や評価のどこで失敗しても `false`。
    public static func satisfies(requirement text: String, descriptor: Int32) -> Bool {
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
        return SecCodeCheckValidity(code, SecCSFlags(), requirement) == errSecSuccess
    }
}
