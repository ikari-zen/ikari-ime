// 設定アプリと殻のブリッジ（ADR 0005 §6.3）。契約。枠の実装とテストは契約コミットで完成させてあり、
// 以後はレーン S3 も U も変えない（変えるときは契約として直す）。
//
// 依存の向き: IkariBridge → IkariProto。殻（IkariIME）が待ち受け側、設定アプリ（IkariSettings）が接続する側。
import Foundation
import IkariProto

public typealias Frame = Ikari_Bridge_V1_Frame
public typealias BridgeErrorCode = Ikari_Bridge_V1_BridgeErrorCode
public typealias EngineEnvelope = Ikari_Engine_V1_Envelope

public enum BridgeProtocol {
    /// `BridgeHello.bridge_version`。互換を壊す変更のときに上げる
    public static let version: UInt32 = 1
    /// 殻が待つソケットの、Application Support の下の相対パス（docs/phase2-contract.md §5.1）
    public static let socketRelativePath = "run/bridge.sock"
}

public enum BridgeLimits {
    /// 長さの接頭辞の大きさ（バイト。ビッグエンディアンの UInt32）
    public static let lengthPrefixBytes = 4
    /// 1 つの枠（protobuf の本体）の上限。エンジンの `MAX_ADMIN_REQUEST_BYTES` と同じ（ADR 0005 §8.4）
    public static let maxFrameBytes = 64 * 1024 * 1024
}

extension Frame {
    /// `BridgeError` の枠。`key` は不正だった設定のキー名だけを入れる（ユーザーの値を入れない）
    public static func error(requestID: UInt64, code: BridgeErrorCode, key: String = "") -> Frame {
        var error = Ikari_Bridge_V1_BridgeError()
        error.code = code
        error.key = key
        var frame = Frame()
        frame.requestID = requestID
        frame.error = error
        return frame
    }
}
