// ブリッジの輸送（ADR 0005 §6.3）。`BridgeTransport` を設定アプリ側も殻側も使い、画面のテストは
// メモリ上の実装に差し替える。本物は Unix ドメインソケットのファイル記述子（`FileDescriptorTransport`）。
import Foundation
import IkariProto

public enum BridgeTransportError: Error, Equatable, Sendable {
    /// 相手が閉じた、または自分で閉じた
    case closed
    /// 読み書きの失敗（errno）
    case io(Int32)
    /// 枠が壊れている、または大きすぎる。接続を閉じること
    case frame(FrameError)
}

/// 枠を送り、枠を受ける。`receive` は枠がそろうまで待つ（呼び出す側が専用のスレッドかキューで回す）。
/// `send` は複数のスレッドから呼んでよい。`receive` は 1 つのスレッドだけが呼ぶ。
public protocol BridgeTransport: Sendable {
    func send(_ frame: Frame) throws
    func receive() throws -> Frame
    /// 以後の `send` と、待っている `receive` を `BridgeTransportError.closed` で終わらせる
    func close()
}

/// ソケット（または同等の双方向のファイル記述子）の上の輸送。
public final class FileDescriptorTransport: BridgeTransport, @unchecked Sendable {
    private static let readChunkBytes = 64 * 1024

    private let descriptor: Int32
    /// `SO_SNDTIMEO` を付けたか（付けたときだけ、書き込みの EAGAIN を時間切れと読む）
    private let hasSendTimeout: Bool
    private let stateLock = NSLock()
    private let sendLock = NSLock()
    private let receiveLock = NSLock()
    private var closed = false
    private var decoder = FrameDecoder()
    private var pending: [Frame] = []

    /// プロセスで 1 度だけ SIGPIPE を無視にする。`SO_NOSIGPIPE` だけでは足りない:
    /// macOS は相手がすでに閉じたソケットへの `setsockopt` を EINVAL で断るので、つないですぐ切れた相手には付かず、
    /// 最初の書き込みで SIGPIPE が起きてプロセスが落ちる（負荷が高いときのテストの異常終了で分かった）。
    /// 書き込みの失敗は errno（EPIPE）で受け取る
    private static let ignoreSIGPIPE: Void = {
        _ = signal(SIGPIPE, SIG_IGN)
    }()

    /// `descriptor` は引き取る（`deinit` で閉じる）。ソケットなら `SO_NOSIGPIPE` を付け、SIGPIPE も無視にして、
    /// 切れた相手への書き込みでプロセスが落ちないようにする。
    ///
    /// `sendTimeout` を渡すと、書き込みが進まない（相手が読まず、送信バッファが空かない）時間の上限を
    /// `SO_SNDTIMEO` で付ける。超えたら接続を閉じ、`send` は `BridgeTransportError.io(ETIMEDOUT)` を投げる
    /// （枠の途中まで書いたかもしれないので、続けて使えない）。上限は「進まない時間」で、枠全体の時間ではない
    /// （大きな枠を読み続けている相手は切らない）。`nil` なら上限なし（相手が読むまで待つ）
    public init(descriptor: Int32, sendTimeout: Duration? = nil) {
        self.descriptor = descriptor
        Self.ignoreSIGPIPE
        var on: Int32 = 1
        _ = setsockopt(descriptor, SOL_SOCKET, SO_NOSIGPIPE, &on, socklen_t(MemoryLayout<Int32>.size))
        hasSendTimeout = sendTimeout != nil
        if let sendTimeout {
            // 相手がすでに閉じていると EINVAL で断られる。そのときは最初の書き込みが EPIPE で失敗するので、無視してよい
            var interval = Self.makeTimeval(sendTimeout)
            _ = setsockopt(descriptor, SOL_SOCKET, SO_SNDTIMEO, &interval, socklen_t(MemoryLayout<timeval>.size))
        }
    }

    /// `Duration` を `timeval` に。0 は「上限なし」の意味になるので、最小でも 1 マイクロ秒にする
    private static func makeTimeval(_ duration: Duration) -> timeval {
        let (seconds, attoseconds) = duration.components
        let attosecondsPerMicrosecond: Int64 = 1_000_000_000_000
        var value = Darwin.timeval(
            tv_sec: __darwin_time_t(max(seconds, 0)),
            tv_usec: __darwin_suseconds_t(max(attoseconds / attosecondsPerMicrosecond, 0)))
        if value.tv_sec == 0 && value.tv_usec == 0 { value.tv_usec = 1 }
        return value
    }

    deinit {
        Darwin.close(descriptor)
    }

    /// 相手が切れたことを表す errno
    private static func isDisconnect(_ code: Int32) -> Bool {
        code == EPIPE || code == ECONNRESET
    }

    private var isClosed: Bool {
        stateLock.lock()
        defer { stateLock.unlock() }
        return closed
    }

    public func send(_ frame: Frame) throws {
        let data: Data
        do {
            data = try FrameCodec.encode(frame)
        } catch let error as FrameError {
            throw BridgeTransportError.frame(error)
        }
        sendLock.lock()
        defer { sendLock.unlock() }
        guard !isClosed else { throw BridgeTransportError.closed }
        try data.withUnsafeBytes { (raw: UnsafeRawBufferPointer) in
            var offset = 0
            while offset < raw.count {
                let written = Darwin.write(descriptor, raw.baseAddress! + offset, raw.count - offset)
                if written < 0 {
                    let code = errno
                    if code == EINTR { continue }
                    if hasSendTimeout && (code == EAGAIN || code == EWOULDBLOCK) {
                        // 相手が読まない。枠の途中で止まっているかもしれないので、接続ごと閉じる（相手はつなぎ直す）
                        close()
                        throw BridgeTransportError.io(ETIMEDOUT)
                    }
                    throw Self.isDisconnect(code) ? BridgeTransportError.closed : BridgeTransportError.io(code)
                }
                offset += written
            }
        }
    }

    public func receive() throws -> Frame {
        receiveLock.lock()
        defer { receiveLock.unlock() }
        var chunk = [UInt8](repeating: 0, count: Self.readChunkBytes)
        while pending.isEmpty {
            guard !isClosed else { throw BridgeTransportError.closed }
            let count = chunk.withUnsafeMutableBytes { Darwin.read(descriptor, $0.baseAddress, $0.count) }
            if count < 0 {
                if errno == EINTR { continue }
                throw isClosed || Self.isDisconnect(errno) ? BridgeTransportError.closed : BridgeTransportError.io(errno)
            }
            if count == 0 { throw BridgeTransportError.closed }
            do {
                pending.append(contentsOf: try decoder.feed(Data(chunk[0..<count])))
            } catch let error as FrameError {
                throw BridgeTransportError.frame(error)
            }
        }
        return pending.removeFirst()
    }

    public func close() {
        stateLock.lock()
        let first = !closed
        closed = true
        stateLock.unlock()
        // 待っている read を起こす。記述子そのものは deinit まで閉じない（別のスレッドが使っている間に再利用されないように）
        if first { _ = Darwin.shutdown(descriptor, SHUT_RDWR) }
    }

    /// 双方向につながったソケットの組（テスト、同じプロセスの中の接続）
    public static func socketPair() throws -> (FileDescriptorTransport, FileDescriptorTransport) {
        var fds: [Int32] = [0, 0]
        guard socketpair(AF_UNIX, SOCK_STREAM, 0, &fds) == 0 else { throw BridgeTransportError.io(errno) }
        return (FileDescriptorTransport(descriptor: fds[0]), FileDescriptorTransport(descriptor: fds[1]))
    }
}
