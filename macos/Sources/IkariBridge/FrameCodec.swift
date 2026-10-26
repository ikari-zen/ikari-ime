// 枠の読み書き（ADR 0005 §6.3）。枠 = 4 バイト（ビッグエンディアン）の長さ + `Frame` の protobuf。
import Foundation
import IkariProto
import SwiftProtobuf

public enum FrameError: Error, Equatable, Sendable {
    /// 長さが `BridgeLimits.maxFrameBytes` を超える（壊れた長さを含む）。読み始める前に断る
    case tooLarge(length: UInt64)
    /// 長さ 0 の枠。正しい `Frame` は必ず 1 つの本体を持つので、中身のない枠は壊れているとみなす
    case empty
    /// protobuf として読めない
    case malformed
}

public enum FrameCodec {
    /// 長さの接頭辞をつけた枠のバイト列
    public static func encode(_ frame: Frame) throws -> Data {
        let body: Data
        do {
            body = try frame.serializedData()
        } catch {
            throw FrameError.malformed
        }
        guard !body.isEmpty else { throw FrameError.empty }
        guard body.count <= BridgeLimits.maxFrameBytes else { throw FrameError.tooLarge(length: UInt64(body.count)) }
        var data = Data(capacity: BridgeLimits.lengthPrefixBytes + body.count)
        let length = UInt32(body.count)
        data.append(contentsOf: [UInt8(length >> 24 & 0xFF), UInt8(length >> 16 & 0xFF), UInt8(length >> 8 & 0xFF), UInt8(length & 0xFF)])
        data.append(body)
        return data
    }
}

/// バイト列を少しずつ受け取って枠に切り出す（ソケットの読みは枠の境目と合わない）。
/// 一度でも失敗したら、枠の位置が分からなくなるので、以後は同じ失敗を返し続ける（接続を閉じること）。
public struct FrameDecoder {
    private var buffer = Data()
    private var failure: FrameError?

    public init() {}

    /// 受け取ったバイト列を足して、そろった枠をすべて返す。足りない分は次の `feed` まで持ち越す
    public mutating func feed(_ chunk: Data) throws -> [Frame] {
        if let failure { throw failure }
        buffer.append(chunk)
        var frames: [Frame] = []
        do {
            while let frame = try nextFrame() { frames.append(frame) }
        } catch let error as FrameError {
            failure = error
            buffer = Data()
            throw error
        }
        return frames
    }

    /// まだ枠にならずに持っているバイト数
    public var bufferedByteCount: Int { buffer.count }

    private mutating func nextFrame() throws -> Frame? {
        let prefix = BridgeLimits.lengthPrefixBytes
        guard buffer.count >= prefix else { return nil }
        let start = buffer.startIndex
        var length: UInt64 = 0
        for offset in 0..<prefix { length = length << 8 | UInt64(buffer[start + offset]) }
        guard length <= UInt64(BridgeLimits.maxFrameBytes) else { throw FrameError.tooLarge(length: length) }
        guard length > 0 else { throw FrameError.empty }
        let total = prefix + Int(length)
        guard buffer.count >= total else { return nil }
        let body = buffer.subdata(in: (start + prefix)..<(start + total))
        buffer = Data(buffer.suffix(from: start + total))
        do {
            return try Frame(serializedBytes: body)
        } catch {
            throw FrameError.malformed
        }
    }
}
