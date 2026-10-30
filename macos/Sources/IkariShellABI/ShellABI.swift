// 殻の 1 本の口（ADR 0041 §4.4。`ikari_shell_*`）を Swift から呼ぶ薄い層（契約 M1-b の stub）。
// イベントを protobuf にして `ikari_shell_dispatch` を呼び、動作の列を読むことと、ホストの口の表を
// Swift のクロージャから作ることだけをする。動作の実行と、IMK・XPC の実装は L-E が書く。
// 約束（所有権、スレッド、誤り）は shell-ffi/include/ikari_shell.h のコメントを正とする。
import Foundation
import IkariProto

#if IKARI_HAS_SHELL_FFI
import IkariShellFFI
import SwiftProtobuf

/// `IkariShellStatus`（ヘッダ）
public enum ShellABIStatus: Equatable, Sendable {
    case ok, null, invalidHandle, decode, busy, panic, unimplemented
    /// ヘッダにない値（版のずれ）
    case unknown(Int32)

    init(_ raw: Int32) {
        switch raw {
        case Int32(IKARI_SHELL_OK.rawValue): self = .ok
        case Int32(IKARI_SHELL_ERR_NULL.rawValue): self = .null
        case Int32(IKARI_SHELL_ERR_INVALID_HANDLE.rawValue): self = .invalidHandle
        case Int32(IKARI_SHELL_ERR_DECODE.rawValue): self = .decode
        case Int32(IKARI_SHELL_ERR_BUSY.rawValue): self = .busy
        case Int32(IKARI_SHELL_ERR_PANIC.rawValue): self = .panic
        case Int32(IKARI_SHELL_ERR_UNIMPLEMENTED.rawValue): self = .unimplemented
        default: self = .unknown(raw)
        }
    }
}

/// `IkariShellTransportResult`（ヘッダ）
public enum ShellTransportResult: Sendable {
    case ok(Data)
    case unavailable
    case timeout
}

/// `IkariShellDownKind`（ヘッダ）
public enum ShellDownKind: Sendable {
    case interrupted, invalidated

    var raw: UInt8 {
        switch self {
        case .interrupted: UInt8(IKARI_SHELL_DOWN_INTERRUPTED.rawValue)
        case .invalidated: UInt8(IKARI_SHELL_DOWN_INVALIDATED.rawValue)
        }
    }
}

/// ホストの口（`IkariShellHost`）。
/// - `surrounding` / `selection` は `dispatch` を呼んだスレッドで、その中からだけ呼ばれる。
/// - `engine*` はどのスレッドからでも並行に呼ばれる（`@Sendable`）。
public struct ShellHost {
    public var surrounding: () -> Ikari_Shell_V1_SurroundingText?
    public var selection: () -> Ikari_Shell_V1_SelectedText?
    /// 新しい接続の番号（0 以外。接続ごとに替える）。つながらなければ 0
    public var engineConnect: @Sendable () -> UInt64
    public var engineRequest: @Sendable (_ handle: UInt64, _ bytes: Data, _ timeoutMs: UInt32) -> ShellTransportResult
    public var enginePost: @Sendable (_ handle: UInt64, _ bytes: Data) -> Void
    public var engineClose: @Sendable (_ handle: UInt64) -> Void
    /// 分からなければ 0 以下
    public var enginePid: @Sendable (_ handle: UInt64) -> Int32

    public init(
        surrounding: @escaping () -> Ikari_Shell_V1_SurroundingText?,
        selection: @escaping () -> Ikari_Shell_V1_SelectedText?,
        engineConnect: @escaping @Sendable () -> UInt64,
        engineRequest: @escaping @Sendable (UInt64, Data, UInt32) -> ShellTransportResult,
        enginePost: @escaping @Sendable (UInt64, Data) -> Void,
        engineClose: @escaping @Sendable (UInt64) -> Void,
        enginePid: @escaping @Sendable (UInt64) -> Int32
    ) {
        self.surrounding = surrounding
        self.selection = selection
        self.engineConnect = engineConnect
        self.engineRequest = engineRequest
        self.enginePost = enginePost
        self.engineClose = engineClose
        self.enginePid = enginePid
    }
}

/// `ctx` として Rust に渡す箱。Rust が `release` を呼ぶまで生かす（`Unmanaged.passRetained`）。
/// 文字列のコールバックは dispatch のスレッドからだけ呼ばれ、engine_* は `@Sendable` なので、`@unchecked` でよい。
private final class HostBox: @unchecked Sendable {
    let host: ShellHost
    init(_ host: ShellHost) { self.host = host }

    static func from(_ ctx: UnsafeMutableRawPointer?) -> HostBox? {
        ctx.map { Unmanaged<HostBox>.fromOpaque($0).takeUnretainedValue() }
    }
}

/// Swift のバイト列を Rust が確保した複製にする（コールバックの *out に入れる。解放は Rust）。
private func rustCopy(_ data: Data) -> IkariShellBuf {
    data.withUnsafeBytes { raw in
        ikari_shell_buf_copy(IkariStr(ptr: raw.bindMemory(to: UInt8.self).baseAddress, len: raw.count))
    }
}

/// Rust が返したバイト列を `Data` に写して解放する（1 回だけ）。
private func takeRustBuf(_ buf: IkariShellBuf) -> Data {
    defer { ikari_shell_buf_free(buf) }
    guard let ptr = buf.ptr, buf.len > 0 else { return Data() }
    return Data(bytes: ptr, count: buf.len)
}

private func borrowed(_ str: IkariStr) -> Data {
    guard let ptr = str.ptr, str.len > 0 else { return Data() }
    return Data(bytes: ptr, count: str.len)
}

private func withIkariStr<R>(_ data: Data, _ body: (IkariStr) -> R) -> R {
    data.withUnsafeBytes { raw in body(IkariStr(ptr: raw.bindMemory(to: UInt8.self).baseAddress, len: raw.count)) }
}

private func withIkariStr<R>(_ string: String, _ body: (IkariStr) -> R) -> R {
    withIkariStr(Data(string.utf8), body)
}

private func writeMessage(_ message: (some SwiftProtobuf.Message)?, to out: UnsafeMutablePointer<IkariShellBuf>?) -> Int32 {
    guard let message, let out, let data = try? message.serializedData() else { return 1 }
    out.pointee = rustCopy(data)
    return 0
}

/// `IkariShellHost` の表。関数ポインタは捕捉のないクロージャ（`@convention(c)`）で、`ctx` から箱を引く。
private func makeHostTable(_ host: ShellHost) -> IkariShellHost {
    let ctx = Unmanaged.passRetained(HostBox(host)).toOpaque()
    return IkariShellHost(
        ctx: ctx,
        surrounding: { ctx, out in writeMessage(HostBox.from(ctx)?.host.surrounding(), to: out) },
        selection: { ctx, out in writeMessage(HostBox.from(ctx)?.host.selection(), to: out) },
        engine_connect: { ctx in HostBox.from(ctx)?.host.engineConnect() ?? 0 },
        engine_request: { ctx, handle, bytes, timeoutMs, reply in
            guard let box = HostBox.from(ctx) else { return Int32(IKARI_SHELL_TRANSPORT_UNAVAILABLE.rawValue) }
            switch box.host.engineRequest(handle, borrowed(bytes), timeoutMs) {
            case .ok(let data):
                reply?.pointee = rustCopy(data)
                return Int32(IKARI_SHELL_TRANSPORT_OK.rawValue)
            case .unavailable: return Int32(IKARI_SHELL_TRANSPORT_UNAVAILABLE.rawValue)
            case .timeout: return Int32(IKARI_SHELL_TRANSPORT_TIMEOUT.rawValue)
            }
        },
        engine_post: { ctx, handle, bytes in HostBox.from(ctx)?.host.enginePost(handle, borrowed(bytes)) },
        engine_close: { ctx, handle in HostBox.from(ctx)?.host.engineClose(handle) },
        engine_pid: { ctx, handle in HostBox.from(ctx)?.host.enginePid(handle) ?? -1 },
        release: { ctx in
            guard let ctx else { return }
            Unmanaged<HostBox>.fromOpaque(ctx).release()
        }
    )
}

/// 落ちた通知の口。**どのスレッドからでも呼べる**（解放ずみの shell には何もしない）。
public struct ShellDownNotifier: Sendable {
    let bits: UInt

    public func engineDown(generation: UInt64, kind: ShellDownKind) {
        ikari_shell_engine_down(OpaquePointer(bitPattern: bits), generation, kind.raw)
    }
}

/// `IkariShell` の持ち主。`new`・`dispatch`・解放（deinit）は 1 つのスレッド（主スレッド）から使う。
public final class ShellABI {
    public struct CreateError: Error, Equatable {
        public let message: String
    }

    private let handle: OpaquePointer

    public init(shellVersion: String, settingsPath: String, resourcesDir: String, host: ShellHost) throws {
        var error = IkariShellBuf(ptr: nil, len: 0)
        let table = makeHostTable(host)
        let created = withIkariStr(shellVersion) { version in
            withIkariStr(settingsPath) { settings in
                withIkariStr(resourcesDir) { resources in
                    var config = IkariShellConfig(shell_version: version, settings_path: settings, resources_dir: resources)
                    return ikari_shell_new(&config, table, &error)
                }
            }
        }
        let message = String(decoding: takeRustBuf(error), as: UTF8.self)
        guard let created else { throw CreateError(message: message) }
        handle = created
    }

    deinit { ikari_shell_free(handle) }

    public var downNotifier: ShellDownNotifier { ShellDownNotifier(bits: UInt(bitPattern: handle)) }

    /// イベントを 1 つ渡し、動作の列を返す。誤りの回も列は有効（pass_through の 1 つ）。
    public func dispatch(_ event: Ikari_Shell_V1_ShellEvent) -> (status: ShellABIStatus, actions: [Ikari_Shell_V1_ShellAction]) {
        // serializedData は、必須の欄のない proto3 では失敗しない。失敗したら空のバイト列（Rust が decode の誤りにする）
        let bytes = (try? event.serializedData()) ?? Data()
        return dispatch(bytes: bytes)
    }

    /// バイト列のままの版（テストと計測）。
    public func dispatch(bytes: Data) -> (status: ShellABIStatus, actions: [Ikari_Shell_V1_ShellAction]) {
        var out = IkariShellBuf(ptr: nil, len: 0)
        let raw = withIkariStr(bytes) { ikari_shell_dispatch(handle, $0, &out) }
        let actions = (try? Ikari_Shell_V1_ShellActions(serializedBytes: takeRustBuf(out)))?.actions ?? []
        return (ShellABIStatus(raw), actions)
    }
}
#endif
